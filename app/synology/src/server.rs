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

pub fn app(cameras: Vec<Camera>) -> Router {
    Router::new()
        .nest("/webapi", api::router())
        .with_state(Arc::new(Mutex::new(cameras)))
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::start;

    #[tokio::test]
    async fn start_reports_bind_failures() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();

        assert!(start(vec![], listener.local_addr().unwrap()).await.is_err());
    }
}
