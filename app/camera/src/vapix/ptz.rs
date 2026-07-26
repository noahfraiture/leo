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
    #[serde(rename = "camera")]
    /// Channel used for the camera, the value is ignored.
    camera_channel: Option<u8>,

    info: Option<u8>,
    rpan: Option<f64>,
    rtilt: Option<f64>,
}

pub(super) async fn handle(
    State(camera): State<CameraState>,
    query: Result<Query<PtzParams>, QueryRejection>,
) -> Result<Response, PtzError> {
    let Query(params) = query.map_err(|_| PtzError::MalformedQuery)?;
    Camera::validate_channel(params.camera_channel.unwrap_or(1))?;

    if params.info.is_some() {
        return information(&params);
    }

    if params.rpan.is_none() && params.rtilt.is_none() {
        return Err(PtzError::UnsupportedCommand);
    }

    if params.rpan.is_some() {
        rpan(camera.clone(), &params)?;
    }
    if params.rtilt.is_some() {
        rtilt(camera.clone(), &params)?;
    }

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}

fn information(params: &PtzParams) -> Result<Response, PtzError> {
    Camera::validate_channel(params.camera_channel.unwrap_or(1))?;
    if params.info.unwrap() != 1 {
        return Err(PtzError::InvalidInfo);
    }
    if params.rpan.is_some() || params.rtilt.is_some() {
        return Err(PtzError::MixedInfoAndMovement);
    }

    Ok(text_response(StatusCode::OK, AVAILABLE_COMMANDS))
}

fn rpan(camera: CameraState, params: &PtzParams) -> Result<Response, PtzError> {
    let offset = params.rpan.ok_or(PtzError::UnsupportedCommand)?;
    let mut camera = camera.lock().map_err(|_| PtzError::CameraUnavailable)?;
    camera.pan(params.camera_channel.unwrap_or(1), offset)?;

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}

fn rtilt(camera: CameraState, params: &PtzParams) -> Result<Response, PtzError> {
    let offset = params.rtilt.ok_or(PtzError::UnsupportedCommand)?;
    let mut camera = camera.lock().map_err(|_| PtzError::CameraUnavailable)?;
    camera.tilt(params.camera_channel.unwrap_or(1), offset)?;

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}
