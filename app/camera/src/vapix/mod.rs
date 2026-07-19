mod error;
mod ptz;

use std::sync::{Arc, Mutex};

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

fn text_response(status: StatusCode, body: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body.into()).into_response()
}
