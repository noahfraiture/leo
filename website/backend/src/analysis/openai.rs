use std::env;

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::analysis::{
    chunking::{ChunkingOptions, FrameChunk, chunk_frames},
    frames::{FrameExtractionConfig, extract_video_frames},
    request::AnalysisRequest,
};

const DEFAULT_MODEL: &str = "gpt-5.5";
const DEFAULT_IMAGE_DETAIL: &str = "low";
const RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

pub struct OpenAiClient {
    http: reqwest::Client,
    config: OpenAiConfig,
}

pub struct OpenAiConfig {
    api_key: String,
    model: String,
    image_detail: String,
}

struct OpenAiChunkRequest<'a> {
    config: &'a OpenAiConfig,
    prompt: &'a str,
    chunk_index: usize,
    chunk_count: usize,
    chunk: &'a FrameChunk,
}

#[derive(Debug, Error)]
pub enum OpenAiError {
    #[error("OPENAI_API_KEY is not configured")]
    MissingApiKey,
    #[error("OpenAI frame extraction produced no frames")]
    EmptyFrames,
    #[error("OpenAI API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("OpenAI did not return any text")]
    EmptyResponse,
    #[error(transparent)]
    FrameExtraction(#[from] crate::analysis::frames::FrameExtractionError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl OpenAiClient {
    pub fn from_env() -> Result<Self, OpenAiError> {
        Ok(Self {
            http: reqwest::Client::new(),
            config: OpenAiConfig::from_env()?,
        })
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, OpenAiError> {
        let frames =
            extract_video_frames(&request.videos, FrameExtractionConfig::from_env()).await?;
        if frames.is_empty() {
            return Err(OpenAiError::EmptyFrames);
        }

        let chunks = chunk_frames(frames, ChunkingOptions::from_env());
        let chunk_count = chunks.len();
        let mut responses = Vec::with_capacity(chunk_count);

        for (index, chunk) in chunks.iter().enumerate() {
            responses.push(
                self.analyze_chunk(&request.prompt, index, chunk_count, chunk)
                    .await?,
            );
        }

        if responses.len() == 1 {
            Ok(responses.remove(0))
        } else {
            self.summarize_chunks(&request.prompt, &responses).await
        }
    }

    async fn analyze_chunk(
        &self,
        prompt: &str,
        chunk_index: usize,
        chunk_count: usize,
        chunk: &FrameChunk,
    ) -> Result<String, OpenAiError> {
        let response = self
            .http
            .post(RESPONSES_URL)
            .bearer_auth(&self.config.api_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&generate_response_request(OpenAiChunkRequest {
                config: &self.config,
                prompt,
                chunk_index,
                chunk_count,
                chunk,
            }))
            .send()
            .await?;
        let response: OpenAiResponse = success_json(response).await?;

        response.text().ok_or(OpenAiError::EmptyResponse)
    }

    async fn summarize_chunks(
        &self,
        prompt: &str,
        chunks: &[String],
    ) -> Result<String, OpenAiError> {
        let response = self
            .http
            .post(RESPONSES_URL)
            .bearer_auth(&self.config.api_key)
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({
                "model": self.config.model,
                "input": [{
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": format!(
                            "Merge these partial video analyses into one answer for the original prompt.\n\nOriginal prompt:\n{prompt}\n\nPartial analyses:\n{}",
                            chunks
                                .iter()
                                .enumerate()
                                .map(|(index, chunk)| format!("Chunk {}:\n{}", index + 1, chunk))
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        )
                    }]
                }],
                "text": {
                    "verbosity": "low"
                }
            }))
            .send()
            .await?;
        let response: OpenAiResponse = success_json(response).await?;

        response.text().ok_or(OpenAiError::EmptyResponse)
    }
}

impl OpenAiConfig {
    fn from_env() -> Result<Self, OpenAiError> {
        let api_key = env::var("OPENAI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(OpenAiError::MissingApiKey)?;
        let model = env::var("OPENAI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let image_detail = env::var("OPENAI_IMAGE_DETAIL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_IMAGE_DETAIL.to_owned());

        Ok(Self {
            api_key,
            model,
            image_detail,
        })
    }
}

async fn success_json<T>(response: reqwest::Response) -> Result<T, OpenAiError>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();

