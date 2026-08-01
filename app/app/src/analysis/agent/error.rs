use rig_core::client::ProviderClientError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("client configuration failed")]
    ProviderError(#[from] ProviderClientError),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
