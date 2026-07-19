use std::fmt::Display;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use crate::{camera::CameraError, vapix::text_response};

#[derive(Debug, Error)]
pub(super) enum PtzError {
    #[error("Invalid PTZ query")]
    MalformedQuery,
    #[error("info must be 1")]
    InvalidInfo,
    #[error("info cannot be combined with PTZ commands")]
    MixedInfoAndMovement,
    #[error("Unsupported PTZ command")]
    UnsupportedCommand,
    #[error("Camera state unavailable")]
    CameraUnavailable,
    #[error(transparent)]
    Camera(#[from] CameraError),
}

impl IntoResponse for PtzError {
    fn into_response(self) -> Response {
        error_response(self)
    }
}

fn error_response(error: impl Display) -> Response {
    text_response(StatusCode::OK, format!("Error:{error}"))
}
