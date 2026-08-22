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
}

pub(super) type Result<T> = std::result::Result<T, Error>;
