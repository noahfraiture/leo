#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("failed to serialize a MediaMTX configuration value")]
    SerializeConfig(#[from] serde_json::Error),
    #[error("failed to create temporary MediaMTX configuration")]
    CreateConfig(#[source] std::io::Error),
    #[error("failed to write temporary MediaMTX configuration")]
    WriteConfig(#[source] std::io::Error),
}
