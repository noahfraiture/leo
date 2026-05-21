use std::{env, str::FromStr, time::Duration};

use thiserror::Error;

use crate::analysis::provider::AnalysisProvider;

pub const CANARY_VIDEO_NAME: &str = "leo-analysis-canary.mp4";
pub const DEFAULT_CANARY_PROMPT: &str =
    "Health check: confirm that this short synthetic video can be processed.";

#[derive(Clone, Debug, PartialEq)]
pub struct CanaryConfig {
    pub enabled: bool,
    pub providers: Vec<String>,
    pub interval_secs: Option<u64>,
    pub prompt: String,
}

#[derive(Debug, Error)]
pub enum CanaryConfigError {
    #[error("canary interval must be a positive integer number of seconds")]
    InvalidInterval,
    #[error("canary provider list did not include any supported providers")]
    EmptyProviders,
}

impl CanaryConfig {
    pub fn from_env() -> Result<Self, CanaryConfigError> {
        Self::from_values(
            env::var("ANALYSIS_CANARY_ENABLED").unwrap_or_default(),
            env::var("ANALYSIS_CANARY_PROVIDERS").unwrap_or_else(|_| "openai,gemini".to_owned()),
            env::var("ANALYSIS_CANARY_INTERVAL_SECS").unwrap_or_default(),
            env::var("ANALYSIS_CANARY_PROMPT").unwrap_or_else(|_| DEFAULT_CANARY_PROMPT.to_owned()),
        )
    }

    pub fn from_values(
        enabled: impl AsRef<str>,
        providers: impl AsRef<str>,
        interval_secs: impl AsRef<str>,
        prompt: impl Into<String>,
    ) -> Result<Self, CanaryConfigError> {
        let enabled = matches!(
            enabled.as_ref().trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
        let providers = providers
            .as_ref()
            .split(',')
            .map(str::trim)
            .filter(|provider| AnalysisProvider::from_str(provider).is_ok())
            .map(|provider| provider.to_ascii_lowercase())
            .collect::<Vec<_>>();
        if providers.is_empty() {
            return Err(CanaryConfigError::EmptyProviders);
        }

        let interval_secs = match interval_secs.as_ref().trim() {
            "" | "0" => None,
            value => Some(
                value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or(CanaryConfigError::InvalidInterval)?,
            ),
        };

        Ok(Self {
            enabled,
            providers,
            interval_secs,
            prompt: prompt.into(),
        })
    }

    pub fn interval(&self) -> Option<Duration> {
        self.interval_secs.map(Duration::from_secs)
    }
}
