//! Hosted Mistral provider client orchestration.

use std::{error::Error as _, time::Duration};

use reqwest::{
    StatusCode,
    header::{CONTENT_TYPE, RETRY_AFTER},
};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::sleep;

use crate::analysis::{
    chunking::{ChunkingOptions, FrameChunk, chunk_frames_by_payload},
    request::{AnalysisRequest, AnalysisTelemetry},
};
use crate::media::{
    VideoFrame,
    frames::{FrameExtractionConfig, extract_video_frames},
};

mod config;
mod dto;
mod request_builder;

use config::{DEFAULT_HTTP_TIMEOUT, MistralConfig};
use dto::MistralResponse;
use request_builder::{
    MistralChunkRequest, generate_chat_completion_request, mistral_frame_payload_bytes,
    summarize_chunks_request,
};

const MISTRAL_CHAT_COMPLETIONS_URL: &str = "https://api.mistral.ai/v1/chat/completions";
const MAX_MISTRAL_REQUEST_ATTEMPTS: usize = 3;
const MAX_MISTRAL_IMAGES_PER_REQUEST: usize = 8;
const MAX_MISTRAL_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_RETRY_AFTER_SECS: u64 = 5;
const MAX_MISTRAL_API_ERROR_BODY_CHARS: usize = 4096;
const MISTRAL_API_ERROR_BODY_TRUNCATION_MARKER: &str = "\n[truncated]";

pub struct MistralClient {
    http: reqwest::Client,
    config: MistralConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestFailure {
    timeout: bool,
    connect: bool,
    body: bool,
    request: bool,
    decode: bool,
}

#[derive(Debug, Error)]
pub enum MistralError {
    #[error("MISTRAL_API_KEY is not configured")]
    MissingApiKey,
    #[error("Mistral frame extraction produced no frames")]
    EmptyFrames,
    #[error(
        "Mistral frame from {video_name} at {timestamp_secs:.3}s is {actual_bytes} bytes; limit is {limit_bytes} bytes"
    )]
    FrameTooLarge {
        video_name: String,
        timestamp_secs: f64,
        actual_bytes: usize,
        limit_bytes: usize,
    },
    #[error("Mistral API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("Mistral did not return any text")]
    EmptyResponse,
    #[error(
        "Mistral request failed during {stage} at attempt {attempt}/{attempts} (payload_bytes={payload_bytes}, timeout={timeout}, connect={connect}, body={body}, request={request}, decode={decode}, chain={chain}): {source}"
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
        decode: bool,
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
        let mut failure = Self {
            timeout: error.is_timeout(),
            connect: error.is_connect(),
            body: error.is_body(),
            request: error.is_request(),
            decode: error.is_decode(),
        };
        let mut source = error.source();

        while let Some(error) = source {
            if let Some(error) = error.downcast_ref::<reqwest::Error>() {
                failure.timeout |= error.is_timeout();
                failure.connect |= error.is_connect();
                failure.body |= error.is_body();
                failure.request |= error.is_request();
                failure.decode |= error.is_decode();
            }
            source = error.source();
        }

        failure
    }

    fn is_retriable(self) -> bool {
        self.timeout || self.connect || self.body || self.request
    }
}

impl MistralClient {
    pub fn from_env() -> Result<Self, MistralError> {
        let config = MistralConfig::from_env()?;
        let http = reqwest::Client::builder()
            .timeout(DEFAULT_HTTP_TIMEOUT)
            .build()?;

        Ok(Self { http, config })
    }

    pub fn from_env_with_model(model: Option<String>) -> Result<Self, MistralError> {
        let mut client = Self::from_env()?;
        if let Some(model) = model
            .as_deref()
            .map(str::trim)
            .filter(|model| !model.is_empty())
        {
            client.config.model = model.to_owned();
        }

        Ok(client)
    }

