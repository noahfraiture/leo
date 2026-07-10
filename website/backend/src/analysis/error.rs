//! Error type that normalizes provider and frame extraction failures.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalysisError {
    #[error(transparent)]
    Provider(#[from] crate::analysis::provider::ProviderParseError),
    #[error(transparent)]
    Gemini(#[from] crate::analysis::gemini::GeminiError),
    #[error(transparent)]
    Gemma(#[from] crate::analysis::gemma::GemmaError),
    #[error(transparent)]
    Mistral(#[from] crate::analysis::mistral::MistralError),
    #[error(transparent)]
    OpenAi(#[from] crate::analysis::openai::OpenAiError),
    #[error(transparent)]
    Qwen(#[from] crate::analysis::qwen::QwenError),
    #[error(transparent)]
    FrameExtraction(#[from] crate::media::frames::FrameExtractionError),
}
