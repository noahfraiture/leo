use std::env;

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;

use crate::analysis::{
    chunking::{ChunkingOptions, FrameChunk, chunk_frames_by_payload},
    frames::{FrameExtractionConfig, extract_video_frames},
    request::{AnalysisRequest, VideoFrame},
};

const DEFAULT_MODEL: &str = "gpt-5.5";
const DEFAULT_IMAGE_DETAIL: &str = "low";
const RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const BASE64_JSON_OVERHEAD_BYTES: usize = 192;

mod prompts {
    pub const VIDEO_ANALYSIS_INSTRUCTIONS: &str = "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter.";

    pub fn chunk_evidence_request(
        user_prompt: &str,
        chunk_number: usize,
        chunk_count: usize,
        start_secs: f64,
        end_secs: f64,
    ) -> String {
        format!(
            "User request:\n{user_prompt}\n\nChunk {chunk_number} of {chunk_count} covers {start_secs:.3}s to {end_secs:.3}s.\nReturn concise evidence notes only: relevant observations, video names, timestamps, and uncertainty."
        )
    }

    pub fn frame_metadata(frame_number: usize, video_name: &str, timestamp_secs: f64) -> String {
        format!("Frame {frame_number}: video={video_name} timestamp={timestamp_secs:.3}s")
    }

    pub fn final_summary_request(user_prompt: &str, chunk_notes: &[String]) -> String {
        format!(
            "User request:\n{user_prompt}\n\nChunk notes:\n{}\n\nWrite the final answer. Use timestamps only when helpful. Do not mention chunking or overlap unless relevant.",
            chunk_notes
                .iter()
                .enumerate()
                .map(|(index, chunk)| format!("Chunk {}:\n{}", index + 1, chunk))
                .collect::<Vec<_>>()
                .join("\n\n")
        )
    }
}

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

enum OpenAiImageInput<'a> {
    // Base64 data URLs keep the app fully local for now. This enum is the seam
    // for switching to Files API image inputs later without changing chunking
    // or prompt construction.
    Base64DataUrl {
        mime_type: &'static str,
        bytes: &'a [u8],
    },
    #[allow(dead_code)]
    FileId { file_id: &'a str },
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
        let frames = extract_video_frames(
            &request.videos,
            FrameExtractionConfig::from_sample_rate_fps(request.settings.frame_sample_rate_fps),
        )
        .await?;
        if frames.is_empty() {
            return Err(OpenAiError::EmptyFrames);
        }

        let chunks = chunk_frames_by_payload(frames, ChunkingOptions::from_env(), |frame| {
            openai_frame_image_input(frame).estimated_payload_bytes()
        });
        let chunk_count = chunks.len();
        let mut responses = Vec::with_capacity(chunk_count);

        for (index, chunk) in chunks.iter().enumerate() {
            responses.push(
                self.analyze_chunk(&request.prompt, index, chunk_count, chunk)
                    .await?,
            );
        }

        self.summarize_chunks(&request.prompt, &responses).await
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
            .json(&summarize_chunks_request(&self.config, prompt, chunks))
            .send()
            .await?;
        let response: OpenAiResponse = success_json(response).await?;

        response.text().ok_or(OpenAiError::EmptyResponse)
    }
}

impl OpenAiImageInput<'_> {
    fn to_json(&self, detail: &str) -> Value {
        match self {
            Self::Base64DataUrl { mime_type, bytes } => json!({
                "type": "input_image",
                "image_url": format!(
                    "data:{};base64,{}",
                    mime_type,
                    STANDARD.encode(bytes),
                ),
                "detail": detail,
            }),
            Self::FileId { file_id } => json!({
                "type": "input_image",
                "file_id": file_id,
                "detail": detail,
            }),
        }
    }

    fn estimated_payload_bytes(&self) -> usize {
        match self {
            Self::Base64DataUrl { mime_type, bytes } => {
                let encoded_len = bytes.len().div_ceil(3) * 4;
                "data:;base64,".len() + mime_type.len() + encoded_len + BASE64_JSON_OVERHEAD_BYTES
            }
            Self::FileId { file_id } => file_id.len() + BASE64_JSON_OVERHEAD_BYTES,
        }
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
        "text": prompts::chunk_evidence_request(
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
            "text": prompts::frame_metadata(
                index + 1,
                &frame.video_name,
                frame.timestamp_secs,
            ),
        }));
        content.push(openai_frame_image_input(frame).to_json(&request.config.image_detail));
    }

    json!({
        "model": request.config.model,
        "instructions": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
        "input": [{
            "role": "user",
            "content": content,
        }],
        "text": {
            "verbosity": "low"
        }
    })
}

fn summarize_chunks_request(config: &OpenAiConfig, prompt: &str, chunks: &[String]) -> Value {
    json!({
        "model": config.model,
        "instructions": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
        "input": [{
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": prompts::final_summary_request(prompt, chunks)
            }]
        }],
        "text": {
            "verbosity": "low"
        }
    })
}

fn openai_frame_image_input(frame: &VideoFrame) -> OpenAiImageInput<'_> {
    OpenAiImageInput::Base64DataUrl {
        mime_type: frame.mime_type,
        bytes: &frame.bytes,
    }
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

    use super::{
        OpenAiChunkRequest, OpenAiConfig, OpenAiImageInput, generate_response_request,
        summarize_chunks_request,
    };
    use crate::analysis::{chunking::FrameChunk, request::VideoFrame};

    #[test]
    fn generate_response_request_uses_minimal_instructions_and_evidence_prompt() {
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
                "instructions": "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter.",
                "input": [{
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "User request:\nFind the key moment.\n\nChunk 1 of 1 covers 0.000s to 5.000s.\nReturn concise evidence notes only: relevant observations, video names, timestamps, and uncertainty."
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

    #[test]
    fn summarize_chunks_request_uses_user_prompt_as_final_answer_driver() {
        let config = OpenAiConfig {
            api_key: "test-key".to_owned(),
            model: "gpt-test".to_owned(),
            image_detail: "low".to_owned(),
        };

        let request = summarize_chunks_request(&config, "Find the key moment.", &["notes".into()]);

        assert_eq!(
            request["input"][0]["content"][0]["text"],
            "User request:\nFind the key moment.\n\nChunk notes:\nChunk 1:\nnotes\n\nWrite the final answer. Use timestamps only when helpful. Do not mention chunking or overlap unless relevant."
        );
    }

    #[test]
    fn image_input_estimates_base64_payload_instead_of_raw_bytes() {
        let bytes = [b'x'; 3];
        let input = OpenAiImageInput::Base64DataUrl {
            mime_type: "image/jpeg",
            bytes: &bytes,
        };

        assert!(input.estimated_payload_bytes() > bytes.len());
        assert!(input.estimated_payload_bytes() >= "data:image/jpeg;base64,eHh4".len());
    }
}
