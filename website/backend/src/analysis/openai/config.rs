use std::{env, time::Duration};

use super::OpenAiError;

const DEFAULT_MODEL: &str = "gpt-5.5";
const DEFAULT_IMAGE_DETAIL: &str = "low";
pub(super) const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(300);

pub struct OpenAiConfig {
    pub(super) api_key: String,
    pub(super) model: String,
    pub(super) image_detail: String,
}

impl OpenAiConfig {
    pub(super) fn from_env() -> Result<Self, OpenAiError> {
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
