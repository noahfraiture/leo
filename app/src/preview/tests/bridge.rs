use std::{
    env,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use super::{
    Bridge, PROBE_INTERVAL, reserve_ports, validate_version, verify_version, wait_until_ready,
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
    ConfigFile::create(&[], "fixture-preview-password").unwrap()
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
        verify_version(executable.to_str().unwrap(), Duration::from_secs(1)),
        Err(Error::VersionCheckFailed(status)) if status.code() == Some(7)
    ));
}

#[test]
fn reports_missing_mediamtx_executable() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("missing-mediamtx");

    let error = verify_version(executable.to_str().unwrap(), Duration::from_secs(1)).unwrap_err();

    assert!(matches!(error, Error::MediaMtxNotFound(_)));
}

#[cfg(unix)]
#[test]
fn version_check_validates_complete_bounded_output() {
    use std::{fs, os::unix::fs::PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("mediamtx");
    for (body, accepted) in [
        ("printf 'v1.18.2\\n'", true),
        ("printf 'v1.18.2%130s' ''", false),
    ] {
        fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();

        let result = verify_version(executable.to_str().unwrap(), Duration::from_secs(1));

        assert_eq!(result.is_ok(), accepted, "{body}: {result:?}");
    }
}

#[cfg(unix)]
#[test]
fn version_check_times_out_and_reaps_a_stalled_process() {
    use std::{fs, os::unix::fs::PermissionsExt};

    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("stalled-mediamtx");
    let pid = directory.path().join("child.pid");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\nprintf 'v1.18.2\\n'\nexec sleep 2\n",
            pid.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let started = Instant::now();

    let result = verify_version(executable.to_str().unwrap(), Duration::from_millis(100));

    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(result, Err(Error::VersionCheckTimeout { .. })));
    let id = fs::read_to_string(pid).unwrap().parse().unwrap();
    assert!(!process_exists(id));
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

    let result = wait_until_ready(&mut child, address, timeout, "fixture-preview-password");

    server.join().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(matches!(result, Err(Error::ReadinessTimeout { .. })));
}

#[test]
fn readiness_authenticates_with_the_current_bridge_password() {
    use base64::{Engine, engine::general_purpose::STANDARD};
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
        let authorization = request
            .lines()
            .find_map(|line| line.strip_prefix("Authorization: Basic "))
            .expect("readiness must authenticate with its own bridge");
        assert_eq!(
            STANDARD.decode(authorization).unwrap(),
            b"leo-preview:fixture-preview-password"
        );
        stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nAccept-Post: application/sdp\r\nContent-Length: 0\r\n\r\n",
                )
                .unwrap();
    });
    let mut child = live_child();

    let result = wait_until_ready(
        &mut child,
        address,
        Duration::from_secs(1),
        "fixture-preview-password",
    );

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

    let error = wait_until_ready(
        &mut child,
        address,
        Duration::from_secs(1),
        "fixture-preview-password",
    )
    .unwrap_err();

    assert!(matches!(
        error,
        Error::ExitedBeforeReady(status) if status.success()
    ));
}

#[cfg(unix)]
#[test]
fn stop_terminates_and_reaps_live_child() {
    let child = live_child();
    let id = child.id();

    Bridge::new(child, config()).stop().unwrap();

    assert!(!process_exists(id));
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
    let preview = preview_metadata(&[source], "fixture-preview-password");
    let serialized = serde_json::to_string(&preview).unwrap();

    assert!(serialized.contains("camera-0-video"));
    assert!(serialized.contains("http://127.0.0.1:8889/camera-0/whep"));
    assert!(serialized.contains("http://127.0.0.1:8889/reader.js"));
    assert!(serialized.contains("fixture-preview-password"));
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

    let PreviewState::Ready { feeds, .. } = preview_metadata(&sources, "fixture-preview-password")
    else {
        panic!("preview metadata should be ready");
    };

    assert_eq!(feeds[0].camera_id, 26);
    assert_eq!(feeds[0].video_id, "camera-0-video");
    assert_eq!(feeds[0].whep_url, "http://127.0.0.1:8889/camera-0/whep");
    assert_eq!(feeds[1].camera_id, 41);
    assert_eq!(feeds[1].video_id, "camera-1-video");
    assert_eq!(feeds[1].whep_url, "http://127.0.0.1:8889/camera-1/whep");
}

#[test]
#[ignore = "requires the local MediaMTX binary"]
fn preview_authentication_rejects_missing_and_stale_passwords() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{fs, io::BufRead, io::BufReader, net::TcpStream};

    let config = ConfigFile::create(
        &[CameraSource {
            id: 1,
            name: "Unused authentication fixture".into(),
            rtsp_url: "rtsp://127.0.0.1:1/live".into(),
        }],
        "current-bridge-password",
    )
    .unwrap();
    let http = TcpListener::bind("127.0.0.1:0").unwrap();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = http.local_addr().unwrap();
    // Isolate this network check from any active desktop preview while keeping its auth config.
    let contents = fs::read_to_string(config.path())
        .unwrap()
        .replace("127.0.0.1:8889", &address.to_string())
        .replace("127.0.0.1:8189", &udp.local_addr().unwrap().to_string());
    fs::write(config.path(), contents).unwrap();
    drop((http, udp));
    let child = Command::new("mediamtx")
        .arg(config.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut bridge = Bridge::new(child, config);
    wait_until_ready(
        bridge.child.as_mut().unwrap(),
        address,
        Duration::from_secs(5),
        "current-bridge-password",
    )
    .unwrap();

    // Authenticated readiness above proves the current password is accepted. These requests must
    // be rejected before MediaMTX attempts to open the deliberately unavailable camera source.
    for (label, password) in [
        ("missing", None),
        ("stale", Some("previous-bridge-password")),
    ] {
        let mut connection = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
        connection
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let authorization = password
            .map(|password| {
                format!(
                    "Authorization: Basic {}\r\n",
                    STANDARD.encode(format!("leo-preview:{password}"))
                )
            })
            .unwrap_or_default();
        let request = format!(
            "POST /camera-0/whep HTTP/1.1\r\nHost: {address}\r\n{authorization}Content-Type: application/sdp\r\nContent-Length: 5\r\nConnection: close\r\n\r\nv=0\r\n"
        );
        connection.write_all(request.as_bytes()).unwrap();
        let mut status_line = String::new();
        BufReader::new(connection)
            .read_line(&mut status_line)
            .unwrap_or_else(|error| panic!("read {label} credential response: {error}"));
        assert_eq!(
            status_line.trim_end(),
            "HTTP/1.1 401 Unauthorized",
            "{label} credential"
        );
    }
}
