use axum::{
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::Response,
};
use serde::Deserialize;

use crate::{camera::Camera, vapix::error::PtzError};

use super::{CameraState, text_response};

const AVAILABLE_COMMANDS: &str = "Available commands:{camera=[n]}rpan=[offset]";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PtzParams {
    camera: Option<u8>,
    info: Option<u8>,
    rpan: Option<f64>,
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
