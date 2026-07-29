use std::{net::SocketAddr, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("unsupported MediaMTX version {0}; expected v1.18.2")]
    UnsupportedVersion(String),
    #[error("mediamtx --version exited unsuccessfully: {0}")]
    VersionCheckFailed(ExitStatus),
    #[error("failed to serialize a MediaMTX configuration value")]
    SerializeConfig(#[from] serde_json::Error),
    #[error("failed to create temporary MediaMTX configuration")]
    CreateConfig(#[source] std::io::Error),
    #[error("failed to write temporary MediaMTX configuration")]
    WriteConfig(#[source] std::io::Error),
    #[error("mediamtx exited before WebRTC became ready: {0}")]
    ExitedBeforeReady(ExitStatus),
    #[error("WebRTC listener {address} did not become ready within {timeout:?}")]
    ReadinessTimeout {
        address: SocketAddr,
        timeout: Duration,
    },
    #[error("preview port {address} is unavailable")]
    PortUnavailable {
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("mediamtx executable was not found on PATH")]
    MediaMtxNotFound(#[source] std::io::Error),
    #[error("failed to start mediamtx")]
    Spawn(#[source] std::io::Error),
    #[error("failed while waiting for mediamtx")]
    Wait(#[source] std::io::Error),
    #[error("failed to stop mediamtx")]
    Stop(#[source] std::io::Error),
}

impl Error {
    pub(super) fn spawn(source: std::io::Error) -> Self {
        if source.kind() == std::io::ErrorKind::NotFound {
            Self::MediaMtxNotFound(source)
        } else {
            Self::Spawn(source)
        }
    }
}

#[cfg(test)]
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}
