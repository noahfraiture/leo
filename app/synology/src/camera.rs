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

#[cfg(test)]
mod tests {
    use super::Camera;

    #[test]
    fn assigns_identity_from_argument_order() {
        let camera = Camera::new(1, "127.0.0.1:8001".parse().unwrap());

        assert_eq!(camera.id, 2);
        assert_eq!(camera.name, "camera-2");
        assert_eq!(camera.address, "127.0.0.1:8001".parse().unwrap());
        assert!(!camera.recording);
    }

    #[test]
    fn cameras_have_independent_recording_state() {
        let mut first = Camera::new(0, "127.0.0.1:8001".parse().unwrap());
        let second = Camera::new(1, "127.0.0.1:8002".parse().unwrap());

        first.recording = true;

        assert!(first.recording);
        assert!(!second.recording);
    }
}
