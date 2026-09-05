//! Errors returned by operator-state transitions and local session storage.

use std::{io, path::PathBuf};

/// Operator-state transition or local session-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("a session can only start while no session is active")]
    StartUnavailable,
    #[error("no cameras are configured; add a camera in Settings before starting a session")]
    NoCamerasConfigured,
    #[error("a session can only stop while a session is active")]
    StopUnavailable,
    #[error("analysis is already running")]
    AnalysisRunning,
    #[error("select a completed session before starting analysis")]
    AnalysisSessionNotSelected,
    #[error("the selected session is no longer complete")]
    AnalysisSessionIncomplete,
    #[error("analysis can only start while no session is active")]
    AnalysisRequiresIdleSession,
    #[error("the selected session has an invalid analysis checkpoint")]
    InvalidAnalysisCheckpoint,
    #[error("model configuration is unavailable")]
    ModelConfigurationUnavailable,
    #[error("the analysis checklist cannot be empty")]
    EmptyChecklist,
    #[error("camera {camera_id} is not configured")]
    UnknownCamera { camera_id: u32 },
    #[error("failed to create session directory {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("monitoring metadata is unavailable; recording continues")]
    MetadataUnavailable,
    #[error("could not discard the previous analysis checkpoint")]
    ResetAnalysis(#[source] io::Error),
    #[error("session metadata operation failed")]
    Session(#[from] backend::session::Error),
}
