use std::{
    fmt::Display,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::camera::{Camera, Status};

type SharedCamera = Arc<Mutex<Camera>>;

const AVAILABLE_COMMANDS: &str = "Available commands:{camera=[n]}rpan=[offset]";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PtzParams {
    camera: Option<u8>,
    info: Option<u8>,
    rpan: Option<f64>,
}

pub async fn start(mut camera: Camera, address: SocketAddr) -> std::io::Result<()> {
    if camera.status != Status::Ready {
        return Ok(());
    }

    let listener = TcpListener::bind(address).await?;
    camera.status = Status::Running;
    axum::serve(listener, app(camera)).await
}

pub fn app(camera: Camera) -> Router {
    let axis = Router::new().route("/ptz.cgi", get(ptz));
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .nest("/axis-cgi/com", axis)
        .with_state(Arc::new(Mutex::new(camera)))
}

async fn ptz(
    State(camera): State<SharedCamera>,
    query: Result<Query<PtzParams>, QueryRejection>,
) -> Response {
    let Query(params) = match query {
        Ok(params) => params,
        Err(error) => return vapix_error(error.body_text()),
    };

    if params.camera.is_some_and(|camera| camera != 1) {
        return vapix_error("Only camera 1 is supported");
    }

    if let Some(info) = params.info {
        if info != 1 {
            return vapix_error("info must be 1");
        }
        if params.rpan.is_some() {
            return vapix_error("info cannot be combined with PTZ commands");
        }
        return text_response(StatusCode::OK, AVAILABLE_COMMANDS);
    }

    let Some(rpan) = params.rpan else {
        return vapix_error("Unsupported PTZ command");
    };
    if !(-360.0..=360.0).contains(&rpan) {
        return vapix_error("rpan must be between -360 and 360");
    }

    let mut camera = match camera.lock() {
        Ok(camera) => camera,
        Err(_) => return vapix_error("Camera state unavailable"),
    };
    camera.pan();

    text_response(StatusCode::NO_CONTENT, "")
}

fn vapix_error(message: impl Display) -> Response {
    text_response(StatusCode::OK, format!("Error:{message}"))
}

fn text_response(status: StatusCode, body: impl Into<String>) -> Response {
    (status, [(header::CONTENT_TYPE, "text/plain")], body.into()).into_response()
}
