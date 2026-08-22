/// Failure while planning, resuming, or executing local video analysis.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Session(#[from] crate::session::Error),

    #[error(transparent)]
    Recording(#[from] crate::recording::Error),

    #[error(transparent)]
    Agent(#[from] super::agent::Error),

    #[error(transparent)]
    Video(#[from] super::video::Error),

    #[error("analysis I/O failed")]
    Io(#[from] std::io::Error),

    #[error("analysis checkpoint is not valid JSON")]
    Json(#[from] serde_json::Error),

    #[error("analysis plan contains no analyzable frame sets")]
    NoAnalyzableFrames,

    #[error("analysis checklist must not be empty")]
    EmptyChecklist,

    #[error("analysis session directory must be a direct directory")]
    InvalidSessionDirectory,

    #[error("analysis requires a direct zero-byte recording completion marker")]
    InvalidCompletionMarker,

    #[error("analysis frame references unknown camera {camera_id}")]
    MissingCamera { camera_id: u32 },

    #[error("frame extraction task failed")]
    ExtractionTask(#[from] tokio::task::JoinError),

    #[error("recording segment discovery task failed")]
    SegmentDiscoveryTask(#[source] tokio::task::JoinError),

    #[error("analysis checkpoint must be a direct regular file")]
    InvalidCheckpointFile,

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

    #[error("analysis checkpoint model must not be blank")]
    BlankCheckpointModel,

    #[error("analysis checkpoint endpoint ID must not be blank")]
    BlankCheckpointEndpointId,

    #[error("analysis checkpoint checklist is empty")]
    EmptyCheckpointChecklist,

    #[error("analysis checkpoint plan fingerprint is empty")]
    EmptyCheckpointPlanFingerprint,

    #[error("analysis checkpoint checklist does not match the requested analysis")]
    CheckpointChecklist,

    #[error("analysis checkpoint model does not match the configured analysis")]
    CheckpointModel,

    #[error("analysis checkpoint endpoint ID does not match the configured analysis")]
    CheckpointEndpointId,

    #[error("analysis checkpoint plan fingerprint does not match the rebuilt plan")]
    CheckpointPlanFingerprint,

    #[error(
        "analysis checkpoint batch count {actual} does not match rebuilt plan batch count {expected}"
    )]
    CheckpointBatchCount { expected: usize, actual: usize },

    #[error("analysis checkpoint warnings do not match the rebuilt plan")]
    CheckpointWarnings,

    #[error(
        "analysis checkpoint contains {completed} completed batches, but the rebuilt plan has {total}"
    )]
    ProgressExceedsPlan { completed: usize, total: usize },

    #[error("analysis plan {field} cannot be represented in the checkpoint fingerprint")]
    PlanValueOverflow { field: &'static str },

    #[error("all {total} analysis batches are already complete")]
    AnalysisComplete { total: usize },
}
