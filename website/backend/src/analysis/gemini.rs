use std::{env, time::Duration};

use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap},
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::{Instant, sleep};

use crate::{
    analysis::request::{AnalysisRequest, AnalysisSettings, AnalysisTelemetry},
    db,
};

const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
const UPLOAD_URL: &str = "https://generativelanguage.googleapis.com/upload/v1beta/files";
const API_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta";
const GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const DEFAULT_FILE_PROCESSING_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(300);
const DEFAULT_UPLOAD_CHUNK_GRANULARITY_BYTES: usize = 256 * 1024;
const DEFAULT_UPLOAD_CHUNK_SIZE_BYTES: usize = 8 * 1024 * 1024;
const FILE_PROCESSING_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_UPLOAD_CHUNK_ATTEMPTS: usize = 4;
const MAX_UPLOAD_SESSION_ATTEMPTS: usize = 3;

mod prompts {
    /// Gemini currently receives the raw user prompt after the uploaded videos.
    ///
    /// Keep this function as the single edit point for Gemini prompt shaping so
    /// future provider-specific instructions do not get scattered through the
    /// request builder.
    pub fn user_prompt(prompt: &str) -> String {
        prompt.to_owned()
    }
}

pub async fn analyze_videos(
    videos: &[db::video::VideoAsset],
    prompt: &str,
) -> Result<String, GeminiError> {
    let client = GeminiClient::from_env()?;

    client
        .analyze(AnalysisRequest {
            videos: videos.to_vec(),
            prompt: prompt.to_owned(),
            settings: AnalysisSettings::default(),
            telemetry: Default::default(),
        })
        .await
}

pub struct GeminiClient {
    http: reqwest::Client,
    config: GeminiConfig,
}

struct GeminiConfig {
    api_key: String,
    model: String,
    file_processing_timeout: Duration,
    upload_chunk_size: usize,
}

struct VideoInput {
    name: String,
    bytes: Vec<u8>,
}

struct UploadSession {
    url: String,
    chunk_granularity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct UploadChunk {
    offset: usize,
    end: usize,
    command: UploadCommand,
}

impl UploadChunk {
    fn len(self) -> usize {
        self.end - self.offset
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadCommand {
    Upload,
    UploadAndFinalize,
}

impl UploadCommand {
    fn as_header(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::UploadAndFinalize => "upload, finalize",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UploadRetryDecision {
    RetryFromOffset(usize),
    RestartSession,
}

#[derive(Debug)]
enum UploadChunksError {
    Gemini(GeminiError),
    FinalizedWithoutResponse(GeminiError),
}

impl From<GeminiError> for UploadChunksError {
    fn from(error: GeminiError) -> Self {
        Self::Gemini(error)
    }
}

#[derive(Debug, PartialEq)]
struct UploadedFile {
    uri: String,
    mime_type: String,
}

#[derive(Debug, Error)]
pub enum GeminiError {
    #[error("GEMINI_API_KEY is not configured")]
    MissingApiKey,
    #[error("Gemini upload response did not include an upload URL")]
    MissingUploadUrl,
    #[error("Gemini upload query response did not include a received-size offset")]
    MissingUploadOffset,
    #[error("Gemini file {name} failed while processing")]
    FileProcessingFailed { name: String },
    #[error("Gemini file {name} did not become active within {timeout_secs} seconds")]
    FileProcessingTimedOut { name: String, timeout_secs: u64 },
    #[error("Gemini API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("Gemini did not return any text")]
    EmptyResponse,
    #[error("Gemini upload chunk granularity header was invalid: {value}")]
    InvalidUploadChunkGranularity { value: String },
    #[error("Gemini upload received-size offset header was invalid: {value}")]
    InvalidUploadOffset { value: String },
    #[error(
        "Gemini upload finalized for {name}, but the final response was lost after {attempts} upload session attempts: {source_message}"
    )]
    UploadFinalizationUnknown {
        name: String,
        attempts: usize,
        source_message: String,
    },
    #[error(
        "Gemini upload request failed for {name} at offset {offset} ({bytes} bytes, timeout={timeout}, connect={connect}, body={body}): {source}"
    )]
    UploadRequest {
        name: String,
        offset: usize,
        bytes: usize,
        timeout: bool,
        connect: bool,
        body: bool,
        #[source]
        source: reqwest::Error,
    },
    #[error(transparent)]
    Header(#[from] reqwest::header::ToStrError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl GeminiError {
    fn is_retriable_upload_failure(&self) -> bool {
        match self {
            Self::UploadRequest {
                timeout,
                connect,
                body,
                ..
            } => *timeout || *connect || *body,
            Self::Api { status, .. } => {
                status.is_server_error()
                    || matches!(
                        *status,
                        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
                    )
            }
            _ => false,
        }
    }
}

impl GeminiClient {
    pub fn from_env() -> Result<Self, GeminiError> {
        let config = GeminiConfig::from_env()?;

        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(DEFAULT_HTTP_TIMEOUT)
                .build()?,
            config,
        })
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, GeminiError> {
        let telemetry = request.telemetry.clone();
        let videos = request
            .videos
            .iter()
            .map(|asset| VideoInput {
                name: asset.video.name.clone(),
                bytes: asset.bytes.clone(),
            })
            .collect::<Vec<_>>();

        self.analyze_inputs(&telemetry, &videos, &request.prompt)
            .await
    }

