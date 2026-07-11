//! Mistral Chat Completions request construction for frame chunks and summaries.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    analysis::{chunking::FrameChunk, prompts::mistral as prompts},
    media::VideoFrame,
};

use super::config::MistralConfig;

const BASE64_JSON_OVERHEAD_BYTES: usize = 192;
const JPEG_DATA_URL_PREFIX: &str = "data:image/jpeg;base64,";

pub(super) struct MistralChunkRequest<'a> {
    pub config: &'a MistralConfig,
    pub prompt: &'a str,
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub chunk: &'a FrameChunk,
}

pub(super) fn generate_chat_completion_request(request: MistralChunkRequest<'_>) -> Value {
    let mut content = vec![json!({
        "type": "text",
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
            "type": "text",
            "text": prompts::frame_metadata(
                index + 1,
                &frame.video_name,
                frame.timestamp_secs,
            ),
        }));
        content.push(json!({
            "type": "image_url",
            "image_url": format!("{JPEG_DATA_URL_PREFIX}{}", STANDARD.encode(&frame.bytes)),
        }));
    }

    json!({
        "model": request.config.model,
        "messages": [
            {
                "role": "system",
                "content": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
            },
            {
                "role": "user",
                "content": content,
            }
        ],
    })
}

pub(super) fn summarize_chunks_request(
    config: &MistralConfig,
    prompt: &str,
    chunks: &[String],
) -> Value {
    json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": prompts::VIDEO_ANALYSIS_INSTRUCTIONS,
            },
            {
                "role": "user",
                "content": prompts::final_summary_request(prompt, chunks),
            }
        ],
    })
}

pub(super) fn mistral_frame_payload_bytes(frame: &VideoFrame) -> usize {
    let encoded_len = frame.bytes.len().div_ceil(3) * 4;
    JPEG_DATA_URL_PREFIX.len() + encoded_len + BASE64_JSON_OVERHEAD_BYTES
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{analysis::chunking::FrameChunk, media::VideoFrame};

    use super::{
        MistralChunkRequest, generate_chat_completion_request, mistral_frame_payload_bytes,
        summarize_chunks_request,
    };
    use crate::analysis::mistral::config::MistralConfig;

    fn config() -> MistralConfig {
        MistralConfig::from_values(Some("test-key"), Some("mistral-test"))
            .expect("configuration should be valid")
    }

    #[test]
    fn chunk_request_uses_exact_chat_completions_content_types() {
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

        let request = generate_chat_completion_request(MistralChunkRequest {
            config: &config(),
            prompt: "Find the key moment.",
            chunk_index: 0,
            chunk_count: 1,
            chunk: &chunk,
        });

        assert_eq!(
            request,
            json!({
                "model": "mistral-test",
                "messages": [
                    {
                        "role": "system",
                        "content": "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter. Return plain text, not Markdown."
                    },
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "text",
                                "text": "User request:\nFind the key moment.\n\nChunk 1 of 1 covers 0.000s to 5.000s.\nReturn concise evidence notes only: relevant observations, video names, timestamps, and uncertainty."
                            },
                            {
                                "type": "text",
                                "text": "Frame 1: video=clip.mp4 timestamp=5.000s"
                            },
                            {
                                "type": "image_url",
                                "image_url": "data:image/jpeg;base64,anBlZw=="
                            }
                        ]
                    }
                ]
            })
        );
    }

    #[test]
    fn summary_request_has_plain_text_user_content_and_no_images() {
        let request =
            summarize_chunks_request(&config(), "Find the key moment.", &["notes".into()]);

        assert_eq!(
            request,
            json!({
                "model": "mistral-test",
                "messages": [
                    {
                        "role": "system",
                        "content": "Analyze sampled video frames. Frames are chronological, may be chunked with overlap, and include video names and timestamps. Follow the user's request; use precise timestamps when they matter. Return plain text, not Markdown."
                    },
                    {
                        "role": "user",
                        "content": "User request:\nFind the key moment.\n\nChunk notes:\nChunk 1:\nnotes\n\nWrite the final answer in plain text, not Markdown. Use timestamps only when helpful. Do not mention chunking or overlap unless relevant."
                    }
                ]
            })
        );
    }

    #[test]
    fn frame_payload_estimate_accounts_for_base64_expansion() {
        let frame = VideoFrame {
            video_name: "clip.mp4".to_owned(),
            timestamp_secs: 1.0,
            mime_type: "image/jpeg",
            bytes: b"xxx".to_vec(),
        };

        let estimated_bytes = mistral_frame_payload_bytes(&frame);

        assert!(estimated_bytes > frame.bytes.len());
        assert!(estimated_bytes >= "data:image/jpeg;base64,eHh4".len());
    }
}
