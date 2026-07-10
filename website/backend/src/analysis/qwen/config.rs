//! Qwen provider configuration loaded from environment variables.

use std::{env, time::Duration};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:1234";
const DEFAULT_MODEL: &str = "qwen/qwen3.6-35b-a3b";
const DEFAULT_IMAGE_DETAIL: &str = "low";
pub(super) const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) struct QwenConfig {
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) model: String,
    pub(super) image_detail: String,
}

impl QwenConfig {
    pub(super) fn from_env() -> Self {
        Self::from_values(
            env::var("QWEN_BASE_URL").ok().as_deref(),
            env::var("QWEN_MODEL").ok().as_deref(),
            env::var("QWEN_API_KEY").ok().as_deref(),
            env::var("QWEN_IMAGE_DETAIL").ok().as_deref(),
        )
    }

    pub(super) fn from_values(
        base_url: Option<&str>,
        model: Option<&str>,
        api_key: Option<&str>,
        image_detail: Option<&str>,
    ) -> Self {
        let base_url = clean_value(base_url)
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();

        Self {
            base_url,
            api_key: clean_value(api_key),
            model: clean_value(model).unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            image_detail: clean_value(image_detail)
                .unwrap_or_else(|| DEFAULT_IMAGE_DETAIL.to_owned()),
        }
    }

    pub(super) fn responses_url(&self) -> String {
        format!("{}/v1/responses", self.base_url)
    }
}

fn clean_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::QwenConfig;

    #[test]
    fn default_config_targets_local_lm_studio_responses_api() {
        let config = QwenConfig::from_values(None, None, None, None);

        assert_eq!(config.base_url, "http://127.0.0.1:1234");
        assert_eq!(config.responses_url(), "http://127.0.0.1:1234/v1/responses");
        assert_eq!(config.model, "qwen/qwen3.6-35b-a3b");
        assert_eq!(config.api_key, None);
        assert_eq!(config.image_detail, "low");
    }

    #[test]
    fn config_trims_overrides_and_removes_trailing_base_url_slash() {
        let config = QwenConfig::from_values(
            Some(" http://localhost:4321/ "),
            Some(" custom/qwen "),
            Some(" local-key "),
            Some(" high "),
        );

        assert_eq!(config.base_url, "http://localhost:4321");
        assert_eq!(config.responses_url(), "http://localhost:4321/v1/responses");
        assert_eq!(config.model, "custom/qwen");
        assert_eq!(config.api_key.as_deref(), Some("local-key"));
        assert_eq!(config.image_detail, "high");
    }
}
