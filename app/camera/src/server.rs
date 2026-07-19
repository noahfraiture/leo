use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{Router, http::StatusCode, routing::get};
use tokio::net::TcpListener;

use crate::{
    camera::{Camera, Status},
    vapix,
};

pub async fn start(mut camera: Camera, address: SocketAddr) -> std::io::Result<()> {
    if camera.status != Status::Ready {
        return Ok(());
    }

    let listener = TcpListener::bind(address).await?;
    camera.status = Status::Running;
    axum::serve(listener, app(camera)).await
}

pub fn app(camera: Camera) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .nest("/axis-cgi", vapix::router())
        .with_state(Arc::new(Mutex::new(camera)))
}
