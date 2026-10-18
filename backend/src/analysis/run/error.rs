/// Operational run failures, kept separate from any candidate judgment.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("analysis storage failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("analysis file is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("analysis worker failed")]
    Worker(#[from] tokio::task::JoinError),
    #[error("invalid analysis run: {0}")]
    Invalid(String),
    #[error("This run is busy. Wait for the current operation to finish.")]
    Busy,
    #[error(transparent)]
    Model(#[from] crate::model::Error),
    #[error(transparent)]
    Exercise(#[from] crate::exercise::Error),
    #[error(transparent)]
    Evidence(#[from] crate::analysis::EvidenceError),
}

/// Result from persisted run and coordination operations.
pub type RunResult<T> = std::result::Result<T, RunError>;
