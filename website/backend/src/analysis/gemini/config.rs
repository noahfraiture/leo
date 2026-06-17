use std::{env, time::Duration};

use super::GeminiError;

const DEFAULT_MODEL: &str = "gemini-3-flash-preview";
pub(super) const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(300);
pub(super) const DEFAULT_UPLOAD_CHUNK_SIZE_BUCKETS_BYTES: [usize; 4] = [
    8 * 1024 * 1024,
    16 * 1024 * 1024,
    32 * 1024 * 1024,
    64 * 1024 * 1024,
];
const DEFAULT_FILE_PROCESSING_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) struct GeminiConfig {
    pub api_key: String,
    pub model: String,
    pub file_processing_timeout: Duration,
    pub upload_chunk_size_buckets: Vec<usize>,
}

impl GeminiConfig {
    pub(super) fn from_env() -> Result<Self, GeminiError> {
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
        let upload_chunk_size = env::var("GEMINI_UPLOAD_CHUNK_SIZE_BYTES").ok();
        let upload_chunk_size_buckets = upload_chunk_size_buckets_from_values(
            upload_chunk_size.as_deref(),
            env::var("GEMINI_UPLOAD_CHUNK_SIZE_BUCKETS_BYTES")
                .ok()
                .as_deref(),
        );

        Ok(Self {
            api_key,
            model,
            file_processing_timeout,
            upload_chunk_size_buckets,
        })
    }
}

fn upload_chunk_size_buckets_from_values(
    fixed_chunk_size: Option<&str>,
    bucket_list: Option<&str>,
) -> Vec<usize> {
    if let Some(chunk_size) = fixed_chunk_size
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
    {
        return vec![chunk_size];
    }

    let buckets = bucket_list
        .unwrap_or_default()
        .split(',')
        .filter_map(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();

    if buckets.is_empty() {
        DEFAULT_UPLOAD_CHUNK_SIZE_BUCKETS_BYTES.to_vec()
    } else {
        buckets
    }
}