    if status.is_success() {
        return Ok(response.json().await?);
    }

    Err(OpenAiError::Api {
        status,
        body: response.text().await.unwrap_or_default(),
    })
}

fn generate_response_request(request: OpenAiChunkRequest<'_>) -> Value {
    let mut content = vec![json!({
        "type": "input_text",
        "text": format!(
            "Analyze the provided video frames for this time window and answer the user's prompt.\n\nUser prompt:\n{}\n\nWindow: chunk {} of {}, {:.3}s to {:.3}s.\nFrames are ordered by timestamp across all selected videos.",
            request.prompt,
            request.chunk_index + 1,
            request.chunk_count,
            request.chunk.start_secs,
            request.chunk.end_secs,
        ),
    })];

    for (index, frame) in request.chunk.frames.iter().enumerate() {
        content.push(json!({
            "type": "input_text",
            "text": format!(
                "Frame {}: video={} timestamp={:.3}s",
                index + 1,
                frame.video_name,
                frame.timestamp_secs,
            ),
        }));
        content.push(json!({
            "type": "input_image",
            "image_url": format!(
                "data:{};base64,{}",
                frame.mime_type,
                STANDARD.encode(&frame.bytes),
            ),
            "detail": request.config.image_detail,
        }));
    }

    json!({
        "model": request.config.model,
        "input": [{
            "role": "user",
            "content": content,
        }],
        "text": {
            "verbosity": "low"
        }
    })
}

#[derive(Deserialize)]
struct OpenAiResponse {
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    output: Vec<OpenAiOutputItem>,
}

impl OpenAiResponse {
    fn text(self) -> Option<String> {
        let text = self
            .output_text
            .into_iter()
            .chain(
                self.output
                    .into_iter()
                    .flat_map(|item| item.content)
                    .filter_map(|content| content.text),
            )
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        if text.is_empty() { None } else { Some(text) }
    }
}

#[derive(Deserialize)]
struct OpenAiOutputItem {
    #[serde(default)]
    content: Vec<OpenAiContent>,
}

#[derive(Deserialize)]
struct OpenAiContent {
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{OpenAiChunkRequest, OpenAiConfig, generate_response_request};
    use crate::analysis::{chunking::FrameChunk, request::VideoFrame};

    #[test]
    fn generate_response_request_places_prompt_metadata_and_images_in_one_message() {
        let config = OpenAiConfig {
            api_key: "test-key".to_owned(),
            model: "gpt-test".to_owned(),
            image_detail: "low".to_owned(),
        };
        let chunk = FrameChunk {
            start_secs: 0.0,
            end_secs: 5.0,
            frames: vec![VideoFrame {
                video_name: "clip.mp4".to_owned(),
                timestamp_secs: 5.0,
                mime_type: "image/jpeg",
                bytes: b"jpeg".to_vec(),
            }],
        };

        let request = generate_response_request(OpenAiChunkRequest {
            config: &config,
            prompt: "Find the key moment.",
            chunk_index: 0,
            chunk_count: 1,
            chunk: &chunk,
        });

        assert_eq!(
            request,
            json!({
                "model": "gpt-test",
                "input": [{
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "Analyze the provided video frames for this time window and answer the user's prompt.\n\nUser prompt:\nFind the key moment.\n\nWindow: chunk 1 of 1, 0.000s to 5.000s.\nFrames are ordered by timestamp across all selected videos."
                        },
                        {
                            "type": "input_text",
                            "text": "Frame 1: video=clip.mp4 timestamp=5.000s"
                        },
                        {
                            "type": "input_image",
                            "image_url": "data:image/jpeg;base64,anBlZw==",
                            "detail": "low"
                        }
                    ]
                }],
                "text": {
                    "verbosity": "low"
                }
            })
        );
    }
}
