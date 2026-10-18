//! Errors returned by operator-state transitions and local session storage.

use std::{io, path::PathBuf};

/// Operator-state transition or local session-storage failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("a session can only start while no session is active")]
    StartUnavailable,
    #[error("the recorder runtime is unavailable; restart Leo before starting another session")]
    RecorderUnavailable,
    #[error("no cameras are configured; add a camera in Settings before starting a session")]
    NoCamerasConfigured,
    #[error("a session can only stop while a session is active")]
    StopUnavailable,
    #[error("analysis is already running")]
    AnalysisRunning,
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
    #[error("Could not open the folder.")]
    OpenFolder(#[source] io::Error),
    #[error("session metadata operation failed")]
    Session(#[from] backend::session::Error),
}

/// A session storage failure, including an interrupted filesystem worker.
#[derive(Debug, thiserror::Error)]
pub enum SessionStorageError {
    #[error(transparent)]
    Session(#[from] backend::session::Error),
    #[error("Session storage worker could not finish.")]
    Worker(#[source] tokio::task::JoinError),
}
