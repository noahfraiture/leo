/// Configuration and operational failures. These are never candidate assessments.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("model configuration field {0} must not be blank")]
    BlankField(&'static str),
    #[error(
        "provider endpoint must be an HTTP(S) base URL without credentials, query, or fragment"
    )]
    InvalidEndpoint,
    #[error("selected model profile is missing or has a duplicate identifier")]
    UnknownModel,
    #[error("selected provider connection is missing or has a duplicate identifier")]
    UnknownProvider,
    #[error("model profile belongs to a different provider connection")]
    ProviderMismatch,
    #[error("a model set must select at least one reader and cannot repeat a reader profile")]
    InvalidReaders,
    #[error("model image and optional output-token limits must be positive")]
    InvalidModelLimits,
    #[error("request exceeds the selected model's image or output-token limit")]
    RequestExceedsModelLimits,
    #[error("concurrency and timeout must be positive and the memory budget cannot exceed 48 GiB")]
    InvalidExecutionPolicy,
    #[error("one selected model exceeds the configured memory budget")]
    ModelExceedsMemoryBudget,
    #[error("job identifiers must be nonblank and unique")]
    InvalidJobIdentity,
    #[error("model images must have unique nonblank identifiers and nonempty bytes")]
    InvalidImage,
    #[error(
        "model request requires instructions, content, a positive timeout, and positive token limits"
    )]
    InvalidRequest,
    #[error("could not create the configured model client")]
    ClientConfiguration,
    #[error("model request timed out")]
    Timeout,
    #[error("model request failed; check the configured endpoint, transport, and model")]
    RequestFailed,
    #[error(
        "model provider rejected the request (HTTP {status}); check the connection and model settings"
    )]
    ProviderStatus { status: u16 },
    #[error("model response contained no text")]
    MissingText,
    #[error(
        "model response did not match the expected JSON structure (line {line}, column {column})"
    )]
    InvalidResponse { line: usize, column: usize },
}

/// Result returned by model configuration, completion, and scheduling operations.
pub type Result<T> = std::result::Result<T, Error>;

impl From<rig_core::completion::CompletionError> for Error {
    fn from(error: rig_core::completion::CompletionError) -> Self {
        // Provider messages and error sources can echo request payloads or credentials.
        match error.provider_response_status() {
            Some(status) => Self::ProviderStatus {
                status: status.as_u16(),
            },
            None => Self::RequestFailed,
        }
    }
}
