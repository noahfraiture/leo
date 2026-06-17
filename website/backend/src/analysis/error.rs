//! Error type that normalizes provider and frame extraction failures.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Provider(#[from] crate::analysis::provider::ProviderParseError),
    #[error(transparent)]
    Gemini(#[from] crate::analysis::gemini::GeminiError),
    #[error(transparent)]
    OpenAi(#[from] crate::analysis::openai::OpenAiError),
    #[error(transparent)]
    FrameExtraction(#[from] crate::media::frames::FrameExtractionError),
}
