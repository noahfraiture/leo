//! Gemini resumable upload chunking, retry, and telemetry helpers.

use reqwest::header::HeaderMap;
#[cfg(test)]
use serde_json::Value;
use serde_json::json;

use crate::analysis::{
    gemini::config::DEFAULT_UPLOAD_CHUNK_SIZE_BUCKETS_BYTES, request::AnalysisTelemetry,
};

use super::GeminiError;

const DEFAULT_UPLOAD_CHUNK_GRANULARITY_BYTES: usize = 256 * 1024;

pub(super) struct VideoInput {
    pub name: String,
    pub bytes: Vec<u8>,
}

pub(super) struct UploadSession {
    pub url: String,
    pub chunk_granularity: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct UploadChunk {
    pub offset: usize,
    pub end: usize,
    pub command: UploadCommand,
}

impl UploadChunk {
    pub(super) fn len(self) -> usize {
        self.end - self.offset
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UploadCommand {
    Upload,
    UploadAndFinalize,
}

impl UploadCommand {
    pub(super) fn as_header(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::UploadAndFinalize => "upload, finalize",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UploadRetryDecision {
    RetryFromOffset(usize),
    RestartSession,
}

#[derive(Debug)]
pub(super) enum UploadChunksError {
    Gemini(GeminiError),
    FinalizedWithoutResponse(GeminiError),
}

impl From<GeminiError> for UploadChunksError {
    fn from(error: GeminiError) -> Self {
        Self::Gemini(error)
    }
}

#[derive(Default)]
pub(super) struct UploadStats {
    pub send_attempts: usize,
    pub retry_count: usize,
    pub timeout_retries: usize,
    pub connect_retries: usize,
    pub body_retries: usize,
    pub api_retries: usize,
    pub offset_queries: usize,
}

impl UploadStats {
    pub(super) fn record_send_attempt(&mut self) {
        self.send_attempts += 1;
    }

    pub(super) fn record_retry(&mut self, error: &GeminiError) {
        match error {
            GeminiError::UploadRequest {
                timeout,
                connect,
                body,
                ..
            } => self.record_upload_retry(*timeout, *connect, *body),
            GeminiError::Api { .. } => self.record_api_retry(),
            _ => self.retry_count += 1,
        }
    }

    pub(super) fn record_upload_retry(&mut self, timeout: bool, connect: bool, body: bool) {
        self.retry_count += 1;
        if timeout {
            self.timeout_retries += 1;
        }
        if connect {
            self.connect_retries += 1;
        }
        if body {
            self.body_retries += 1;
        }
    }

    pub(super) fn record_api_retry(&mut self) {
        self.retry_count += 1;
        self.api_retries += 1;
    }

    pub(super) fn record_offset_query(&mut self) {
        self.offset_queries += 1;
    }

    #[cfg(test)]
    pub(super) fn summary_fields(
        &self,
        chunk_size_bytes: usize,
        logical_chunks: usize,
        video_size_bytes: usize,
        chunk_granularity_bytes: usize,
        duration_ms: i64,
    ) -> Value {
        json!({
            "chunk_size_bytes": chunk_size_bytes,
            "logical_chunks": logical_chunks,
            "video_size_bytes": video_size_bytes,
            "chunk_granularity_bytes": chunk_granularity_bytes,
            "send_attempts": self.send_attempts,
            "retry_count": self.retry_count,
            "timeout_retries": self.timeout_retries,
            "connect_retries": self.connect_retries,
            "body_retries": self.body_retries,
            "api_retries": self.api_retries,
            "offset_queries": self.offset_queries,
            "duration_ms": duration_ms,
        })
    }
}

pub(super) fn log_upload_completed(
    telemetry: &AnalysisTelemetry,
    video: &VideoInput,
    stats: &UploadStats,
    total_bytes: usize,
    total_chunks: usize,
    upload_chunk_size: usize,
    chunk_granularity: usize,
    duration_ms: i64,
) {
    telemetry.log(
        "info",
        "gemini",
        "upload_completed",
        [
            ("video_name", json!(video.name)),
            ("bytes", json!(total_bytes)),
            ("chunks", json!(total_chunks)),
            ("chunk_size", json!(upload_chunk_size)),
            ("granularity", json!(chunk_granularity)),
            ("send_attempts", json!(stats.send_attempts)),
            ("retry_count", json!(stats.retry_count)),
            ("timeout_retries", json!(stats.timeout_retries)),
            ("connect_retries", json!(stats.connect_retries)),
            ("body_retries", json!(stats.body_retries)),
            ("api_retries", json!(stats.api_retries)),
            ("offset_queries", json!(stats.offset_queries)),
            ("duration_ms", json!(duration_ms)),
        ],
    );
}

pub(super) fn upload_session_from_headers(
    headers: &HeaderMap,
) -> Result<UploadSession, GeminiError> {
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

pub(super) fn upload_chunks(
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

pub(super) fn select_upload_chunk_size(telemetry: &AnalysisTelemetry, buckets: &[usize]) -> usize {
    let buckets = if buckets.is_empty() {
        DEFAULT_UPLOAD_CHUNK_SIZE_BUCKETS_BYTES.as_slice()
    } else {
        buckets
    };
    if telemetry.is_canary {
        return buckets[0];
    }

    let Some(analysis_id) = telemetry
        .analysis_id
        .as_deref()
        .filter(|analysis_id| !analysis_id.is_empty())
    else {
        return buckets[0];
    };

    buckets[stable_bucket_index(analysis_id, buckets.len())]
}

fn stable_bucket_index(value: &str, bucket_count: usize) -> usize {
    if bucket_count == 0 {
        return 0;
    }

    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    (hash as usize) % bucket_count
}

pub(super) fn next_upload_chunk(
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

pub(super) fn upload_retry_decision(
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

pub(super) fn upload_offset_from_headers(headers: &HeaderMap) -> Result<usize, GeminiError> {
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

pub(super) fn video_mime_type(name: &str) -> &'static str {
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

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderMap;
    use serde_json::json;

    use crate::analysis::{gemini::upload::*, request::AnalysisTelemetry};

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
    fn upload_chunk_size_bucket_is_stable_for_analysis_id() {
        let buckets = vec![8, 16, 32, 64];
        let telemetry = AnalysisTelemetry::new("analysis-123", "gemini");

        let selected = select_upload_chunk_size(&telemetry, &buckets);

        assert!(buckets.contains(&selected));
        assert_eq!(selected, select_upload_chunk_size(&telemetry, &buckets));
        assert_eq!(
            selected,
            select_upload_chunk_size(&AnalysisTelemetry::new("analysis-123", "gemini"), &buckets)
        );
        assert_eq!(
            select_upload_chunk_size(&AnalysisTelemetry::default(), &buckets),
            8
        );
    }

    #[test]
    fn canary_upload_chunk_size_uses_first_bucket() {
        let buckets = vec![8, 16, 32, 64];
        let telemetry = AnalysisTelemetry::new("analysis-123", "gemini").with_canary(true);

        assert_eq!(select_upload_chunk_size(&telemetry, &buckets), 8);
    }

    #[test]
    fn upload_stats_count_retries_and_summary_fields() {
        let mut stats = UploadStats::default();

        stats.record_send_attempt();
        stats.record_upload_retry(true, false, false);
        stats.record_offset_query();
        stats.record_send_attempt();
        stats.record_api_retry();

        let fields = stats.summary_fields(32, 4, 20, 12, 1_234);

        assert_eq!(fields["chunk_size_bytes"], json!(32));
        assert_eq!(fields["logical_chunks"], json!(4));
        assert_eq!(fields["send_attempts"], json!(2));
        assert_eq!(fields["retry_count"], json!(2));
        assert_eq!(fields["timeout_retries"], json!(1));
        assert_eq!(fields["api_retries"], json!(1));
        assert_eq!(fields["offset_queries"], json!(1));
        assert_eq!(fields["duration_ms"], json!(1_234));
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
}
