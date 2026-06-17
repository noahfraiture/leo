use std::time::Duration;

use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap},
};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::{Instant, sleep};

use crate::analysis::{
    prompts::gemini as prompts,
    request::{AnalysisRequest, AnalysisSettings, AnalysisTelemetry},
};
use crate::media::AnalysisVideo;

mod config;
mod dto;
mod upload;

use config::{DEFAULT_HTTP_TIMEOUT, GeminiConfig};
use dto::{
    GenerateContentResponse, GetFileResponse, UploadResponse, UploadedFile, UploadedFileResponse,
};
use upload::{
    UploadChunk, UploadChunksError, UploadCommand, UploadRetryDecision, UploadSession, UploadStats,
    VideoInput, log_upload_completed, next_upload_chunk, select_upload_chunk_size, upload_chunks,
    upload_offset_from_headers, upload_retry_decision, upload_session_from_headers,
    video_mime_type,
};

const UPLOAD_URL: &str = "https://generativelanguage.googleapis.com/upload/v1beta/files";
const API_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta";
const GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const FILE_PROCESSING_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_UPLOAD_CHUNK_ATTEMPTS: usize = 4;
const MAX_UPLOAD_SESSION_ATTEMPTS: usize = 3;

pub async fn analyze_videos(videos: &[AnalysisVideo], prompt: &str) -> Result<String, GeminiError> {
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

    pub fn from_env_with_model(model: Option<String>) -> Result<Self, GeminiError> {
        let mut client = Self::from_env()?;
        if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
            client.config.model = model;
        }

        Ok(client)
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, GeminiError> {
        let telemetry = request.telemetry.clone();
        let videos = request
            .videos
            .iter()
            .map(|asset| VideoInput {
                name: asset.name.clone(),
                bytes: asset.bytes.clone(),
            })
            .collect::<Vec<_>>();
        let upload_chunk_size =
            select_upload_chunk_size(&telemetry, &self.config.upload_chunk_size_buckets);
        let bucket_index = self
            .config
            .upload_chunk_size_buckets
            .iter()
            .position(|bucket| *bucket == upload_chunk_size)
            .unwrap_or_default();
        telemetry.log(
            "info",
            "gemini",
            "upload_chunk_size_selected",
            [
                ("chunk_size", json!(upload_chunk_size)),
                ("bucket_index", json!(bucket_index)),
                (
                    "bucket_count",
                    json!(self.config.upload_chunk_size_buckets.len()),
                ),
                ("strategy", json!("analysis_id_hash")),
            ],
        );

        self.analyze_inputs(&telemetry, &videos, &request.prompt, upload_chunk_size)
            .await
    }

    async fn analyze_inputs(
        &self,
        telemetry: &AnalysisTelemetry,
        videos: &[VideoInput],
        prompt: &str,
        upload_chunk_size: usize,
    ) -> Result<String, GeminiError> {
        let mut files = Vec::with_capacity(videos.len());

        for video in videos {
            files.push(
                self.upload_video(telemetry, video, upload_chunk_size)
                    .await?,
            );
        }

        self.generate_content(telemetry, &files, prompt).await
    }

    async fn upload_video(
        &self,
        telemetry: &AnalysisTelemetry,
        video: &VideoInput,
        upload_chunk_size: usize,
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
                .upload_video_chunks(
                    telemetry,
                    video,
                    &upload_session,
                    mime_type,
                    upload_chunk_size,
                )
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
        upload_chunk_size: usize,
    ) -> Result<UploadResponse, UploadChunksError> {
        let started_at = Instant::now();
        let mut stats = UploadStats::default();
        let total_bytes = video.bytes.len();
        let total_chunks =
            upload_chunks(total_bytes, session.chunk_granularity, upload_chunk_size).len();
        telemetry.log(
            "info",
            "gemini",
            "upload_started",
            [
                ("video_name", json!(video.name)),
                ("bytes", json!(total_bytes)),
                ("mime", json!(mime_type)),
                ("chunks", json!(total_chunks)),
                ("chunk_size", json!(upload_chunk_size)),
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
                upload_chunk_size,
            );
            attempts_at_offset += 1;
            stats.record_send_attempt();
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
                Ok(Some(upload)) => {
                    log_upload_completed(
                        telemetry,
                        video,
                        &stats,
                        total_bytes,
                        total_chunks,
                        upload_chunk_size,
                        session.chunk_granularity,
                        started_at.elapsed().as_millis() as i64,
                    );
                    return Ok(upload);
                }
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
                    stats.record_retry(&error);
                    let received_offset = self
                        .query_upload_offset_with_retries(telemetry, video, session, chunk.offset)
                        .await?;
                    stats.record_offset_query();
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
                            chunk_index =
                                upload_chunks(offset, session.chunk_granularity, upload_chunk_size)
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
        stats.record_send_attempt();
        match self.send_upload_chunk(video, session, chunk).await? {
            Some(upload) => {
                log_upload_completed(
                    telemetry,
                    video,
                    &stats,
                    total_bytes,
                    total_chunks,
                    upload_chunk_size,
                    session.chunk_granularity,
                    started_at.elapsed().as_millis() as i64,
                );
                Ok(upload)
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

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
                            "text": "Find the key moments.\n\nReturn plain text, not Markdown."
                        }
                    ]
                }]
            })
        );
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
                upload_chunk_size_buckets: vec![8],
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
}
