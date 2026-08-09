use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::Router;
use tokio::net::TcpListener;

use crate::{api, camera::Camera};

/// Binds the configured address and serves the Synology-shaped API.
pub async fn start(cameras: Vec<Camera>, address: SocketAddr) -> std::io::Result<()> {
    let listener = TcpListener::bind(address).await?;
    axum::serve(listener, app(cameras)).await
}

/// Builds the API router with shared in-memory camera state.
fn app(cameras: Vec<Camera>) -> Router {
    Router::new()
        .nest("/webapi", api::router())
        .with_state(Arc::new(Mutex::new(cameras)))
}
