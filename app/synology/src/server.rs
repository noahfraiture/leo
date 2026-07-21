use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::Router;
use tokio::net::TcpListener;

use crate::{api, camera::Camera};

pub async fn start(cameras: Vec<Camera>, address: SocketAddr) -> std::io::Result<()> {
    let listener = TcpListener::bind(address).await?;
    axum::serve(listener, app(cameras)).await
}

fn app(cameras: Vec<Camera>) -> Router {
    Router::new()
        .nest("/webapi", api::router())
        .with_state(Arc::new(Mutex::new(cameras)))
}
