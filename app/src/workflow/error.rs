use std::{io, path::PathBuf};

/// Workflow transition or local session-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("a session can only start while the workflow is idle")]
    StartUnavailable,
    #[error("no cameras are configured; add a camera in Settings before starting a session")]
    NoCamerasConfigured,
    #[error("a session can only stop while the workflow is active")]
    StopUnavailable,
    #[error("analysis is already running")]
    AnalysisRunning,
    #[error("select a completed session before starting analysis")]
    AnalysisSessionNotSelected,
    #[error("the selected session is no longer complete")]
    AnalysisSessionIncomplete,
    #[error("analysis can only start while the recording workflow is idle")]
    AnalysisRequiresIdleSession,
    #[error("the selected session has an invalid analysis checkpoint")]
    InvalidAnalysisCheckpoint,
    #[error("model configuration is unavailable")]
    ModelConfigurationUnavailable,
    #[error("the analysis checklist cannot be empty")]
    EmptyChecklist,
    #[error("camera {camera_id} is not configured")]
    UnknownCamera { camera_id: u32 },
    #[error("camera {camera_id} sampling interval is outside the millisecond range")]
    InvalidSamplingInterval { camera_id: u32 },
    #[error("failed to create session directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("session metadata operation failed")]
    Session(#[from] backend::session::Error),
}
