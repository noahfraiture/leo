pub mod chunking;
pub mod error;
pub mod gemini;
pub mod job;
pub mod openai;
pub mod prompts;
pub mod provider;
pub mod request;

use std::str::FromStr;

use error::AnalysisError;
use provider::AnalysisProvider;
use request::{AnalysisRequest, AnalysisSettings, AnalysisTelemetry};

use crate::media::AnalysisVideo;

pub async fn analyze_videos(
    provider: AnalysisProvider,
    videos: Vec<AnalysisVideo>,
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
    videos: Vec<AnalysisVideo>,
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
