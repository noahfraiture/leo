use rig_core::client::ProviderClientError;
use rig_core::completion::CompletionError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("client configuration failed")]
    ProviderError(#[from] ProviderClientError),

    #[error("analysis request failed")]
    Completion(#[from] CompletionError),

    #[error("analysis response was not valid structured JSON")]
    ResponseJson(#[from] serde_json::Error),

    #[error("analysis response contained no text")]
    MissingTextResponse,
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
