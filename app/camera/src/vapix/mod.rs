mod ptz;

use std::{
    fmt::Display,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::camera::Camera;

pub(crate) type CameraState = Arc<Mutex<Camera>>;

pub(crate) fn router() -> Router<CameraState> {
    Router::new().route("/com/ptz.cgi", get(ptz::handle))
}

fn error_response(error: impl Display) -> Response {
    text_response(StatusCode::OK, format!("Error:{error}"))
}

fn text_response(status: StatusCode, body: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body.into()).into_response()
}