    pub async fn analyze(&self, request: AnalysisRequest) -> Result<String, MistralError> {
        let telemetry = request.telemetry.clone();
        let frames = extract_video_frames(
            &request.videos,
            FrameExtractionConfig::from_sample_rate_fps(request.settings.frame_sample_rate_fps),
        )
        .await?;
        if frames.is_empty() {
            return Err(MistralError::EmptyFrames);
        }

        for frame in &frames {
            validate_frame_size(frame)?;
        }

        let frame_count = frames.len();
        let raw_frame_bytes = frames.iter().map(|frame| frame.bytes.len()).sum::<usize>();
        let estimated_frame_payload_bytes = frames
            .iter()
            .map(mistral_frame_payload_bytes)
            .sum::<usize>();
        let chunking = mistral_chunking_options(ChunkingOptions::from_env());
        telemetry.log(
            "info",
            "mistral",
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

        let chunks = chunk_frames_by_payload(frames, chunking, mistral_frame_payload_bytes);
        let chunk_count = chunks.len();
        telemetry.log(
            "info",
            "mistral",
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
    ) -> Result<String, MistralError> {
        let request = generate_chat_completion_request(MistralChunkRequest {
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
            .map(mistral_frame_payload_bytes)
            .sum::<usize>();
        let stage = format!("chunk {}/{}", chunk_index + 1, chunk_count);
        telemetry.log(
            "info",
            "mistral",
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
            .send_chat_completion_request(telemetry, &stage, payload_bytes, &request)
            .await?;
        let text = response.text().ok_or(MistralError::EmptyResponse)?;
        telemetry.log(
            "info",
            "mistral",
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
    ) -> Result<String, MistralError> {
        let request = summarize_chunks_request(&self.config, prompt, chunks);
        let payload_bytes = json_payload_size(&request);
        telemetry.log(
            "info",
            "mistral",
            "summary_request",
            [
                ("chunks", json!(chunks.len())),
                ("json_payload_bytes", json!(payload_bytes)),
            ],
        );

        let response = self
            .send_chat_completion_request(telemetry, "summary", payload_bytes, &request)
            .await?;
        let text = response.text().ok_or(MistralError::EmptyResponse)?;
        telemetry.log(
            "info",
            "mistral",
            "summary_response",
            [("chars", json!(text.len()))],
        );

        Ok(text)
    }

    async fn send_chat_completion_request(
        &self,
        telemetry: &AnalysisTelemetry,
        stage: &str,
        payload_bytes: usize,
        body: &Value,
    ) -> Result<MistralResponse, MistralError> {
        self.send_chat_completion_request_to(
            telemetry,
            stage,
            payload_bytes,
            body,
            MISTRAL_CHAT_COMPLETIONS_URL,
        )
        .await
    }

    async fn send_chat_completion_request_to(
        &self,
        telemetry: &AnalysisTelemetry,
        stage: &str,
        payload_bytes: usize,
        body: &Value,
        endpoint: &str,
    ) -> Result<MistralResponse, MistralError> {
        for attempt in 1..=MAX_MISTRAL_REQUEST_ATTEMPTS {
            telemetry.log(
                "info",
                "mistral",
                "request_send",
                [
                    ("stage", json!(stage)),
                    ("attempt", json!(attempt)),
                    ("attempts", json!(MAX_MISTRAL_REQUEST_ATTEMPTS)),
                    ("payload_bytes", json!(payload_bytes)),
                ],
            );
            let response = self
                .http
                .post(endpoint)
                .bearer_auth(&self.config.api_key)
                .header(CONTENT_TYPE, "application/json")
                .json(body)
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status();
                    let delay = retry_delay(
                        response
                            .headers()
                            .get(RETRY_AFTER)
                            .and_then(|value| value.to_str().ok()),
                        attempt as u64,
                    );

                    if status.is_success() {
                        match response.json().await {
                            Ok(response) => return Ok(response),
                            Err(source) => {
                                retry_transport_failure(
                                    telemetry,
                                    stage,
                                    attempt,
                                    payload_bytes,
                                    delay,
                                    source,
                                )
                                .await?;
                                continue;
                            }
                        }
                    }

                    let body = match response.text().await {
                        Ok(body) => body,
                        Err(source) => {
                            retry_transport_failure(
                                telemetry,
                                stage,
                                attempt,
                                payload_bytes,
                                delay,
                                source,
                            )
                            .await?;
                            continue;
                        }
                    };

                    if is_retriable_status(status) && attempt < MAX_MISTRAL_REQUEST_ATTEMPTS {
                        telemetry.log(
                            "warn",
                            "mistral",
                            "request_retry",
                            [
                                ("stage", json!(stage)),
                                ("attempt", json!(attempt)),
                                ("attempts", json!(MAX_MISTRAL_REQUEST_ATTEMPTS)),
                                ("payload_bytes", json!(payload_bytes)),
                                ("status", json!(status.as_u16())),
                                ("retry_after_secs", json!(delay.as_secs())),
                            ],
                        );
                        sleep(delay).await;
                        continue;
                    }

                    return Err(MistralError::Api {
                        status,
                        body: bounded_api_error_body(body),
                    });
                }
                Err(source) => {
                    retry_transport_failure(
                        telemetry,
                        stage,
                        attempt,
                        payload_bytes,
                        retry_delay(None, attempt as u64),
                        source,
                    )
                    .await?;
                    continue;
                }
            }
        }

        unreachable!("Mistral request retry loop should return")
    }
}

async fn retry_transport_failure(
    telemetry: &AnalysisTelemetry,
    stage: &str,
    attempt: usize,
    payload_bytes: usize,
    delay: Duration,
    source: reqwest::Error,
) -> Result<(), MistralError> {
    let failure = RequestFailure::from_error(&source);
    let chain = error_chain(&source);
    if failure.is_retriable() && attempt < MAX_MISTRAL_REQUEST_ATTEMPTS {
        telemetry.log(
            "warn",
            "mistral",
            "request_retry",
            [
                ("stage", json!(stage)),
                ("attempt", json!(attempt)),
                ("attempts", json!(MAX_MISTRAL_REQUEST_ATTEMPTS)),
                ("payload_bytes", json!(payload_bytes)),
                ("timeout", json!(failure.timeout)),
                ("connect", json!(failure.connect)),
                ("body", json!(failure.body)),
                ("request", json!(failure.request)),
                ("decode", json!(failure.decode)),
                ("retry_after_secs", json!(delay.as_secs())),
                ("error", json!(source.to_string())),
                ("chain", json!(chain)),
            ],
        );
        sleep(delay).await;
        return Ok(());
    }

    Err(MistralError::Request {
        stage: stage.to_owned(),
        attempt,
        attempts: MAX_MISTRAL_REQUEST_ATTEMPTS,
        payload_bytes,
        timeout: failure.timeout,
        connect: failure.connect,
        body: failure.body,
        request: failure.request,
        decode: failure.decode,
        chain,
        source,
    })
}

fn mistral_chunking_options(base: ChunkingOptions) -> ChunkingOptions {
    ChunkingOptions {
        max_images_per_request: base
            .max_images_per_request
            .min(MAX_MISTRAL_IMAGES_PER_REQUEST),
        ..base
    }
}

fn validate_frame_size(frame: &VideoFrame) -> Result<(), MistralError> {
    let actual_bytes = frame.bytes.len();
    if actual_bytes > MAX_MISTRAL_IMAGE_BYTES {
        return Err(MistralError::FrameTooLarge {
            video_name: frame.video_name.clone(),
            timestamp_secs: frame.timestamp_secs,
            actual_bytes,
            limit_bytes: MAX_MISTRAL_IMAGE_BYTES,
        });
    }

    Ok(())
}

fn is_retriable_status(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    ) || status.is_server_error()
}

fn retry_delay(retry_after: Option<&str>, fallback_secs: u64) -> Duration {
    let seconds = retry_after
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(fallback_secs)
        .min(MAX_RETRY_AFTER_SECS);

    Duration::from_secs(seconds)
}

fn bounded_api_error_body(body: String) -> String {
    if body.chars().count() <= MAX_MISTRAL_API_ERROR_BODY_CHARS {
        return body;
    }

    let marker_chars = MISTRAL_API_ERROR_BODY_TRUNCATION_MARKER.chars().count();
    let retained_chars = MAX_MISTRAL_API_ERROR_BODY_CHARS.saturating_sub(marker_chars);
    let mut bounded = body.chars().take(retained_chars).collect::<String>();
    bounded.push_str(MISTRAL_API_ERROR_BODY_TRUNCATION_MARKER);
    bounded
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
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use reqwest::StatusCode;
    use serde_json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };

    use crate::{
        analysis::{chunking::ChunkingOptions, request::AnalysisTelemetry},
        media::VideoFrame,
    };

    use super::{
        MAX_MISTRAL_IMAGE_BYTES, MAX_MISTRAL_REQUEST_ATTEMPTS, MAX_RETRY_AFTER_SECS,
        MISTRAL_CHAT_COMPLETIONS_URL, MistralClient, MistralError, RequestFailure,
        config::MistralConfig, is_retriable_status, mistral_chunking_options, retry_delay,
        validate_frame_size,
    };

    fn test_client() -> MistralClient {
        MistralClient {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(1))
                .build()
                .expect("test HTTP client should build"),
            config: MistralConfig::from_values(Some("test-key"), Some("mistral-test"))
                .expect("test configuration should be valid"),
        }
    }

    async fn scripted_http_server(
        responses: Vec<Vec<u8>>,
    ) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let attempts = Arc::new(AtomicUsize::new(0));
        let server_attempts = attempts.clone();
        let handle = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("request should connect");
                let mut request = [0_u8; 8 * 1024];
                let read_bytes = socket
                    .read(&mut request)
                    .await
                    .expect("request should be readable");
                assert!(read_bytes > 0, "request should contain bytes");
                server_attempts.fetch_add(1, Ordering::SeqCst);
                socket
                    .write_all(&response)
                    .await
                    .expect("response should be writable");
                socket.shutdown().await.expect("socket should close");
            }
        });

        (endpoint, attempts, handle)
    }

    fn http_response(
        status: &str,
        body: &str,
        retry_after_secs: Option<u64>,
        declared_content_length: Option<usize>,
    ) -> Vec<u8> {
        let retry_after = retry_after_secs
            .map(|seconds| format!("Retry-After: {seconds}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{retry_after}Connection: close\r\n\r\n{body}",
            declared_content_length.unwrap_or(body.len()),
        )
        .into_bytes()
    }

    fn successful_response(text: &str) -> Vec<u8> {
        http_response(
            "200 OK",
            &json!({
                "choices": [{
                    "message": { "content": text }
                }]
            })
            .to_string(),
            None,
            None,
        )
    }

    #[test]
    fn hosted_endpoint_and_attempt_limit_are_fixed() {
        assert_eq!(
            MISTRAL_CHAT_COMPLETIONS_URL,
            "https://api.mistral.ai/v1/chat/completions"
        );
        assert_eq!(MAX_MISTRAL_REQUEST_ATTEMPTS, 3);
    }

    #[test]
    fn chunking_options_clamp_the_shared_image_limit_to_eight() {
        let base = ChunkingOptions {
            max_images_per_request: 450,
            max_payload_bytes_per_request: 123_456,
            overlap_percent: 12.5,
        };

        let clamped = mistral_chunking_options(base);

        assert_eq!(clamped.max_images_per_request, 8);
        assert_eq!(clamped.max_payload_bytes_per_request, 123_456);
        assert_eq!(clamped.overlap_percent, 12.5);
        assert_eq!(
            mistral_chunking_options(ChunkingOptions {
                max_images_per_request: 4,
                ..base
            })
            .max_images_per_request,
            4
        );
    }

    #[test]
    fn frame_size_limit_accepts_ten_mib_and_rejects_one_byte_more() {
        let mut frame = VideoFrame {
            video_name: "large clip.mp4".to_owned(),
            timestamp_secs: 12.5,
            mime_type: "image/jpeg",
            bytes: vec![0; MAX_MISTRAL_IMAGE_BYTES],
        };

        assert!(validate_frame_size(&frame).is_ok());

        frame.bytes.push(0);
        let error = validate_frame_size(&frame).expect_err("oversized frame should fail");
        let message = error.to_string();

        assert!(message.contains("large clip.mp4"));
        assert!(message.contains("12.500"));
        assert!(message.contains(&(MAX_MISTRAL_IMAGE_BYTES + 1).to_string()));
        assert!(message.contains(&MAX_MISTRAL_IMAGE_BYTES.to_string()));
        assert!(matches!(
            error,
            MistralError::FrameTooLarge {
                video_name,
                timestamp_secs,
                actual_bytes,
                limit_bytes: MAX_MISTRAL_IMAGE_BYTES,
            } if video_name == "large clip.mp4"
                && timestamp_secs == 12.5
                && actual_bytes == MAX_MISTRAL_IMAGE_BYTES + 1
        ));
    }

    #[test]
    fn transport_failure_classification_retries_send_failures() {
        for failure in [
            RequestFailure {
                timeout: true,
                connect: false,
                body: false,
                request: false,
                decode: false,
            },
            RequestFailure {
                timeout: false,
                connect: true,
                body: false,
                request: false,
                decode: false,
            },
            RequestFailure {
                timeout: false,
                connect: false,
                body: true,
                request: false,
                decode: false,
            },
            RequestFailure {
                timeout: false,
                connect: false,
                body: false,
                request: true,
                decode: false,
            },
        ] {
            assert!(failure.is_retriable());
        }

        assert!(
            !RequestFailure {
                timeout: false,
                connect: false,
                body: false,
                request: false,
                decode: true,
            }
            .is_retriable()
        );
    }

    #[test]
    fn status_classification_retries_only_transient_http_failures() {
        assert!(is_retriable_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_retriable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retriable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retriable_status(StatusCode::BAD_GATEWAY));
        assert!(!is_retriable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retriable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retriable_status(StatusCode::NOT_FOUND));
    }

    #[test]
    fn retry_after_integer_seconds_are_clamped_to_a_bounded_delay() {
        assert_eq!(retry_delay(Some("2"), 1), Duration::from_secs(2));
        assert_eq!(
            retry_delay(Some("999"), 1),
            Duration::from_secs(MAX_RETRY_AFTER_SECS)
        );
        assert_eq!(retry_delay(Some("not-seconds"), 2), Duration::from_secs(2));
        assert_eq!(retry_delay(None, 3), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn response_body_transport_failures_are_retried() {
        let truncated = http_response("200 OK", "{\"choices\":", Some(0), Some(128));
        let (endpoint, attempts, server) = scripted_http_server(vec![
            truncated.clone(),
            truncated,
            successful_response("summary"),
        ])
        .await;

        let response = test_client()
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
            .expect("body transport failures should be retried");

        assert_eq!(response.text().as_deref(), Some("summary"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn json_decode_failures_are_not_retried_and_preserve_decode_context() {
        let (endpoint, attempts, server) = scripted_http_server(vec![http_response(
            "200 OK",
            "not valid json",
            Some(0),
            None,
        )])
        .await;

        let error = match test_client()
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
        {
            Ok(_) => panic!("decode failure should fail"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert!(matches!(
            error,
            MistralError::Request {
                attempt: 1,
                timeout: false,
                connect: false,
                body: false,
                request: false,
                decode: true,
                ..
            }
        ));
        assert!(message.contains("decode=true"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn transient_statuses_are_retried_up_to_a_successful_third_attempt() {
        let (endpoint, attempts, server) = scripted_http_server(vec![
            http_response("429 Too Many Requests", "rate limited", Some(0), None),
            http_response("500 Internal Server Error", "try again", Some(0), None),
            successful_response("summary"),
        ])
        .await;

        let response = test_client()
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
            .expect("transient statuses should be retried");

        assert_eq!(response.text().as_deref(), Some("summary"));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn permanent_client_errors_return_immediately_with_status_and_body() {
        let (endpoint, attempts, server) = scripted_http_server(vec![http_response(
            "400 Bad Request",
            "invalid image",
            Some(0),
            None,
        )])
        .await;

        let error = match test_client()
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
        {
            Ok(_) => panic!("permanent status should fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            MistralError::Api { status, body }
                if status == StatusCode::BAD_REQUEST && body == "invalid image"
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn api_error_bodies_are_bounded_without_changing_short_utf8_bodies() {
        const EXPECTED_MAX_ERROR_BODY_CHARS: usize = 4096;
        const EXPECTED_TRUNCATION_MARKER: &str = "\n[truncated]";

        let large_body = "🚨".repeat(5_000);
        let (endpoint, attempts, server) = scripted_http_server(vec![
            http_response("400 Bad Request", "short error", Some(0), None),
            http_response("400 Bad Request", &large_body, Some(0), None),
        ])
        .await;
        let client = test_client();

        let short_error = match client
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
        {
            Ok(_) => panic!("short API error should fail"),
            Err(error) => error,
        };
        let MistralError::Api {
            body: short_body, ..
        } = short_error
        else {
            panic!("short API error should preserve status context");
        };

        let large_error = match client
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
        {
            Ok(_) => panic!("large API error should fail"),
            Err(error) => error,
        };
        let MistralError::Api {
            body: bounded_body, ..
        } = large_error
        else {
            panic!("large API error should preserve status context");
        };

        assert_eq!(short_body, "short error");
        assert_eq!(bounded_body.chars().count(), EXPECTED_MAX_ERROR_BODY_CHARS);
        assert!(bounded_body.starts_with('🚨'));
        assert!(bounded_body.ends_with(EXPECTED_TRUNCATION_MARKER));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        server.await.expect("test server should finish");
    }

    #[tokio::test]
    async fn exhausted_status_retries_preserve_the_final_status_and_body() {
        let (endpoint, attempts, server) = scripted_http_server(vec![
            http_response("503 Service Unavailable", "first", Some(0), None),
            http_response("503 Service Unavailable", "second", Some(0), None),
            http_response("503 Service Unavailable", "final", Some(0), None),
        ])
        .await;

        let error = match test_client()
            .send_chat_completion_request_to(
                &AnalysisTelemetry::default(),
                "summary",
                2,
                &json!({}),
                &endpoint,
            )
            .await
        {
            Ok(_) => panic!("three transient statuses should exhaust retries"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            MistralError::Api { status, body }
                if status == StatusCode::SERVICE_UNAVAILABLE && body == "final"
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        server.await.expect("test server should finish");
    }
}
