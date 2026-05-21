pub mod canary;
pub mod chunking;
pub mod error;
pub mod frames;
pub mod gemini;
pub mod openai;
pub mod provider;
pub mod request;

use std::str::FromStr;

use error::AnalysisError;
use provider::AnalysisProvider;
use request::{AnalysisRequest, AnalysisSettings, AnalysisTelemetry};

use crate::db;

pub async fn analyze_videos(
    provider: AnalysisProvider,
    videos: Vec<db::video::VideoAsset>,
    prompt: impl Into<String>,
    settings: AnalysisSettings,
) -> Result<String, AnalysisError> {
    analyze_videos_with_telemetry(
        provider,
        videos,
        prompt,
        settings,
        AnalysisTelemetry::default(),
    )
    .await
}

pub async fn analyze_videos_with_telemetry(
    provider: AnalysisProvider,
    videos: Vec<db::video::VideoAsset>,
    prompt: impl Into<String>,
    settings: AnalysisSettings,
    telemetry: AnalysisTelemetry,
) -> Result<String, AnalysisError> {
    let request = AnalysisRequest {
        videos,
        prompt: prompt.into(),
        settings,
        telemetry,
    };

    match provider {
        AnalysisProvider::Gemini => Ok(gemini::GeminiClient::from_env()?.analyze(request).await?),
        AnalysisProvider::OpenAi => Ok(openai::OpenAiClient::from_env()?.analyze(request).await?),
    }
}

pub fn provider_from_value(value: &str) -> Result<AnalysisProvider, AnalysisError> {
    Ok(AnalysisProvider::from_str(value)?)
}

#[cfg(test)]
mod tests {
    use crate::analysis::canary::CanaryConfig;

    #[test]
    fn canary_config_parses_enabled_provider_list_and_interval() {
        let config =
            CanaryConfig::from_values("true", "openai, gemini", "3600", "Run the health check")
                .expect("config should parse");

        assert!(config.enabled);
        assert_eq!(config.providers, vec!["openai", "gemini"]);
        assert_eq!(config.interval_secs, Some(3600));
        assert_eq!(config.prompt, "Run the health check");
    }
}
