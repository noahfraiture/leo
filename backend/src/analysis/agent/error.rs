use rig_core::client::ProviderClientError;
use rig_core::completion::CompletionError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("analysis configuration {0} must not be blank")]
    BlankConfiguration(&'static str),

    #[error("client configuration failed")]
    Provider(#[from] ProviderClientError),

    #[error("analysis request failed")]
    Completion(#[from] CompletionError),

    #[error("analysis response was not valid structured JSON")]
    ResponseJson(#[from] serde_json::Error),

    #[error("analysis response contained no text")]
    MissingTextResponse,

    #[cfg(feature = "test-support")]
    #[error("evaluation metering failed: {0}")]
    Metering(#[from] std::io::Error),

    #[cfg(feature = "test-support")]
    #[error("local evaluation requires a loopback endpoint")]
    LocalEndpoint,
}

pub(super) type Result<T> = std::result::Result<T, Error>;
