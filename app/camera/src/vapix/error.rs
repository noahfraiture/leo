use std::fmt::Display;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use super::ptz::text_response;
use crate::camera;

#[derive(Debug, Error)]
pub(super) enum Error {
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
    Camera(#[from] camera::Error),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        error_response(self)
    }
}

fn error_response(error: impl Display) -> Response {
    text_response(StatusCode::OK, format!("Error:{error}"))
}
