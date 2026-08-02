#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Agent(#[from] crate::analysis::agent::Error),

    #[error("analysis checkpoint I/O failed")]
    Io(#[from] std::io::Error),

    #[error("analysis checkpoint is not valid JSON")]
    Json(#[from] serde_json::Error),

    #[error(
        "analysis checkpoint batch indices are not contiguous: expected {expected}, found {actual}"
    )]
    NonContiguousBatch { expected: usize, actual: usize },

    #[error(
        "analysis checkpoint contains {completed} completed batches, but the rebuilt plan has {total}"
    )]
    ProgressExceedsPlan { completed: usize, total: usize },

    #[error("all {total} analysis batches are already complete")]
    AnalysisComplete { total: usize },
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
