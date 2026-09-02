use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream, UdpSocket},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{ConfigFile, Error};

const MEDIAMTX_VERSION: &str = "v1.18.2";
const WEBRTC_HTTP_ADDRESS: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8889));
const WEBRTC_UDP_ADDRESS: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8189));
const READINESS_TIMEOUT: Duration = Duration::from_secs(5);
const PROBE_INTERVAL: Duration = Duration::from_millis(50);

pub(crate) struct Bridge {
    child: Option<Child>,
    _config: ConfigFile,
}

impl Bridge {
    fn new(child: Child, config: ConfigFile) -> Self {
        Self {
            child: Some(child),
            _config: config,
        }
    }

    fn cleanup(&mut self) -> Result<(), Error> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let (status_error, stop_error) = match child.try_wait() {
            Ok(Some(_)) => (None, None),
            Ok(None) => (None, child.kill().err().map(Error::Stop)),
            Err(error) => (
                Some(Error::Wait(error)),
                child.kill().err().map(Error::Stop),
            ),
        };
        let wait_error = child.wait().err().map(Error::Wait);

        match status_error.or(stop_error).or(wait_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(crate) fn stop(mut self) -> Result<(), Error> {
        self.cleanup()
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            tracing::error!(error = %error, "preview cleanup failed");
        }
    }
}

#[derive(Clone, PartialEq)]
/// Credential-bearing camera input used only to configure the local preview bridge.
pub struct CameraSource {
    /// Stable deployment camera ID, independent of the preview path index.
    pub id: u32,
    pub name: String,
    pub rtsp_url: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
/// Browser-safe metadata for one live camera preview.
pub struct PreviewFeed {
    /// Stable deployment camera ID, independent of the preview path index.
    pub camera_id: u32,
    pub name: String,
    pub video_id: String,
    pub whep_url: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(crate) enum PreviewState {
    NoCameras,
    Ready {
        feeds: Vec<PreviewFeed>,
        script_url: String,
    },
    Unavailable {
        message: String,
    },
}

fn validate_version(output: &[u8]) -> Result<(), Error> {
    let version = String::from_utf8_lossy(output).trim().to_owned();
    if version == MEDIAMTX_VERSION {
        Ok(())
    } else {
        Err(Error::UnsupportedVersion(version))
    }
}

fn verify_version(executable: &str) -> Result<(), Error> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .map_err(Error::spawn)?;
    if !output.status.success() {
        return Err(Error::VersionCheckFailed(output.status));
    }
    validate_version(&output.stdout)
}

fn reserve_ports(
    tcp_address: SocketAddr,
    udp_address: SocketAddr,
) -> Result<(TcpListener, UdpSocket), Error> {
    let tcp = TcpListener::bind(tcp_address).map_err(|source| Error::PortUnavailable {
        address: tcp_address,
        source,
    })?;
    let udp = UdpSocket::bind(udp_address).map_err(|source| Error::PortUnavailable {
        address: udp_address,
        source,
    })?;
    Ok((tcp, udp))
}

fn wait_until_ready(
    child: &mut Child,
    address: SocketAddr,
    timeout: Duration,
) -> Result<(), Error> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(Error::Wait)? {
            return Err(Error::ExitedBeforeReady(status));
        }
        let ready = probe(address, deadline);
        if let Some(status) = child.try_wait().map_err(Error::Wait)? {
            return Err(Error::ExitedBeforeReady(status));
        }
        if ready {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Error::ReadinessTimeout { address, timeout });
        }
        thread::sleep(PROBE_INTERVAL.min(remaining));
    }
}

fn probe(address: SocketAddr, deadline: Instant) -> bool {
    let mut io_timeout = PROBE_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
    if io_timeout.is_zero() {
        return false;
    }
    let Ok(mut stream) = TcpStream::connect_timeout(&address, io_timeout) else {
        return false;
    };
    io_timeout = PROBE_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
    if io_timeout.is_zero() || stream.set_write_timeout(Some(io_timeout)).is_err() {
        return false;
    }
    let request =
        format!("OPTIONS /camera-0/whep HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = [0; 8192];
    let mut size = 0;
    while size < response.len() {
        io_timeout = PROBE_INTERVAL.min(deadline.saturating_duration_since(Instant::now()));
        if io_timeout.is_zero() || stream.set_read_timeout(Some(io_timeout)).is_err() {
            return false;
        }
        let Ok(read) = stream.read(&mut response[size..]) else {
            return false;
        };
        if read == 0 {
            break;
        }
        size += read;
        if response[..size]
            .windows(4)
            .any(|bytes| bytes == b"\r\n\r\n")
        {
            break;
        }
    }
    let Ok(response) = std::str::from_utf8(&response[..size]) else {
        return false;
    };
    let Some(headers) = response.split_once("\r\n\r\n").map(|(headers, _)| headers) else {
        return false;
    };
    let mut lines = headers.lines();
    lines.next() == Some("HTTP/1.1 204 No Content")
        && lines.any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("Accept-Post")
                    && value.trim().eq_ignore_ascii_case("application/sdp")
            })
        })
}

pub(crate) fn start(sources: Vec<CameraSource>) -> Result<(PreviewState, Bridge), Error> {
    let ports = reserve_ports(WEBRTC_HTTP_ADDRESS, WEBRTC_UDP_ADDRESS)?;
    verify_version("mediamtx")?;
    let config = ConfigFile::create(&sources)?;
    drop(ports);
    let mut child = Command::new("mediamtx")
        .arg(config.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(Error::spawn)?;

    if let Err(error) = wait_until_ready(&mut child, WEBRTC_HTTP_ADDRESS, READINESS_TIMEOUT) {
        if let Err(cleanup_error) = Bridge::new(child, config).stop() {
            tracing::error!(error = %cleanup_error, "preview startup cleanup failed");
        }
        return Err(error);
    }
    let state = preview_metadata(&sources);
    Ok((state, Bridge::new(child, config)))
}

pub(crate) fn preview_metadata(sources: &[CameraSource]) -> PreviewState {
    let feeds = sources
        .iter()
        .enumerate()
        .map(|(index, source)| PreviewFeed {
            camera_id: source.id,
            name: source.name.clone(),
            video_id: format!("camera-{index}-video"),
            whep_url: format!("http://127.0.0.1:8889/camera-{index}/whep"),
        })
        .collect();
    PreviewState::Ready {
        feeds,
        script_url: "http://127.0.0.1:8889/reader.js".into(),
    }
}

#[cfg(test)]
#[path = "tests/bridge.rs"]
mod tests;
