use std::{net::SocketAddr, time::Duration};

use tokio::{net::TcpStream, time::timeout};

#[derive(Clone)]
pub struct Camera {
    pub id: u32,
    pub name: String,
    pub address: SocketAddr,
    pub recording: bool,
}

impl Camera {
    pub fn new(index: usize, address: SocketAddr) -> Self {
        let id = index as u32 + 1;
        Self {
            id,
            name: format!("camera-{id}"),
            address,
            recording: false,
        }
    }

    pub(crate) async fn reachable(&self) -> bool {
        matches!(
            timeout(Duration::from_millis(250), TcpStream::connect(self.address)).await,
            Ok(Ok(_))
        )
    }
}
