use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub(crate) enum Error {
    #[error("Only camera 1 is supported")]
    UnsupportedChannel,
    #[error("rpan must be between -360 and 360")]
    PanOutOfRange,
}
