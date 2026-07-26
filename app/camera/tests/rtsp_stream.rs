#![cfg(unix)]

use std::{
    env, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use tempfile::NamedTempFile;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const READER_DURATION: Duration = Duration::from_secs(12);
const MIN_READER_DURATION: Duration = Duration::from_secs(11);
const READER_TIMEOUT: Duration = Duration::from_secs(30);
const CAMERA_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const LISTENER_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const ESRCH: i32 = 3;
const SIGNAL_EXISTS: i32 = 0;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

#[test]
fn sigint_during_rtsp_startup_cleans_up() {
    let directory = tempfile::tempdir().expect("create fake MediaMTX directory");
    let bin = directory.path().join("bin");
    let tmp = directory.path().join("tmp");
    let started = directory.path().join("started");
    fs::create_dir(&bin).expect("create fake MediaMTX bin directory");
    fs::create_dir(&tmp).expect("create camera TMPDIR");

    let mediamtx = bin.join("mediamtx");
    fs::write(
        &mediamtx,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$FAKE_MEDIAMTX_STARTED\"\nexec /bin/sleep 30\n",
    )
    .expect("write fake MediaMTX");
    fs::set_permissions(&mediamtx, fs::Permissions::from_mode(0o755))
        .expect("make fake MediaMTX executable");

    let mut path = vec![bin];
    path.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path = env::join_paths(path).expect("construct camera PATH");
    let (http_address, rtsp_address) = reserve_addresses();
    let fixture = format!("{}/fixtures/default.mp4", env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_camera"));
    command
        .args(["--address", &http_address.to_string()])
        .args(["--rtsp-address", &rtsp_address.to_string()])
        .args(["--video", &fixture])
        .env("PATH", path)
        .env("TMPDIR", &tmp)
        .env("FAKE_MEDIAMTX_STARTED", &started)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut camera = ProcessGuard::spawn("camera", &mut command);

    wait_for_file(&mut camera, &started, HEALTH_TIMEOUT);
    let fake_mediamtx_pid = fs::read_to_string(&started)
        .expect("read fake MediaMTX PID")
        .trim()
        .parse::<i32>()
        .expect("parse fake MediaMTX PID");
    signal(camera.child.id() as i32, SIGINT).expect("interrupt camera during RTSP startup");
    let status = camera
        .wait_until(CAMERA_SHUTDOWN_TIMEOUT)
        .expect("wait for interrupted camera")
        .expect("camera did not stop after startup interrupt");

    wait_for_process_group_empty(camera.process_group, fake_mediamtx_pid, CLEANUP_TIMEOUT);
    assert!(status.success(), "camera exited unsuccessfully: {status}");
    assert!(
        fs::read_dir(&tmp)
            .expect("read camera TMPDIR")
            .next()
            .is_none(),
        "camera left its temporary RTSP config behind"
    );
}

#[test]
#[ignore = "requires MediaMTX and FFprobe from the Nix development shell"]
fn fixture_streams_h264_to_two_readers_and_stops_cleanly() {
    let (http_address, rtsp_address) = reserve_addresses();
    let fixture = format!("{}/fixtures/default.mp4", env!("CARGO_MANIFEST_DIR"));
    assert!(fs::metadata(&fixture).is_ok_and(|metadata| metadata.is_file()));

    let mut command = Command::new(env!("CARGO_BIN_EXE_camera"));
    command
        .args(["--address", &http_address.to_string()])
        .args(["--rtsp-address", &rtsp_address.to_string()])
        .args(["--video", &fixture])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut camera = ProcessGuard::spawn("camera", &mut command);

    wait_for_health(&mut camera, http_address);

    let url = format!("rtsp://{rtsp_address}/axis-media/media.amp");
    let mut readers = [
        Reader::spawn("reader 1", &url),
        Reader::spawn("reader 2", &url),
    ];
    wait_for_readers(&mut readers);
    for reader in &mut readers {
        reader.assert_h264();
    }

    let status = camera.stop(CAMERA_SHUTDOWN_TIMEOUT);
    assert!(status.success(), "camera exited unsuccessfully: {status}");
    assert_listener_closes("HTTP", http_address);
    assert_listener_closes("RTSP", rtsp_address);
}

fn reserve_addresses() -> (SocketAddr, SocketAddr) {
    let http = TcpListener::bind(("127.0.0.1", 0)).expect("reserve HTTP port");
    let rtsp = TcpListener::bind(("127.0.0.1", 0)).expect("reserve RTSP port");
    let http_address = http.local_addr().expect("read reserved HTTP address");
    let rtsp_address = rtsp.local_addr().expect("read reserved RTSP address");
    assert_ne!(http_address.port(), rtsp_address.port());
    (http_address, rtsp_address)
}

fn wait_for_file(camera: &mut ProcessGuard, path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists() {
            return;
        }
        if let Some(status) = camera.poll().expect("poll camera during RTSP startup") {
            panic!("camera exited before fake MediaMTX started: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "fake MediaMTX did not start within {timeout:?}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_process_group_empty(process_group: i32, fake_mediamtx_pid: i32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        match signal(-process_group, SIGNAL_EXISTS) {
            Err(error) if error.raw_os_error() == Some(ESRCH) => return,
            Err(error) => panic!("inspect camera process group: {error}"),
            Ok(()) => {}
        }
        assert!(
            Instant::now() < deadline,
            "camera process group remained alive after {timeout:?}; fake MediaMTX PID {fake_mediamtx_pid}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_health(camera: &mut ProcessGuard, address: SocketAddr) {
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    loop {
        if health_is_ok(address).unwrap_or(false) {
            return;
        }
        if let Some(status) = camera.poll().expect("poll camera during HTTP readiness") {
            panic!("camera exited before /health became ready: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "/health did not become ready within {HEALTH_TIMEOUT:?}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn health_is_ok(address: SocketAddr) -> io::Result<bool> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(250))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )?;

    let mut response = [0; 64];
    let length = stream.read(&mut response)?;
    Ok(response[..length].starts_with(b"HTTP/1.1 200")
        || response[..length].starts_with(b"HTTP/1.0 200"))
}

struct Reader {
    name: &'static str,
    process: ProcessGuard,
    stdout: NamedTempFile,
    stderr: NamedTempFile,
    started: Instant,
    finished_after: Option<Duration>,
}

impl Reader {
    fn spawn(name: &'static str, url: &str) -> Self {
        let stdout = NamedTempFile::new().expect("create FFprobe stdout file");
        let stderr = NamedTempFile::new().expect("create FFprobe stderr file");
        let mut command = Command::new("ffprobe");
        command
            .args(["-v", "error", "-rtsp_transport", "tcp"])
            .args([
                "-read_intervals",
                &format!("%+{}", READER_DURATION.as_secs()),
            ])
            .args(["-select_streams", "v:0", "-count_packets"])
            .args(["-show_entries", "stream=codec_name,nb_read_packets"])
            .args(["-of", "default=noprint_wrappers=1"])
            .arg(url)
            .stdout(Stdio::from(
                stdout.reopen().expect("open FFprobe stdout file"),
            ))
            .stderr(Stdio::from(
                stderr.reopen().expect("open FFprobe stderr file"),
            ));
        let process = ProcessGuard::spawn(name, &mut command);

        Self {
            name,
            process,
            stdout,
            stderr,
            started: Instant::now(),
            finished_after: None,
        }
    }

    fn assert_h264(&mut self) {
        let status = self
            .process
            .poll()
            .expect("poll completed FFprobe")
            .expect("FFprobe completed");
        let stdout = fs::read_to_string(self.stdout.path()).expect("read FFprobe stdout");
        let stderr = fs::read_to_string(self.stderr.path()).expect("read FFprobe stderr");
        assert!(status.success(), "{} failed: {status}\n{stderr}", self.name);

        let elapsed = self.finished_after.expect("record FFprobe completion time");
        assert!(
            elapsed >= MIN_READER_DURATION,
            "{} consumed the stream for only {elapsed:?}",
            self.name
        );
        assert_eq!(
            entry(&stdout, "codec_name"),
            Some("h264"),
            "{} output:\n{stdout}",
            self.name
        );
        let packets = entry(&stdout, "nb_read_packets")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or_default();
        assert!(
            packets > 150,
            "{} received only {packets} H.264 packets; output:\n{stdout}",
            self.name
        );
    }
}

fn entry<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
}

fn wait_for_readers(readers: &mut [Reader; 2]) {
    let deadline = Instant::now() + READER_TIMEOUT;
    loop {
        let mut complete = true;
        for reader in &mut *readers {
            if reader.finished_after.is_none() {
                if reader.process.poll().expect("poll FFprobe").is_some() {
                    reader.finished_after = Some(reader.started.elapsed());
                } else {
                    complete = false;
                }
            }
        }
        if complete {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "FFprobe readers did not complete within {READER_TIMEOUT:?}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_listener_closes(name: &str, address: SocketAddr) {
    let deadline = Instant::now() + LISTENER_CLOSE_TIMEOUT;
    while TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_ok() {
        assert!(
            Instant::now() < deadline,
            "{name} listener {address} remained open"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

struct ProcessGuard {
    name: &'static str,
    child: Child,
    process_group: i32,
    status: Option<ExitStatus>,
}

impl ProcessGuard {
    fn spawn(name: &'static str, command: &mut Command) -> Self {
        command.process_group(0);
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("start {name}: {error}"));
        let process_group = child.id() as i32;
        Self {
            name,
            child,
            process_group,
            status: None,
        }
    }

    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }

    fn stop(&mut self, timeout: Duration) -> ExitStatus {
        if let Some(status) = self
            .poll()
            .unwrap_or_else(|error| panic!("poll {}: {error}", self.name))
        {
            panic!("{} exited before the test stopped it: {status}", self.name);
        }
        signal(self.child.id() as i32, SIGINT)
            .unwrap_or_else(|error| panic!("stop {}: {error}", self.name));
        self.wait_until(timeout)
            .unwrap_or_else(|error| panic!("wait for {}: {error}", self.name))
            .unwrap_or_else(|| panic!("{} did not stop within {timeout:?}", self.name))
    }

    fn wait_until(&mut self, timeout: Duration) -> io::Result<Option<ExitStatus>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.poll()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if self.poll().ok().flatten().is_none() {
            let _ = signal(self.child.id() as i32, SIGINT);
            let _ = self.wait_until(CLEANUP_TIMEOUT);
        }
        let _ = signal(-self.process_group, SIGKILL);
        if self.status.is_none() {
            let _ = self.wait_until(CLEANUP_TIMEOUT);
        }
    }
}

fn signal(pid: i32, value: i32) -> io::Result<()> {
    // SAFETY: POSIX kill only reads the integer process and signal identifiers.
    if unsafe { kill(pid, value) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
