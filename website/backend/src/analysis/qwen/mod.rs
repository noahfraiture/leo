//! Local Qwen provider client orchestration.

use std::{error::Error as _, time::Duration};

use reqwest::{StatusCode, header::CONTENT_TYPE};
use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::sleep;

use crate::analysis::{
    chunking::{ChunkingOptions, FrameChunk, chunk_frames_by_payload},
    request::{AnalysisRequest, AnalysisTelemetry},
};
use crate::media::frames::{FrameExtractionConfig, extract_video_frames};

mod config;
mod dto;
mod request_builder;

use config::{DEFAULT_HTTP_TIMEOUT, QwenConfig};
use dto::QwenResponse;
use request_builder::{
    QwenChunkRequest, generate_response_request, qwen_frame_payload_bytes, summarize_chunks_request,
};

const MAX_QWEN_REQUEST_ATTEMPTS: usize = 3;

pub struct QwenClient {
    http: reqwest::Client,
    config: QwenConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestFailure {
    timeout: bool,
    connect: bool,
    body: bool,
    request: bool,
}

#[derive(Debug, Error)]
pub enum QwenError {
    #[error("Qwen frame extraction produced no frames")]
    EmptyFrames,
    #[error("Qwen API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("Qwen did not return any text")]
    EmptyResponse,
    #[error(
        "Qwen request failed during {stage} at attempt {attempt}/{attempts} (payload_bytes={payload_bytes}, timeout={timeout}, connect={connect}, body={body}, request={request}, chain={chain}): {source}"
    )]
    Request {
        stage: String,
        attempt: usize,
        attempts: usize,
        payload_bytes: usize,
        timeout: bool,
        connect: bool,
        body: bool,
        request: bool,
        chain: String,
        #[source]
        source: reqwest::Error,
    },
    #[error(transparent)]
    FrameExtraction(#[from] crate::media::frames::FrameExtractionError),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl RequestFailure {
    fn from_error(error: &reqwest::Error) -> Self {
        Self {
            timeout: error.is_timeout(),
            connect: error.is_connect(),
            body: error.is_body(),
            request: error.is_request(),
        }
    }

    fn is_retriable(self) -> bool {
        self.timeout || self.connect || self.body || self.request
    }
}

