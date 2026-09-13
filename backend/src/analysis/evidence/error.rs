use thiserror::Error;

/// Evidence preparation failures are operational errors, never candidate judgments.
#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("Cannot read recording evidence: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cannot sample recording evidence: {0}")]
    Sampling(#[from] super::super::video::Error),
    #[error("Invalid evidence: {0}")]
    Invalid(String),
    #[error("Cannot serialize evidence: {0}")]
    Json(#[from] serde_json::Error),
}
