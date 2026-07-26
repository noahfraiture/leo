use std::{net::SocketAddr, path::PathBuf, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("failed to canonicalize video fixture {path:?}")]
    Fixture {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("video fixture is not a regular file: {0:?}")]
    FixtureNotFile(PathBuf),
    #[error("video fixture path is not valid UTF-8: {0:?}")]
    FixtureNotUtf8(PathBuf),
    #[error("failed to serialize a MediaMTX configuration value")]
    SerializeConfig(#[source] serde_json::Error),
    #[error("failed to create temporary MediaMTX configuration")]
    CreateConfig(#[source] std::io::Error),
    #[error("failed to write temporary MediaMTX configuration")]
    WriteConfig(#[source] std::io::Error),
    #[error("mediamtx executable was not found on PATH")]
    MediaMtxNotFound(#[source] std::io::Error),
    #[error("failed to start mediamtx")]
    Spawn(#[source] std::io::Error),
    #[error("mediamtx exited before RTSP became ready: {0}")]
    ExitedBeforeReady(ExitStatus),
    #[error("RTSP listener {address} did not become ready within {timeout:?}")]
    ReadinessTimeout {
        address: SocketAddr,
        timeout: Duration,
    },
    #[error("failed while waiting for mediamtx")]
    Wait(#[source] std::io::Error),
    #[error("mediamtx exited unexpectedly: {0}")]
    UnexpectedExit(ExitStatus),
    #[error("failed to stop mediamtx")]
    Stop(#[source] std::io::Error),
}