impl QwenClient {
    pub fn from_env() -> Result<Self, QwenError> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(DEFAULT_HTTP_TIMEOUT)
                .build()?,
            config: QwenConfig::from_env(),
        })
    }

    pub fn from_env_with_model(model: Option<String>) -> Result<Self, QwenError> {
        let mut client = Self::from_env()?;
        if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
            client.config.model = model;
        }

        Ok(client)
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, QwenError> {
        let telemetry = request.telemetry.clone();
        let frames = extract_video_frames(
            &request.videos,
            FrameExtractionConfig::from_sample_rate_fps(request.settings.frame_sample_rate_fps),
        )
        .await?;
        if frames.is_empty() {
            return Err(QwenError::EmptyFrames);
        }

        let frame_count = frames.len();
        let raw_frame_bytes = frames.iter().map(|frame| frame.bytes.len()).sum::<usize>();
        let estimated_frame_payload_bytes =
            frames.iter().map(qwen_frame_payload_bytes).sum::<usize>();
        let chunking = ChunkingOptions::from_env();
        telemetry.log(
            "info",
            "qwen",
            "frames_extracted",
            [
                ("videos", json!(request.videos.len())),
                ("frames", json!(frame_count)),
                ("raw_frame_bytes", json!(raw_frame_bytes)),
                (
                    "estimated_frame_payload_bytes",
                    json!(estimated_frame_payload_bytes),
                ),
                (
                    "sample_rate_fps",
                    json!(request.settings.frame_sample_rate_fps),
                ),
            ],
        );

        let chunks = chunk_frames_by_payload(frames, chunking, qwen_frame_payload_bytes);
        let chunk_count = chunks.len();
        telemetry.log(
            "info",
            "qwen",
            "frames_chunked",
            [
                ("chunks", json!(chunk_count)),
                ("max_images", json!(chunking.max_images_per_request)),
                (
                    "max_payload_bytes",
                    json!(chunking.max_payload_bytes_per_request),
                ),
                ("overlap_percent", json!(chunking.overlap_percent)),
            ],
        );
        let mut responses = Vec::with_capacity(chunk_count);

        for (index, chunk) in chunks.iter().enumerate() {
            responses.push(
                self.analyze_chunk(&telemetry, &request.prompt, index, chunk_count, chunk)
                    .await?,
            );
        }

        self.summarize_chunks(&telemetry, &request.prompt, &responses)
            .await
    }

    async fn analyze_chunk(
        &self,
        telemetry: &AnalysisTelemetry,
        prompt: &str,
        chunk_index: usize,
        chunk_count: usize,
        chunk: &FrameChunk,
    ) -> Result<String, QwenError> {
        let request = generate_response_request(QwenChunkRequest {
            config: &self.config,
            prompt,
            chunk_index,
            chunk_count,
            chunk,
        });
        let payload_bytes = json_payload_size(&request);
        let estimated_frame_payload_bytes = chunk
            .frames
            .iter()
            .map(qwen_frame_payload_bytes)
            .sum::<usize>();
        let stage = format!("chunk {}/{}", chunk_index + 1, chunk_count);
        telemetry.log(
            "info",
            "qwen",
            "chunk_request",
            [
                ("chunk", json!(chunk_index + 1)),
                ("chunks", json!(chunk_count)),
                ("frames", json!(chunk.frames.len())),
                ("start_secs", json!(chunk.start_secs)),
                ("end_secs", json!(chunk.end_secs)),
                (
                    "estimated_frame_payload_bytes",
                    json!(estimated_frame_payload_bytes),
                ),
                ("json_payload_bytes", json!(payload_bytes)),
            ],
        );

        let response = self
            .send_response_request(telemetry, &stage, payload_bytes, &request)
            .await?;
        let text = response.text().ok_or(QwenError::EmptyResponse)?;
        telemetry.log(
            "info",
            "qwen",
            "chunk_response",
            [
                ("chunk", json!(chunk_index + 1)),
                ("chunks", json!(chunk_count)),
                ("chars", json!(text.len())),
            ],
        );

        Ok(text)
    }

    async fn summarize_chunks(
        &self,
        telemetry: &AnalysisTelemetry,
        prompt: &str,
        chunks: &[String],
    ) -> Result<String, QwenError> {
        let request = summarize_chunks_request(&self.config, prompt, chunks);
        let payload_bytes = json_payload_size(&request);
        telemetry.log(
            "info",
            "qwen",
            "summary_request",
            [
                ("chunks", json!(chunks.len())),
                ("json_payload_bytes", json!(payload_bytes)),
            ],
        );

        let response = self
            .send_response_request(telemetry, "summary", payload_bytes, &request)
            .await?;
        let text = response.text().ok_or(QwenError::EmptyResponse)?;
        telemetry.log(
            "info",
            "qwen",
            "summary_response",
            [("chars", json!(text.len()))],
        );

        Ok(text)
    }

    async fn send_response_request(
        &self,
        telemetry: &AnalysisTelemetry,
        stage: &str,
        payload_bytes: usize,
        body: &Value,
    ) -> Result<QwenResponse, QwenError> {
        for attempt in 1..=MAX_QWEN_REQUEST_ATTEMPTS {
            telemetry.log(
                "info",
                "qwen",
                "request_send",
                [
                    ("stage", json!(stage)),
                    ("attempt", json!(attempt)),
                    ("attempts", json!(MAX_QWEN_REQUEST_ATTEMPTS)),
                    ("payload_bytes", json!(payload_bytes)),
                ],
            );

            let mut request = self
                .http
                .post(self.config.responses_url())
                .header(CONTENT_TYPE, "application/json")
                .json(body);
            if let Some(api_key) = &self.config.api_key {
                request = request.bearer_auth(api_key);
            }

            let response = request.send().await;

            match response {
                Ok(response) => return success_json(response).await,
                Err(source) => {
                    let failure = RequestFailure::from_error(&source);
                    let chain = error_chain(&source);
                    if failure.is_retriable() && attempt < MAX_QWEN_REQUEST_ATTEMPTS {
                        telemetry.log(
                            "warn",
                            "qwen",
                            "request_retry",
                            [
                                ("stage", json!(stage)),
                                ("attempt", json!(attempt)),
                                ("attempts", json!(MAX_QWEN_REQUEST_ATTEMPTS)),
                                ("payload_bytes", json!(payload_bytes)),
                                ("timeout", json!(failure.timeout)),
                                ("connect", json!(failure.connect)),
                                ("body", json!(failure.body)),
                                ("request", json!(failure.request)),
                                ("error", json!(source.to_string())),
                                ("chain", json!(chain)),
                            ],
                        );
                        sleep(Duration::from_secs(attempt as u64)).await;
                        continue;
                    }

                    return Err(QwenError::Request {
                        stage: stage.to_owned(),
                        attempt,
                        attempts: MAX_QWEN_REQUEST_ATTEMPTS,
                        payload_bytes,
                        timeout: failure.timeout,
                        connect: failure.connect,
                        body: failure.body,
                        request: failure.request,
                        chain,
                        source,
                    });
                }
            }
        }

        unreachable!("Qwen request retry loop should return")
    }
}

async fn success_json<T>(response: reqwest::Response) -> Result<T, QwenError>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();

    if status.is_success() {
        return Ok(response.json().await?);
    }

    Err(QwenError::Api {
        status,
        body: response.text().await.unwrap_or_default(),
    })
}

fn json_payload_size(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_default()
}

fn error_chain(error: &reqwest::Error) -> String {
    let mut messages = Vec::new();
    let mut source = error.source();

    while let Some(error) = source {
        messages.push(error.to_string());
        source = error.source();
    }

    if messages.is_empty() {
        "none".to_owned()
    } else {
        messages.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::RequestFailure;

    #[test]
    fn request_failures_mark_transient_send_errors_as_retriable() {
        assert!(
            RequestFailure {
                timeout: true,
                connect: false,
                body: false,
                request: false,
            }
            .is_retriable()
        );
        assert!(
            RequestFailure {
                timeout: false,
                connect: true,
                body: false,
                request: false,
            }
            .is_retriable()
        );
        assert!(
            RequestFailure {
                timeout: false,
                connect: false,
                body: true,
                request: false,
            }
            .is_retriable()
        );
        assert!(
            RequestFailure {
                timeout: false,
                connect: false,
                body: false,
                request: true,
            }
            .is_retriable()
        );
        assert!(
            !RequestFailure {
                timeout: false,
                connect: false,
                body: false,
                request: false,
            }
            .is_retriable()
        );
    }
}
