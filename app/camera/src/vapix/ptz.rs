use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;

use crate::camera::Camera;

use super::Error;

const AVAILABLE_COMMANDS: &str = "Available commands:{camera=[n]}rpan=[offset]";

pub(crate) type CameraState = Arc<Mutex<Camera>>;

pub(crate) fn router() -> Router<CameraState> {
    Router::new().route("/com/ptz.cgi", get(handle))
}

pub(super) fn text_response(status: StatusCode, body: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body.into()).into_response()
}

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
) -> Result<Response, Error> {
    let Query(params) = query.map_err(|_| Error::MalformedQuery)?;
    Camera::validate_channel(params.camera_channel.unwrap_or(1))?;

    if params.info.is_some() {
        return information(&params);
    }

    if params.rpan.is_none() && params.rtilt.is_none() {
        return Err(Error::UnsupportedCommand);
    }

    if params.rpan.is_some() {
        rpan(camera.clone(), &params)?;
    }
    if params.rtilt.is_some() {
        rtilt(camera.clone(), &params)?;
    }

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}

fn information(params: &PtzParams) -> Result<Response, Error> {
    Camera::validate_channel(params.camera_channel.unwrap_or(1))?;
    if params.info.unwrap() != 1 {
        return Err(Error::InvalidInfo);
    }
    if params.rpan.is_some() || params.rtilt.is_some() {
        return Err(Error::MixedInfoAndMovement);
    }

    Ok(text_response(StatusCode::OK, AVAILABLE_COMMANDS))
}

fn rpan(camera: CameraState, params: &PtzParams) -> Result<Response, Error> {
    let offset = params.rpan.ok_or(Error::UnsupportedCommand)?;
    let mut camera = camera.lock().map_err(|_| Error::CameraUnavailable)?;
    camera.pan(params.camera_channel.unwrap_or(1), offset)?;

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}

fn rtilt(camera: CameraState, params: &PtzParams) -> Result<Response, Error> {
    let offset = params.rtilt.ok_or(Error::UnsupportedCommand)?;
    let mut camera = camera.lock().map_err(|_| Error::CameraUnavailable)?;
    camera.tilt(params.camera_channel.unwrap_or(1), offset)?;

    Ok(text_response(StatusCode::NO_CONTENT, ""))
}
