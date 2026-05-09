use std::{env, time::Duration};

use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap},
};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::{Instant, sleep};

use crate::{analysis::request::AnalysisRequest, db};

const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
const UPLOAD_URL: &str = "https://generativelanguage.googleapis.com/upload/v1beta/files";
const API_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta";
const GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const DEFAULT_FILE_PROCESSING_TIMEOUT: Duration = Duration::from_secs(300);
const FILE_PROCESSING_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub async fn analyze_videos(
    videos: &[db::video::VideoAsset],
    prompt: &str,
) -> Result<String, GeminiError> {
    let client = GeminiClient::from_env()?;

    client
        .analyze(AnalysisRequest {
            videos: videos.to_vec(),
            prompt: prompt.to_owned(),
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
}

struct VideoInput {
    name: String,
    bytes: Vec<u8>,
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
    #[error("Gemini file {name} failed while processing")]
    FileProcessingFailed { name: String },
    #[error("Gemini file {name} did not become active within {timeout_secs} seconds")]
    FileProcessingTimedOut { name: String, timeout_secs: u64 },
    #[error("Gemini API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("Gemini did not return any text")]
    EmptyResponse,
    #[error(transparent)]
    Header(#[from] reqwest::header::ToStrError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl GeminiClient {
    pub fn from_env() -> Result<Self, GeminiError> {
        let config = GeminiConfig::from_env()?;

        Ok(Self {
            http: reqwest::Client::new(),
            config,
        })
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, GeminiError> {
        let videos = request
            .videos
            .iter()
            .map(|asset| VideoInput {
                name: asset.video.name.clone(),
                bytes: asset.bytes.clone(),
            })
            .collect::<Vec<_>>();

        self.analyze_inputs(&videos, &request.prompt).await
    }

    async fn analyze_inputs(
        &self,
        videos: &[VideoInput],
        prompt: &str,
    ) -> Result<String, GeminiError> {
        let mut files = Vec::with_capacity(videos.len());

        for video in videos {
            files.push(self.upload_video(video).await?);
        }

        self.generate_content(&files, prompt).await
    }

    async fn upload_video(&self, video: &VideoInput) -> Result<UploadedFile, GeminiError> {
        let mime_type = video_mime_type(&video.name);
        let upload_url = self.start_upload(video, mime_type).await?;
        let response = self
            .http
            .post(upload_url)
            .header(CONTENT_LENGTH, video.bytes.len())
            .header("X-Goog-Upload-Offset", "0")
            .header("X-Goog-Upload-Command", "upload, finalize")
            .body(video.bytes.clone())
            .send()
            .await?;
        let upload: UploadResponse = success_json(response).await?;
        let file = self.wait_for_file_active(upload.file).await?;

        Ok(UploadedFile {
            uri: file.uri,
            mime_type: mime_type.to_owned(),
        })
    }

    async fn start_upload(
        &self,
        video: &VideoInput,
        mime_type: &'static str,
    ) -> Result<String, GeminiError> {
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
        let upload_url = headers
            .get("x-goog-upload-url")
            .ok_or(GeminiError::MissingUploadUrl)?
            .to_str()?;

        Ok(upload_url.to_owned())
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
        files: &[UploadedFile],
        prompt: &str,
    ) -> Result<String, GeminiError> {
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

        response.text().ok_or(GeminiError::EmptyResponse)
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

        Ok(Self {
            api_key,
            model,
            file_processing_timeout,
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
    parts.push(json!({ "text": prompt }));

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
    fn file_state_detects_ready_and_failed_states() {
        assert!(FileState::Active.is_active());
        assert!(!FileState::Processing.is_active());
        assert!(FileState::Failed.is_failed());
        assert!(FileState::Unspecified.is_waitable());
        assert!(FileState::Processing.is_waitable());
        assert!(!FileState::Failed.is_waitable());
    }
}
