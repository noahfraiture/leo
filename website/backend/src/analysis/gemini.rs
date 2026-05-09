use std::env;

use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, HeaderMap},
};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::db;

const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
const UPLOAD_URL: &str = "https://generativelanguage.googleapis.com/upload/v1beta/files";
const GENERATE_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models";

pub async fn analyze_videos(
    videos: &[db::video::VideoAsset],
    prompt: &str,
) -> Result<String, GeminiError> {
    let client = GeminiClient::from_env()?;
    let videos = videos
        .iter()
        .map(|asset| VideoInput {
            name: asset.video.name.clone(),
            bytes: asset.bytes.clone(),
        })
        .collect::<Vec<_>>();

    client.analyze(&videos, prompt).await
}

struct GeminiClient {
    http: reqwest::Client,
    config: GeminiConfig,
}

struct GeminiConfig {
    api_key: String,
    model: String,
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
    fn from_env() -> Result<Self, GeminiError> {
        let config = GeminiConfig::from_env()?;

        Ok(Self {
            http: reqwest::Client::new(),
            config,
        })
    }

    async fn analyze(&self, videos: &[VideoInput], prompt: &str) -> Result<String, GeminiError> {
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

        Ok(UploadedFile {
            uri: upload.file.uri,
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

        Ok(Self { api_key, model })
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
struct UploadedFileResponse {
    uri: String,
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
}
