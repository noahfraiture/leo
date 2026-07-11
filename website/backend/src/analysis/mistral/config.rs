//! Hosted Mistral configuration loaded from environment variables.

use std::{env, time::Duration};

use super::MistralError;

const DEFAULT_MODEL: &str = "mistral-medium-latest";
pub(super) const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) struct MistralConfig {
    pub(super) api_key: String,
    pub(super) model: String,
}

impl MistralConfig {
    pub(super) fn from_env() -> Result<Self, MistralError> {
        Self::from_values(
            env::var("MISTRAL_API_KEY").ok().as_deref(),
            env::var("MISTRAL_MODEL").ok().as_deref(),
        )
    }

    pub(super) fn from_values(
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> Result<Self, MistralError> {
        let api_key = clean_value(api_key).ok_or(MistralError::MissingApiKey)?;
        let model = clean_value(model).unwrap_or_else(|| DEFAULT_MODEL.to_owned());

        Ok(Self { api_key, model })
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
    use crate::analysis::mistral::MistralError;

    use super::MistralConfig;

    #[test]
    fn config_requires_an_api_key_and_uses_the_default_model() {
        let config = MistralConfig::from_values(Some(" test-key "), None)
            .expect("non-empty API key should be accepted");

        assert_eq!(config.api_key, "test-key");
        assert_eq!(config.model, "mistral-medium-latest");
        assert!(matches!(
            MistralConfig::from_values(None, None),
            Err(MistralError::MissingApiKey)
        ));
        assert!(matches!(
            MistralConfig::from_values(Some(" \t "), None),
            Err(MistralError::MissingApiKey)
        ));
    }

    #[test]
    fn config_uses_a_non_empty_model_override() {
        let config = MistralConfig::from_values(Some("test-key"), Some(" custom-model "))
            .expect("configuration should be valid");
        let blank_model = MistralConfig::from_values(Some("test-key"), Some("  "))
            .expect("configuration should be valid");

        assert_eq!(config.model, "custom-model");
        assert_eq!(blank_model.model, "mistral-medium-latest");
    }
}
