use std::{net::SocketAddr, time::Duration};

use tokio::{net::TcpStream, time::timeout};

use crate::recording::Recording;

/// One camera known to the simulator and its attached fixture recordings.
#[derive(Clone)]
pub struct Camera {
    /// One-based identifier assigned from command-line order.
    pub id: u32,
    /// Stable simulator-generated display name.
    pub name: String,
    /// Camera HTTP address used for reachability checks.
    pub address: SocketAddr,
    /// Legacy ExternalRecording state, independent from the fixture catalogue.
    pub recording: bool,
    /// Immutable recordings loaded from the optional catalogue at startup.
    pub(crate) recordings: Vec<Recording>,
}

impl Camera {
    /// Creates a camera with its one-based ID and an empty recording catalogue.
    pub fn new(index: usize, address: SocketAddr) -> Self {
        let id = index as u32 + 1;
        Self {
            id,
            name: format!("camera-{id}"),
            address,
            recording: false,
            recordings: Vec::new(),
        }
    }

    /// Reports whether the camera address accepts a TCP connection within 250 ms.
    pub async fn reachable(&self) -> bool {
        matches!(
            timeout(Duration::from_millis(250), TcpStream::connect(self.address)).await,
            Ok(Ok(_))
        )
    }
}
