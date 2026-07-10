//! Qwen Responses API request construction for frame chunks and summaries.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    analysis::{chunking::FrameChunk, prompts::qwen as prompts},
    media::VideoFrame,
};

use super::config::QwenConfig;

const BASE64_JSON_OVERHEAD_BYTES: usize = 192;

pub(super) struct QwenChunkRequest<'a> {
    pub config: &'a QwenConfig,
    pub prompt: &'a str,
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub chunk: &'a FrameChunk,
}

pub(super) enum QwenImageInput<'a> {
    Base64DataUrl {
        mime_type: &'static str,
        bytes: &'a [u8],
    },
}

impl QwenImageInput<'_> {
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
        }
    }

    pub(super) fn estimated_payload_bytes(&self) -> usize {
        match self {
            Self::Base64DataUrl { mime_type, bytes } => {
                let encoded_len = bytes.len().div_ceil(3) * 4;
                "data:;base64,".len() + mime_type.len() + encoded_len + BASE64_JSON_OVERHEAD_BYTES
            }
        }
    }
}

pub(super) fn qwen_frame_payload_bytes(frame: &VideoFrame) -> usize {
    qwen_frame_image_input(frame).estimated_payload_bytes()
}

pub(super) fn generate_response_request(request: QwenChunkRequest<'_>) -> Value {
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
        content.push(qwen_frame_image_input(frame).to_json(&request.config.image_detail));
    }

    json!({
        "model": request.config.model,
        "instructions": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
        "reasoning": { "effort": "none" },
        "input": [{
            "role": "user",
            "content": content,
        }]
    })
}

pub(super) fn summarize_chunks_request(
    config: &QwenConfig,
    prompt: &str,
    chunks: &[String],
) -> Value {
    json!({
        "model": config.model,
        "instructions": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
        "reasoning": { "effort": "none" },
        "input": [{
            "role": "user",
            "content": [{
                "type": "input_text",
                "text": prompts::final_summary_request(prompt, chunks)
            }]
        }]
    })
}

fn qwen_frame_image_input(frame: &VideoFrame) -> QwenImageInput<'_> {
    QwenImageInput::Base64DataUrl {
        mime_type: frame.mime_type,
        bytes: &frame.bytes,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{analysis::chunking::FrameChunk, media::VideoFrame};

    use super::{
        QwenChunkRequest, QwenImageInput, generate_response_request, qwen_frame_payload_bytes,
        summarize_chunks_request,
    };
    use crate::analysis::qwen::config::QwenConfig;

    #[test]
    fn generate_response_request_uses_lm_studio_model_and_frame_inputs() {
        let config = QwenConfig::from_values(None, None, None, None);
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

        let request = generate_response_request(QwenChunkRequest {
            config: &config,
            prompt: "Find the key moment.",
            chunk_index: 0,
            chunk_count: 1,
            chunk: &chunk,
        });

        assert_eq!(
            request,
            json!({
                "model": "qwen/qwen3.6-35b-a3b",
                "instructions": "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter. Return plain text, not Markdown.",
                "reasoning": { "effort": "none" },
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
            })
        );
    }

    #[test]
    fn summarize_chunks_request_uses_user_prompt_as_final_answer_driver() {
        let config = QwenConfig::from_values(None, None, None, None);

        let request = summarize_chunks_request(&config, "Find the key moment.", &["notes".into()]);

        assert_eq!(
            request["input"][0]["content"][0]["text"],
            "User request:\nFind the key moment.\n\nChunk notes:\nChunk 1:\nnotes\n\nWrite the final answer in plain text, not Markdown. Use timestamps only when helpful. Do not mention chunking or overlap unless relevant."
        );
        assert_eq!(request["reasoning"], json!({ "effort": "none" }));
        assert!(request.get("text").is_none());
    }

    #[test]
    fn image_input_estimates_base64_payload_instead_of_raw_bytes() {
        let bytes = [b'x'; 3];
        let input = QwenImageInput::Base64DataUrl {
            mime_type: "image/jpeg",
            bytes: &bytes,
        };

        assert!(input.estimated_payload_bytes() > bytes.len());
        assert!(input.estimated_payload_bytes() >= "data:image/jpeg;base64,eHh4".len());
    }

    #[test]
    fn qwen_frame_payload_bytes_counts_image_payload() {
        let frame = VideoFrame {
            video_name: "clip.mp4".to_owned(),
            timestamp_secs: 1.0,
            mime_type: "image/jpeg",
            bytes: b"jpeg".to_vec(),
        };

        assert!(qwen_frame_payload_bytes(&frame) > frame.bytes.len());
    }
}