    async fn analyze_inputs(
        &self,
        telemetry: &AnalysisTelemetry,
        videos: &[VideoInput],
        prompt: &str,
    ) -> Result<String, GeminiError> {
        let mut files = Vec::with_capacity(videos.len());

        for video in videos {
            files.push(self.upload_video(telemetry, video).await?);
        }

        self.generate_content(telemetry, &files, prompt).await
    }

    async fn upload_video(
        &self,
        telemetry: &AnalysisTelemetry,
        video: &VideoInput,
    ) -> Result<UploadedFile, GeminiError> {
        let mime_type = video_mime_type(&video.name);

        for attempt in 1..=MAX_UPLOAD_SESSION_ATTEMPTS {
            if attempt > 1 {
                telemetry.log(
                    "warn",
                    "gemini",
                    "upload_session_restarted",
                    [
                        ("video_name", json!(video.name)),
                        ("attempt", json!(attempt)),
                        ("attempts", json!(MAX_UPLOAD_SESSION_ATTEMPTS)),
                    ],
                );
            }

            let upload_session = self.start_upload(video, mime_type).await?;
            let upload = match self
                .upload_video_chunks(telemetry, video, &upload_session, mime_type)
                .await
            {
                Ok(upload) => upload,
                Err(UploadChunksError::Gemini(error)) => return Err(error),
                Err(UploadChunksError::FinalizedWithoutResponse(error))
                    if attempt < MAX_UPLOAD_SESSION_ATTEMPTS =>
                {
                    telemetry.log(
                        "warn",
                        "gemini",
                        "upload_final_response_lost",
                        [
                            ("video_name", json!(video.name)),
                            ("attempt", json!(attempt)),
                            ("attempts", json!(MAX_UPLOAD_SESSION_ATTEMPTS)),
                            ("error", json!(error.to_string())),
                        ],
                    );
                    continue;
                }
                Err(UploadChunksError::FinalizedWithoutResponse(error)) => {
                    return Err(GeminiError::UploadFinalizationUnknown {
                        name: video.name.clone(),
                        attempts: MAX_UPLOAD_SESSION_ATTEMPTS,
                        source_message: error.to_string(),
                    });
                }
            };

            let file = self.wait_for_file_active(upload.file).await?;

            return Ok(UploadedFile {
                uri: file.uri,
                mime_type: mime_type.to_owned(),
            });
        }

        unreachable!("upload session retry loop should return")
    }

