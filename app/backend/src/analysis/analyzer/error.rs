#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error(transparent)]
    Agent(#[from] crate::analysis::agent::Error),

    #[error(transparent)]
    Recording(#[from] crate::recording::Error),

    #[error(transparent)]
    Video(#[from] crate::analysis::video::Error),

    #[error("analysis I/O failed")]
    Io(#[from] std::io::Error),

    #[error("analysis checkpoint is not valid JSON")]
    Json(#[from] serde_json::Error),

    #[error("analysis session end cannot be represented as a UTC millisecond timestamp")]
    SessionEndUtcOverflow,

    #[error("analysis plan contains no frame sets")]
    EmptyPlan,

    #[error("Synology catalogue contains duplicate recording ID {recording_id}")]
    DuplicateRecordingId { recording_id: u64 },

    #[error("analysis frame references missing recording {recording_id}")]
    MissingVideo { recording_id: u64 },

    #[error("recording {recording_id} has invalid planned bounds")]
    InvalidVideoBounds { recording_id: u64 },

    #[error("recording {recording_id} has invalid batch window {start:?}..{end:?}")]
    InvalidBatchWindow {
        recording_id: u64,
        start: std::time::Duration,
        end: std::time::Duration,
    },

    #[error(
        "recording {recording_id} frame offset {offset:?} precedes download start {download_start:?}"
    )]
    InvalidLocalOffset {
        recording_id: u64,
        offset: std::time::Duration,
        download_start: std::time::Duration,
    },

    #[error("analysis frame references unknown camera {camera_id}")]
    MissingCamera { camera_id: u32 },

    #[error("frame extraction task failed")]
    ExtractionTask(#[from] tokio::task::JoinError),

    #[error(
        "analysis checkpoint schema version {actual} does not match expected version {expected}"
    )]
    CheckpointSchema { expected: u8, actual: u8 },

    #[error(
        "analysis checkpoint session ID {actual} does not match expected session ID {expected}"
    )]
    CheckpointSession {
        expected: uuid::Uuid,
        actual: uuid::Uuid,
    },

    #[error(
        "analysis checkpoint batch count {actual} does not match rebuilt plan batch count {expected}"
    )]
    CheckpointBatchCount { expected: usize, actual: usize },

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

pub(super) type Result<T> = std::result::Result<T, Error>;
