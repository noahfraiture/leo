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
use request::AnalysisRequest;

use crate::db;

pub async fn analyze_videos(
    provider: AnalysisProvider,
    videos: Vec<db::video::VideoAsset>,
    prompt: impl Into<String>,
) -> Result<String, AnalysisError> {
    let request = AnalysisRequest {
        videos,
        prompt: prompt.into(),
    };

    match provider {
        AnalysisProvider::Gemini => Ok(gemini::GeminiClient::from_env()?.analyze(request).await?),
        AnalysisProvider::OpenAi => Ok(openai::OpenAiClient::from_env()?.analyze(request).await?),
    }
}

pub fn provider_from_value(value: &str) -> Result<AnalysisProvider, AnalysisError> {
    Ok(AnalysisProvider::from_str(value)?)
}
