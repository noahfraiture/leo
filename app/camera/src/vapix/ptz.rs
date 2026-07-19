use axum::{
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use thiserror::Error;

use crate::camera::{Camera, CameraError};

use super::{CameraState, error_response, text_response};

const AVAILABLE_COMMANDS: &str = "Available commands:{camera=[n]}rpan=[offset]";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PtzParams {
    camera: Option<u8>,
    info: Option<u8>,
    rpan: Option<f64>,
}

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

pub(super) async fn handle(
    State(camera): State<CameraState>,
    query: Result<Query<PtzParams>, QueryRejection>,
) -> Result<Response, PtzError> {
    let Query(params) = query.map_err(|_| PtzError::MalformedQuery)?;

    if let Some(response) = information(&params)? {
        return Ok(response);
    }

    movement(camera, params)
}

fn information(params: &PtzParams) -> Result<Option<Response>, PtzError> {
    let Some(info) = params.info else {
        return Ok(None);
    };

    Camera::validate_channel(params.camera.unwrap_or(1))?;
    if info != 1 {
        return Err(PtzError::InvalidInfo);
    }
    if params.rpan.is_some() {
        return Err(PtzError::MixedInfoAndMovement);
    }

    Ok(Some(text_response(StatusCode::OK, AVAILABLE_COMMANDS)))
}

fn movement(camera: CameraState, params: PtzParams) -> Result<Response, PtzError> {
    let channel = params.camera.unwrap_or(1);
    Camera::validate_channel(channel)?;
    let offset = params.rpan.ok_or(PtzError::UnsupportedCommand)?;
    let mut camera = camera.lock().map_err(|_| PtzError::CameraUnavailable)?;
    camera.pan(channel, offset)?;

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}
