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
mod tests {
    use std::{
        env,
        io::{Read, Write},
        net::{TcpListener, UdpSocket},
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    use super::{
        Bridge, PROBE_INTERVAL, READINESS_TIMEOUT, reserve_ports, validate_version, verify_version,
        wait_until_ready,
    };
    use crate::preview::{CameraSource, ConfigFile, Error, PreviewState, preview_metadata};

    const LIVE_CHILD_ENV: &str = "APP_PREVIEW_LIVE_CHILD";
    const LIVE_CHILD_TEST: &str = "preview::bridge::tests::live_child_process";

    fn child(args: &[&str]) -> Child {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    }

    fn live_child() -> Child {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args(["--exact", LIVE_CHILD_TEST])
            .env(LIVE_CHILD_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().unwrap()
    }

    fn config() -> ConfigFile {
        ConfigFile::create(&[]).unwrap()
    }

    #[cfg(unix)]
    fn process_exists(id: u32) -> bool {
        Command::new("kill")
            .args(["-0", &id.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    #[test]
    fn live_child_process() {
        if env::var_os(LIVE_CHILD_ENV).is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }

    #[test]
    fn accepts_supported_mediamtx_version() {
        assert_eq!(validate_version(b"v1.18.2\n"), Ok(()));
    }

    #[test]
    fn rejects_unsupported_mediamtx_version() {
        assert!(matches!(
            validate_version(b"v1.18.1\n"),
            Err(Error::UnsupportedVersion(version)) if version == "v1.18.1"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_supported_version_from_unsuccessful_command() {
        use std::{fs, os::unix::fs::PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("mediamtx");
        fs::write(&executable, "#!/bin/sh\nprintf 'v1.18.2\\n'\nexit 7\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(matches!(
            verify_version(executable.to_str().unwrap()),
            Err(Error::VersionCheckFailed(status)) if status.code() == Some(7)
        ));
    }

    #[test]
    fn reports_missing_mediamtx_executable() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("missing-mediamtx");

        let error = verify_version(executable.to_str().unwrap()).unwrap_err();

        assert!(matches!(error, Error::MediaMtxNotFound(_)));
    }

    #[test]
    fn rejects_occupied_tcp_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let error = reserve_ports(address, "127.0.0.1:0".parse().unwrap()).unwrap_err();

        assert!(matches!(
            error,
            Error::PortUnavailable {
                address: unavailable,
                ..
            } if unavailable == address
        ));
    }

    #[test]
    fn rejects_occupied_udp_port() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = socket.local_addr().unwrap();

        let error = reserve_ports("127.0.0.1:0".parse().unwrap(), address).unwrap_err();

        assert!(matches!(
            error,
            Error::PortUnavailable {
                address: unavailable,
                ..
            } if unavailable == address
        ));
    }

    #[test]
    fn readiness_rejects_unrelated_http_listener_that_stays_alive() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let timeout = Duration::from_millis(150);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + timeout + Duration::from_millis(100);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_read_timeout(Some(PROBE_INTERVAL)).unwrap();
                        let _ = stream.read(&mut [0; 512]);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("unrelated listener failed: {error}"),
                }
            }
        });
        let mut child = live_child();

        let result = wait_until_ready(&mut child, address, timeout);

        server.join().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(result, Err(Error::ReadinessTimeout { .. })));
    }

    #[test]
    fn readiness_succeeds_without_authorization_header() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = [0; 1024];
            let size = stream.read(&mut request).unwrap();
            let request = std::str::from_utf8(&request[..size]).unwrap();
            assert!(request.starts_with("OPTIONS /camera-0/whep HTTP/1.1\r\n"));
            assert!(!request.lines().any(|line| {
                line.split_once(':')
                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case("Authorization"))
            }));
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nAccept-Post: application/sdp\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
        });
        let mut child = live_child();

        let result = wait_until_ready(&mut child, address, Duration::from_secs(1));

        let server_result = server.join();
        child.kill().unwrap();
        child.wait().unwrap();
        server_result.unwrap();
        result.unwrap();
    }

    #[test]
    fn readiness_reports_child_exit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut child = child(&["--list"]);

        let error = wait_until_ready(&mut child, address, Duration::from_secs(1)).unwrap_err();

        assert!(matches!(
            error,
            Error::ExitedBeforeReady(status) if status.success()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn readiness_rejects_unrelated_listener_when_child_exits() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut child = live_child();
        let child_id = child.id();
        let terminator = std::thread::spawn(move || {
            listener.accept().unwrap();
            assert!(
                Command::new("kill")
                    .arg(child_id.to_string())
                    .status()
                    .unwrap()
                    .success()
            );
        });

        let result = wait_until_ready(&mut child, address, Duration::from_secs(1));

        terminator.join().unwrap();
        child.wait().unwrap();
        assert!(matches!(result, Err(Error::ExitedBeforeReady(_))));
    }

    #[test]
    fn readiness_timeout_is_bounded() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for _ in 0..20 {
                if stream.write_all(b"H").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        let mut child = live_child();
        let timeout = Duration::from_millis(100);
        let started = Instant::now();

        let error = wait_until_ready(&mut child, address, timeout).unwrap_err();
        let elapsed = started.elapsed();

        server.join().unwrap();
        assert!(elapsed < Duration::from_millis(500));
        assert!(matches!(
            error,
            Error::ReadinessTimeout {
                address: timed_out,
                timeout: elapsed,
            } if timed_out == address && elapsed == timeout
        ));
        assert_eq!(READINESS_TIMEOUT, Duration::from_secs(5));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stop_terminates_and_reaps_live_child() {
        let child = live_child();
        let id = child.id();

        Bridge::new(child, config()).stop().unwrap();

        assert!(!process_exists(id));
    }

    #[test]
    fn stop_waits_for_already_exited_child() {
        let mut child = child(&["--list"]);
        assert!(child.wait().unwrap().success());

        Bridge::new(child, config()).stop().unwrap();
    }

    #[test]
    fn cleanup_is_idempotent() {
        let mut child = child(&["--list"]);
        assert!(child.wait().unwrap().success());
        let mut bridge = Bridge::new(child, config());

        bridge.cleanup().unwrap();
        assert!(bridge.child.is_none());
        bridge.cleanup().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn drop_terminates_and_reaps_live_child() {
        let child = live_child();
        let id = child.id();

        drop(Bridge::new(child, config()));

        let stopped = !process_exists(id);
        if !stopped {
            Command::new("kill").arg(id.to_string()).status().unwrap();
        }
        assert!(stopped);
    }

    #[test]
    fn metadata_does_not_expose_camera_credentials() {
        let source = CameraSource {
            id: 26,
            name: "Workshop".into(),
            rtsp_url: "rtsp://camera-user:camera-pass@127.0.0.1/live".into(),
        };
        let preview = preview_metadata(&[source]);
        let serialized = serde_json::to_string(&preview).unwrap();

        assert!(serialized.contains("camera-0-video"));
        assert!(serialized.contains("http://127.0.0.1:8889/camera-0/whep"));
        assert!(serialized.contains("http://127.0.0.1:8889/reader.js"));
        assert!(!serialized.contains("\"user\""));
        assert!(!serialized.contains("\"password\""));
        assert!(!serialized.contains("app-preview"));
        assert!(!serialized.contains("local-password"));
        assert!(!serialized.contains("camera-user"));
        assert!(!serialized.contains("camera-pass"));
        assert!(!serialized.contains("rtsp://"));
    }

    #[test]
    fn metadata_preserves_stable_ids_with_index_based_paths() {
        let sources = vec![
            CameraSource {
                id: 26,
                name: "Salon 1".into(),
                rtsp_url: "rtsp://camera-one.example/live".into(),
            },
            CameraSource {
                id: 41,
                name: "Salon 2".into(),
                rtsp_url: "rtsp://camera-two.example/live".into(),
            },
        ];

        let PreviewState::Ready { feeds, .. } = preview_metadata(&sources) else {
            panic!("preview metadata should be ready");
        };

        assert_eq!(feeds[0].camera_id, 26);
        assert_eq!(feeds[0].video_id, "camera-0-video");
        assert_eq!(feeds[0].whep_url, "http://127.0.0.1:8889/camera-0/whep");
        assert_eq!(feeds[1].camera_id, 41);
        assert_eq!(feeds[1].video_id, "camera-1-video");
        assert_eq!(feeds[1].whep_url, "http://127.0.0.1:8889/camera-1/whep");
    }
}
