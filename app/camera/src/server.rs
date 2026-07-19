use std::net::SocketAddr;

use axum::{Router, http::StatusCode, routing::get};
use tokio::net::TcpListener;

use crate::camera::{Camera, Status};

pub async fn start(mut camera: Camera, address: SocketAddr) -> std::io::Result<()> {
    if camera.status != Status::Ready {
        return Ok(());
    }

    let listener = TcpListener::bind(address).await?;
    camera.status = Status::Running;
    axum::serve(listener, app(camera)).await
}

pub fn app(_camera: Camera) -> Router {
    let axis = Router::new().route("/ptz.cgi", get(|| async {}));
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .nest("/axis-cgi/com", axis)
}
