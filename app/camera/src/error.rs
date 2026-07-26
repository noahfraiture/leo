#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("HTTP server failed")]
    Http(#[source] std::io::Error),
    #[error("failed to listen for Ctrl-C")]
    ShutdownSignal(#[source] std::io::Error),
    #[error(transparent)]
    Rtsp(#[from] crate::rtsp::Error),
}