    async fn start_upload(
        &self,
        video: &VideoInput,
        mime_type: &'static str,
    ) -> Result<UploadSession, GeminiError> {
        let response = self
            .http
            .post(UPLOAD_URL)
            .header("x-goog-api-key", &self.config.api_key)
            .header("X-Goog-Upload-Protocol", "resumable")
            .header("X-Goog-Upload-Command", "start")
            .header("X-Goog-Upload-Header-Content-Length", video.bytes.len())
            .header("X-Goog-Upload-Header-Content-Type", mime_type)
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "file": {
                    "display_name": video.name,
                }
            }))
            .send()
            .await?;

        let headers = success_headers(response).await?;
        upload_session_from_headers(&headers)
    }

    async fn upload_video_chunks(
        &self,
        telemetry: &AnalysisTelemetry,
        video: &VideoInput,
        session: &UploadSession,
        mime_type: &'static str,
    ) -> Result<UploadResponse, UploadChunksError> {
        let total_bytes = video.bytes.len();
        let total_chunks = upload_chunks(
            total_bytes,
            session.chunk_granularity,
            self.config.upload_chunk_size,
        )
        .len();
        telemetry.log(
            "info",
            "gemini",
            "upload_started",
            [
                ("video_name", json!(video.name)),
                ("bytes", json!(total_bytes)),
                ("mime", json!(mime_type)),
                ("chunks", json!(total_chunks)),
                ("chunk_size", json!(self.config.upload_chunk_size)),
                ("granularity", json!(session.chunk_granularity)),
            ],
        );

        let mut offset = 0;
        let mut chunk_index = 0;
        let mut attempts_at_offset = 0;

        while offset < total_bytes {
            let chunk = next_upload_chunk(
                total_bytes,
                offset,
                session.chunk_granularity,
                self.config.upload_chunk_size,
            );
            attempts_at_offset += 1;
            telemetry.log(
                "info",
                "gemini",
                "upload_chunk_send",
                [
                    ("video_name", json!(video.name)),
                    ("chunk", json!(chunk_index + 1)),
                    ("chunks", json!(total_chunks)),
                    ("offset", json!(chunk.offset)),
                    ("bytes", json!(chunk.len())),
                    ("command", json!(chunk.command.as_header())),
                    ("attempt", json!(attempts_at_offset)),
                    ("attempts", json!(MAX_UPLOAD_CHUNK_ATTEMPTS)),
                ],
            );

            match self.send_upload_chunk(video, session, chunk).await {
                Ok(Some(upload)) => return Ok(upload),
                Ok(None) => {
                    offset = chunk.end;
                    chunk_index += 1;
                    attempts_at_offset = 0;
                }
                Err(error) if error.is_retriable_upload_failure() => {
                    if attempts_at_offset >= MAX_UPLOAD_CHUNK_ATTEMPTS {
                        return Err(error.into());
                    }

                    telemetry.log(
                        "warn",
                        "gemini",
                        "upload_chunk_retry",
                        [
                            ("video_name", json!(video.name)),
                            ("offset", json!(chunk.offset)),
                            ("error", json!(error.to_string())),
                        ],
                    );
                    let received_offset = self
                        .query_upload_offset_with_retries(telemetry, video, session, chunk.offset)
                        .await?;
                    telemetry.log(
                        "info",
                        "gemini",
                        "upload_offset_queried",
                        [
                            ("video_name", json!(video.name)),
                            ("requested_offset", json!(chunk.offset)),
                            ("received_offset", json!(received_offset)),
                        ],
                    );

                    match upload_retry_decision(total_bytes, chunk, received_offset)? {
                        UploadRetryDecision::RestartSession => {
                            return Err(UploadChunksError::FinalizedWithoutResponse(error));
                        }
                        UploadRetryDecision::RetryFromOffset(next_offset)
                            if next_offset > offset =>
                        {
                            offset = next_offset;
                            chunk_index = upload_chunks(
                                offset,
                                session.chunk_granularity,
                                self.config.upload_chunk_size,
                            )
                            .len();
                            attempts_at_offset = 0;
                        }
                        UploadRetryDecision::RetryFromOffset(_) => {}
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }

        let chunk = UploadChunk {
            offset: total_bytes,
            end: total_bytes,
            command: UploadCommand::UploadAndFinalize,
        };
        match self.send_upload_chunk(video, session, chunk).await? {
            Some(upload) => Ok(upload),
            None => Err(GeminiError::EmptyResponse.into()),
        }
    }

    async fn send_upload_chunk(
        &self,
        video: &VideoInput,
        session: &UploadSession,
        chunk: UploadChunk,
    ) -> Result<Option<UploadResponse>, GeminiError> {
        let response = self
            .http
            .post(&session.url)
            .header("x-goog-api-key", &self.config.api_key)
            .header(CONTENT_LENGTH, chunk.len())
            .header("X-Goog-Upload-Offset", chunk.offset.to_string())
            .header("X-Goog-Upload-Command", chunk.command.as_header())
            .body(video.bytes[chunk.offset..chunk.end].to_vec())
            .send()
            .await
            .map_err(|source| GeminiError::UploadRequest {
                name: video.name.clone(),
                offset: chunk.offset,
                bytes: chunk.len(),
                timeout: source.is_timeout(),
                connect: source.is_connect(),
                body: source.is_body(),
                source,
            })?;

        if chunk.command == UploadCommand::UploadAndFinalize {
            return Ok(Some(success_json(response).await?));
        }

        success_upload_chunk(response).await?;
        Ok(None)
    }

    async fn query_upload_offset_with_retries(
        &self,
        telemetry: &AnalysisTelemetry,
        video: &VideoInput,
        session: &UploadSession,
        requested_offset: usize,
    ) -> Result<usize, GeminiError> {
        for attempt in 1..=MAX_UPLOAD_CHUNK_ATTEMPTS {
            match self
                .query_upload_offset(video, session, requested_offset)
                .await
            {
                Ok(offset) => return Ok(offset),
                Err(error)
                    if error.is_retriable_upload_failure()
                        && attempt < MAX_UPLOAD_CHUNK_ATTEMPTS =>
                {
                    telemetry.log(
                        "warn",
                        "gemini",
                        "upload_offset_query_retry",
                        [
                            ("video_name", json!(video.name)),
                            ("requested_offset", json!(requested_offset)),
                            ("attempt", json!(attempt)),
                            ("attempts", json!(MAX_UPLOAD_CHUNK_ATTEMPTS)),
                            ("error", json!(error.to_string())),
                        ],
                    );
                    sleep(Duration::from_secs(attempt as u64)).await;
                }
                Err(error) => return Err(error),
            }
        }

        unreachable!("upload query retry loop should return")
    }

    async fn query_upload_offset(
        &self,
        video: &VideoInput,
        session: &UploadSession,
        requested_offset: usize,
    ) -> Result<usize, GeminiError> {
        let response = self
            .http
            .post(&session.url)
            .header("x-goog-api-key", &self.config.api_key)
            .header(CONTENT_LENGTH, 0)
            .header("X-Goog-Upload-Command", "query")
            .send()
            .await
            .map_err(|source| GeminiError::UploadRequest {
                name: video.name.clone(),
                offset: requested_offset,
                bytes: 0,
                timeout: source.is_timeout(),
                connect: source.is_connect(),
                body: source.is_body(),
                source,
            })?;
        let headers = success_headers(response).await?;

        upload_offset_from_headers(&headers)
    }

    async fn wait_for_file_active(
        &self,
        mut file: UploadedFileResponse,
    ) -> Result<UploadedFileResponse, GeminiError> {
        let deadline = Instant::now() + self.config.file_processing_timeout;

        loop {
            if file.state.is_active() {
                return Ok(file);
            }

            if file.state.is_failed() {
                return Err(GeminiError::FileProcessingFailed { name: file.name });
            }

            if Instant::now() >= deadline {
                return Err(GeminiError::FileProcessingTimedOut {
                    name: file.name,
                    timeout_secs: self.config.file_processing_timeout.as_secs(),
                });
            }

            sleep(FILE_PROCESSING_POLL_INTERVAL).await;
            file = self.get_file(&file.name).await?;
        }
    }

    async fn get_file(&self, name: &str) -> Result<UploadedFileResponse, GeminiError> {
        let response = self
            .http
            .get(format!("{API_URL_PREFIX}/{name}"))
            .header("x-goog-api-key", &self.config.api_key)
            .send()
            .await?;
        let response: GetFileResponse = success_json(response).await?;

        Ok(response.into_file())
    }

    async fn generate_content(
        &self,
        telemetry: &AnalysisTelemetry,
        files: &[UploadedFile],
        prompt: &str,
    ) -> Result<String, GeminiError> {
        telemetry.log(
            "info",
            "gemini",
            "generate_content_request",
            [("files", json!(files.len()))],
        );
        let response = self
            .http
            .post(format!(
                "{}/{model}:generateContent",
                GENERATE_URL_PREFIX,
                model = self.config.model
            ))
            .header("x-goog-api-key", &self.config.api_key)
            .json(&generate_content_request(files, prompt))
            .send()
            .await?;
        let response: GenerateContentResponse = success_json(response).await?;

        let text = response.text().ok_or(GeminiError::EmptyResponse)?;
        telemetry.log(
            "info",
            "gemini",
            "generate_content_response",
            [("chars", json!(text.len()))],
        );
        Ok(text)
    }
}

impl GeminiConfig {
    fn from_env() -> Result<Self, GeminiError> {
        let api_key = env::var("GEMINI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(GeminiError::MissingApiKey)?;
        let model = env::var("GEMINI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let file_processing_timeout = env::var("GEMINI_FILE_PROCESSING_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_FILE_PROCESSING_TIMEOUT);
        let upload_chunk_size = env::var("GEMINI_UPLOAD_CHUNK_SIZE_BYTES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_UPLOAD_CHUNK_SIZE_BYTES);

        Ok(Self {
            api_key,
            model,
            file_processing_timeout,
            upload_chunk_size,
        })
    }
}

async fn success_headers(response: reqwest::Response) -> Result<HeaderMap, GeminiError> {
    let status = response.status();

    if status.is_success() {
        return Ok(response.headers().clone());
    }

    Err(GeminiError::Api {
        status,
        body: response.text().await.unwrap_or_default(),
    })
}

async fn success_json<T>(response: reqwest::Response) -> Result<T, GeminiError>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();

    if status.is_success() {
        return Ok(response.json().await?);
    }

    Err(GeminiError::Api {
        status,
        body: response.text().await.unwrap_or_default(),
    })
}

async fn success_upload_chunk(response: reqwest::Response) -> Result<(), GeminiError> {
    let status = response.status();

    if status.is_success() || status.as_u16() == 308 {
        return Ok(());
    }

    Err(GeminiError::Api {
        status,
        body: response.text().await.unwrap_or_default(),
    })
}

fn upload_session_from_headers(headers: &HeaderMap) -> Result<UploadSession, GeminiError> {
    let url = headers
        .get("x-goog-upload-url")
        .ok_or(GeminiError::MissingUploadUrl)?
        .to_str()?
        .to_owned();
    let chunk_granularity = match headers.get("x-goog-upload-chunk-granularity") {
        Some(value) => {
            let value = value.to_str()?;
            value
                .parse::<usize>()
                .map_err(|_| GeminiError::InvalidUploadChunkGranularity {
                    value: value.to_owned(),
                })?
        }
        None => DEFAULT_UPLOAD_CHUNK_GRANULARITY_BYTES,
    };

    Ok(UploadSession {
        url,
        chunk_granularity: chunk_granularity.max(1),
    })
}

fn upload_chunks(
    total_bytes: usize,
    chunk_granularity: usize,
    preferred_chunk_size: usize,
) -> Vec<UploadChunk> {
    if total_bytes == 0 {
        return vec![UploadChunk {
            offset: 0,
            end: 0,
            command: UploadCommand::UploadAndFinalize,
        }];
    }

    let chunk_granularity = chunk_granularity.max(1);
    let preferred_chunk_size = preferred_chunk_size.max(chunk_granularity);
    let chunk_size = (preferred_chunk_size / chunk_granularity)
        .saturating_mul(chunk_granularity)
        .max(chunk_granularity);
    let mut chunks = Vec::new();
    let mut offset = 0;

    while offset < total_bytes {
        let remaining = total_bytes - offset;
        let is_last = remaining <= chunk_size;
        let end = if is_last {
            total_bytes
        } else {
            offset + chunk_size
        };
        chunks.push(UploadChunk {
            offset,
            end,
            command: if is_last {
                UploadCommand::UploadAndFinalize
            } else {
                UploadCommand::Upload
            },
        });
        offset = end;
    }

    chunks
}

fn next_upload_chunk(
    total_bytes: usize,
    offset: usize,
    chunk_granularity: usize,
    preferred_chunk_size: usize,
) -> UploadChunk {
    let chunk_granularity = chunk_granularity.max(1);
    let preferred_chunk_size = preferred_chunk_size.max(chunk_granularity);
    let chunk_size = (preferred_chunk_size / chunk_granularity)
        .saturating_mul(chunk_granularity)
        .max(chunk_granularity);
    let remaining = total_bytes - offset;
    let is_last = remaining <= chunk_size;
    let end = if is_last {
        total_bytes
    } else {
        offset + chunk_size
    };

    UploadChunk {
        offset,
        end,
        command: if is_last {
            UploadCommand::UploadAndFinalize
        } else {
            UploadCommand::Upload
        },
    }
}

fn upload_retry_decision(
    total_bytes: usize,
    chunk: UploadChunk,
    received_offset: usize,
) -> Result<UploadRetryDecision, GeminiError> {
    if received_offset > total_bytes {
        return Err(GeminiError::InvalidUploadOffset {
            value: received_offset.to_string(),
        });
    }

    if chunk.command == UploadCommand::UploadAndFinalize && received_offset == total_bytes {
        return Ok(UploadRetryDecision::RestartSession);
    }

    Ok(UploadRetryDecision::RetryFromOffset(received_offset))
}

fn upload_offset_from_headers(headers: &HeaderMap) -> Result<usize, GeminiError> {
    let value = headers
        .get("x-goog-upload-size-received")
        .ok_or(GeminiError::MissingUploadOffset)?
        .to_str()?;

    value
        .parse::<usize>()
        .map_err(|_| GeminiError::InvalidUploadOffset {
            value: value.to_owned(),
        })
}

fn generate_content_request(files: &[UploadedFile], prompt: &str) -> Value {
    let mut parts = files
        .iter()
        .map(|file| {
            json!({
                "file_data": {
                    "mime_type": file.mime_type,
                    "file_uri": file.uri,
                }
            })
        })
        .collect::<Vec<_>>();
    parts.push(json!({ "text": prompts::user_prompt(prompt) }));

    json!({
        "contents": [{
            "role": "user",
            "parts": parts,
        }]
    })
}

fn video_mime_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some(extension) if extension.eq_ignore_ascii_case("mp4") => "video/mp4",
        Some(extension) if extension.eq_ignore_ascii_case("mpeg") => "video/mpeg",
        Some(extension) if extension.eq_ignore_ascii_case("mov") => "video/quicktime",
        Some(extension) if extension.eq_ignore_ascii_case("avi") => "video/avi",
        Some(extension) if extension.eq_ignore_ascii_case("flv") => "video/x-flv",
        Some(extension) if extension.eq_ignore_ascii_case("mpg") => "video/mpg",
        Some(extension) if extension.eq_ignore_ascii_case("webm") => "video/webm",
        Some(extension) if extension.eq_ignore_ascii_case("wmv") => "video/wmv",
        Some(extension) if extension.eq_ignore_ascii_case("3gp") => "video/3gpp",
        _ => "video/mp4",
    }
}

#[derive(Deserialize)]
struct UploadResponse {
    file: UploadedFileResponse,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GetFileResponse {
    Wrapped { file: UploadedFileResponse },
    Direct(UploadedFileResponse),
}

impl GetFileResponse {
    fn into_file(self) -> UploadedFileResponse {
        match self {
            Self::Wrapped { file } | Self::Direct(file) => file,
        }
    }
}

#[derive(Deserialize)]
struct UploadedFileResponse {
    name: String,
    uri: String,
    #[serde(default, deserialize_with = "deserialize_file_state")]
    state: FileState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileState {
    Unspecified,
    Processing,
    Active,
    Failed,
}

impl Default for FileState {
    fn default() -> Self {
        Self::Unspecified
    }
}

impl FileState {
    fn is_active(self) -> bool {
        matches!(self, Self::Active)
    }

    fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    fn is_waitable(self) -> bool {
        matches!(self, Self::Unspecified | Self::Processing)
    }
}

fn deserialize_file_state<'de, D>(deserializer: D) -> Result<FileState, D::Error>
where
    D: Deserializer<'de>,
{
    let state = Option::<String>::deserialize(deserializer)?;

    Ok(match state.as_deref() {
        Some("ACTIVE") => FileState::Active,
        Some("FAILED") => FileState::Failed,
        Some("PROCESSING") => FileState::Processing,
        _ => FileState::Unspecified,
    })
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

impl GenerateContentResponse {
    fn text(self) -> Option<String> {
        let text = self
            .candidates
            .into_iter()
            .flat_map(|candidate| candidate.content.parts)
            .filter_map(|part| part.text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Deserialize)]
struct ResponsePart {
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn video_mime_type_maps_supported_extensions() {
        assert_eq!(video_mime_type("clip.mp4"), "video/mp4");
        assert_eq!(video_mime_type("clip.mov"), "video/quicktime");
        assert_eq!(video_mime_type("clip.avi"), "video/avi");
        assert_eq!(video_mime_type("clip.webm"), "video/webm");
        assert_eq!(video_mime_type("clip.wmv"), "video/wmv");
        assert_eq!(video_mime_type("clip.3gp"), "video/3gpp");
        assert_eq!(video_mime_type("clip.unknown"), "video/mp4");
    }

    #[test]
    fn generate_content_request_puts_uploaded_videos_before_prompt() {
        let files = [
            UploadedFile {
                uri: "https://files.example/one".to_owned(),
                mime_type: "video/mp4".to_owned(),
            },
            UploadedFile {
                uri: "https://files.example/two".to_owned(),
                mime_type: "video/webm".to_owned(),
            },
        ];

        let request = generate_content_request(&files, "Find the key moments.");

        assert_eq!(
            request,
            json!({
                "contents": [{
                    "role": "user",
                    "parts": [
                        {
                            "file_data": {
                                "mime_type": "video/mp4",
                                "file_uri": "https://files.example/one"
                            }
                        },
                        {
                            "file_data": {
                                "mime_type": "video/webm",
                                "file_uri": "https://files.example/two"
                            }
                        },
                        {
                            "text": "Find the key moments."
                        }
                    ]
                }]
            })
        );
    }

    #[test]
    fn upload_response_keeps_file_name_uri_and_state() {
        let response: UploadResponse = serde_json::from_value(json!({
            "file": {
                "name": "files/46pyf29h2xti",
                "uri": "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti",
                "state": "PROCESSING"
            }
        }))
        .expect("upload response should deserialize");

        assert_eq!(response.file.name, "files/46pyf29h2xti");
        assert_eq!(
            response.file.uri,
            "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti"
        );
        assert_eq!(response.file.state, FileState::Processing);
    }

    #[test]
    fn get_file_response_accepts_direct_file_shape() {
        let response: GetFileResponse = serde_json::from_value(json!({
            "name": "files/46pyf29h2xti",
            "uri": "https://generativelanguage.googleapis.com/v1beta/files/46pyf29h2xti",
            "state": "ACTIVE"
        }))
        .expect("get file response should deserialize");
        let file = response.into_file();

        assert_eq!(file.name, "files/46pyf29h2xti");
        assert_eq!(file.state, FileState::Active);
    }

    #[test]
    fn upload_chunks_split_large_files_on_preferred_boundaries() {
        let chunks = upload_chunks(36, 4, 16);

        assert_eq!(
            chunks,
            vec![
                UploadChunk {
                    offset: 0,
                    end: 16,
                    command: UploadCommand::Upload,
                },
                UploadChunk {
                    offset: 16,
                    end: 32,
                    command: UploadCommand::Upload,
                },
                UploadChunk {
                    offset: 32,
                    end: 36,
                    command: UploadCommand::UploadAndFinalize,
                },
            ]
        );
    }

    #[test]
    fn upload_chunks_respect_google_chunk_granularity() {
        let chunks = upload_chunks(37, 6, 16);

        assert_eq!(
            chunks,
            vec![
                UploadChunk {
                    offset: 0,
                    end: 12,
                    command: UploadCommand::Upload,
                },
                UploadChunk {
                    offset: 12,
                    end: 24,
                    command: UploadCommand::Upload,
                },
                UploadChunk {
                    offset: 24,
                    end: 36,
                    command: UploadCommand::Upload,
                },
                UploadChunk {
                    offset: 36,
                    end: 37,
                    command: UploadCommand::UploadAndFinalize,
                },
            ]
        );
    }

    #[test]
    fn upload_session_reads_chunk_granularity_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-upload-url",
            "https://uploads.example/session".parse().unwrap(),
        );
        headers.insert("x-goog-upload-chunk-granularity", "262144".parse().unwrap());

        let session = upload_session_from_headers(&headers).expect("session should parse");

        assert_eq!(session.url, "https://uploads.example/session");
        assert_eq!(session.chunk_granularity, 262144);
    }

    #[test]
    fn upload_offset_reads_resumable_query_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-goog-upload-size-received", "83886080".parse().unwrap());

        let offset = upload_offset_from_headers(&headers).expect("offset should parse");

        assert_eq!(offset, 83886080);
    }

    #[test]
    fn upload_retry_restarts_session_when_final_chunk_was_consumed() {
        let total_bytes = 364_996_893;
        let final_chunk = UploadChunk {
            offset: 360_710_144,
            end: total_bytes,
            command: UploadCommand::UploadAndFinalize,
        };

        let decision = upload_retry_decision(total_bytes, final_chunk, total_bytes)
            .expect("retry decision should be valid");

        assert_eq!(decision, UploadRetryDecision::RestartSession);
    }

    #[tokio::test]
    async fn upload_query_timeout_uses_retriable_upload_context() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("connection should accept");
            sleep(Duration::from_secs(1)).await;
        });
        let client = GeminiClient {
            http: reqwest::Client::builder()
                .timeout(Duration::from_millis(25))
                .build()
                .expect("client should build"),
            config: GeminiConfig {
                api_key: "test-key".to_owned(),
                model: "test-model".to_owned(),
                file_processing_timeout: Duration::from_secs(1),
                upload_chunk_size: 8,
            },
        };
        let video = VideoInput {
            name: "large.mp4".to_owned(),
            bytes: Vec::new(),
        };
        let session = UploadSession {
            url: format!("http://{address}/upload"),
            chunk_granularity: 1,
        };

        let error = client
            .query_upload_offset(&video, &session, 123)
            .await
            .expect_err("query should time out");

        assert!(error.is_retriable_upload_failure());
        match error {
            GeminiError::UploadRequest {
                name,
                offset,
                bytes,
                timeout,
                ..
            } => {
                assert_eq!(name, "large.mp4");
                assert_eq!(offset, 123);
                assert_eq!(bytes, 0);
                assert!(timeout);
            }
            other => panic!("expected upload request error, got {other}"),
        }
    }

    #[test]
    fn file_state_detects_ready_and_failed_states() {
        assert!(FileState::Active.is_active());
        assert!(!FileState::Processing.is_active());
        assert!(FileState::Failed.is_failed());
        assert!(FileState::Unspecified.is_waitable());
        assert!(FileState::Processing.is_waitable());
        assert!(!FileState::Failed.is_waitable());
    }
}
