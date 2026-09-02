#![cfg(unix)]

use std::{
    env, fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use backend::{
    analysis::extract_jpeg_for_test,
    recording::{
        RecorderEvent, RecorderSettings, RecorderStatus, RecordingCamera, RecordingSegment,
        test_support,
    },
};
use tempfile::NamedTempFile;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const READER_DURATION: Duration = Duration::from_secs(12);
const MIN_READER_DURATION: Duration = Duration::from_secs(11);
const READER_TIMEOUT: Duration = Duration::from_secs(30);
const CAMERA_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const RECORDER_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
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

#[tokio::test]
#[ignore = "requires MediaMTX, FFmpeg, and FFprobe from the Nix development shell"]
async fn host_recorder_records_playable_mkv() {
    let (http_address, rtsp_address) = reserve_addresses();
    let fixture = camera_fixture("salon-1.mp4");
    let mut camera = start_camera(http_address, rtsp_address, &fixture);
    let recordings = tempfile::tempdir().expect("create recorder output directory");
    let recordings_root = create_recordings_root(recordings.path());
    let camera_directory = recordings_root.join("camera-1");
    let processes = tempfile::tempdir().expect("create recorder process directory");
    let (ffmpeg, ffmpeg_pids) = instrumented_executable(processes.path(), "ffmpeg");
    let (ffprobe, ffprobe_pids) = instrumented_executable(processes.path(), "ffprobe");
    let (runtime, recorder, mut events) =
        test_support::spawn(recorder_settings(), ffmpeg, ffprobe).expect("start recorder runtime");

    tokio::time::timeout(
        RECORDER_OPERATION_TIMEOUT,
        recorder.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: format!("rtsp://{rtsp_address}/axis-media/media.amp"),
            }],
            recordings_root,
        ),
    )
    .await
    .expect("host recorder Start timed out")
    .expect("start host recorder");
    wait_for_recorder_status(&mut events, RecorderStatus::Recording);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let segments = tokio::time::timeout(RECORDER_OPERATION_TIMEOUT, recorder.stop())
        .await
        .expect("host recorder Stop timed out")
        .expect("stop host recorder");
    wait_for_recorder_status(&mut events, RecorderStatus::Stopped);
    runtime.shutdown().expect("shut down recorder runtime");

    assert_eq!(segments.len(), 1, "expected one finalized segment");
    let segment = &segments[0];
    assert_eq!(segment.camera_id, 1);
    assert!(segment.start_utc_ms < segment.end_utc_ms);
    assert_finalized_files_match_segments(&camera_directory, &segments);
    assert_finalized_matroska_h264(&segment.path);

    let camera_pid = camera.child.id() as i32;
    let status = camera.stop(CAMERA_SHUTDOWN_TIMEOUT);
    assert!(status.success(), "camera exited unsuccessfully: {status}");
    wait_for_process_group_empty(camera.process_group, camera_pid, CLEANUP_TIMEOUT);
    assert_listener_closes("HTTP", http_address);
    assert_listener_closes("RTSP", rtsp_address);
    assert_recorded_processes_stopped(&ffmpeg_pids);
    assert_recorded_processes_stopped(&ffprobe_pids);
}

#[tokio::test]
#[ignore = "requires MediaMTX, FFmpeg, and FFprobe from the Nix development shell"]
async fn host_recorder_reconnects_into_a_second_segment() {
    let (http_address, rtsp_address) = reserve_addresses();
    let fixture = camera_fixture("salon-2.mp4");
    let mut camera = start_camera(http_address, rtsp_address, &fixture);
    let recordings = tempfile::tempdir().expect("create recorder output directory");
    let recordings_root = create_recordings_root(recordings.path());
    let camera_directory = recordings_root.join("camera-1");
    let processes = tempfile::tempdir().expect("create recorder process directory");
    let (ffmpeg, ffmpeg_pids) = instrumented_executable(processes.path(), "ffmpeg");
    let (ffprobe, ffprobe_pids) = instrumented_executable(processes.path(), "ffprobe");
    let (runtime, recorder, mut events) =
        test_support::spawn(recorder_settings(), ffmpeg, ffprobe).expect("start recorder runtime");

    tokio::time::timeout(
        RECORDER_OPERATION_TIMEOUT,
        recorder.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: format!("rtsp://{rtsp_address}/axis-media/media.amp"),
            }],
            recordings_root,
        ),
    )
    .await
    .expect("host recorder Start timed out")
    .expect("start host recorder");
    wait_for_recorder_status(&mut events, RecorderStatus::Recording);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let first_camera_pid = camera.child.id() as i32;
    let status = camera.stop(CAMERA_SHUTDOWN_TIMEOUT);
    assert!(status.success(), "camera exited unsuccessfully: {status}");
    wait_for_process_group_empty(camera.process_group, first_camera_pid, CLEANUP_TIMEOUT);
    assert_listener_closes("HTTP", http_address);
    assert_listener_closes("RTSP", rtsp_address);
    wait_for_recorder_status(&mut events, RecorderStatus::Reconnecting);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let mut camera = start_camera(http_address, rtsp_address, &fixture);
    wait_for_recorder_status(&mut events, RecorderStatus::Recording);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let segments = tokio::time::timeout(RECORDER_OPERATION_TIMEOUT, recorder.stop())
        .await
        .expect("host recorder Stop timed out")
        .expect("stop host recorder");
    wait_for_recorder_status(&mut events, RecorderStatus::Stopped);
    runtime.shutdown().expect("shut down recorder runtime");

    assert_eq!(segments.len(), 2, "expected pre- and post-gap segments");
    assert_finalized_files_match_segments(&camera_directory, &segments);
    assert_eq!(segments[0].camera_id, 1);
    assert_eq!(segments[1].camera_id, 1);
    assert!(segments[0].start_utc_ms < segments[1].start_utc_ms);
    let gap_ms = segments[1].start_utc_ms - segments[0].end_utc_ms;
    assert!(
        gap_ms > 0,
        "expected a positive reconnect gap, got {gap_ms}ms"
    );
    for segment in &segments {
        assert!(segment.start_utc_ms < segment.end_utc_ms);
        assert_finalized_matroska_h264(&segment.path);
        let jpeg = extract_jpeg_for_test(&segment.path, Duration::from_millis(500))
            .expect("extract JPEG from finalized segment");
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(jpeg.ends_with(&[0xff, 0xd9]));
    }

    let camera_pid = camera.child.id() as i32;
    let status = camera.stop(CAMERA_SHUTDOWN_TIMEOUT);
    assert!(status.success(), "camera exited unsuccessfully: {status}");
    wait_for_process_group_empty(camera.process_group, camera_pid, CLEANUP_TIMEOUT);
    assert_listener_closes("HTTP", http_address);
    assert_listener_closes("RTSP", rtsp_address);
    assert_recorded_processes_stopped(&ffmpeg_pids);
    assert_recorded_processes_stopped(&ffprobe_pids);
}

fn camera_fixture(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    assert!(
        fs::metadata(&path).is_ok_and(|metadata| metadata.is_file()),
        "missing camera fixture: {}",
        path.display()
    );
    path
}

fn start_camera(
    http_address: SocketAddr,
    rtsp_address: SocketAddr,
    fixture: &Path,
) -> ProcessGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_camera"));
    command
        .args(["--address", &http_address.to_string()])
        .args(["--rtsp-address", &rtsp_address.to_string()])
        .args([
            "--video",
            fixture.to_str().expect("fixture path should be UTF-8"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut camera = ProcessGuard::spawn("camera", &mut command);
    wait_for_health(&mut camera, http_address);
    camera
}

fn create_recordings_root(directory: &Path) -> PathBuf {
    let root = directory.join("recordings");
    fs::create_dir_all(root.join("camera-1")).expect("create camera recording directory");
    root
}

fn recorder_settings() -> RecorderSettings {
    RecorderSettings {
        io_timeout: Duration::from_secs(10),
        retry_delay: Duration::from_secs(1),
        stop_timeout: Duration::from_secs(5),
    }
}

fn instrumented_executable(directory: &Path, name: &str) -> (PathBuf, PathBuf) {
    let wrapper = directory.join(format!("{name}-wrapper"));
    let pids = wrapper.with_extension("pids");
    fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ \"$#\" -ne 1 ] || [ \"$1\" != \"-version\" ]; then\n  printf '%s\\n' \"$$\" >> \"$0.pids\"\nfi\nexec {name} \"$@\"\n"
        ),
    )
    .expect("write instrumented executable wrapper");
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755))
        .expect("make instrumented wrapper executable");
    (wrapper, pids)
}

fn wait_for_recorder_status(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
    expected: RecorderStatus,
) {
    let timeout = Duration::from_secs(30);
    let deadline = Instant::now() + timeout;
    loop {
        match events.try_recv() {
            Ok(RecorderEvent::Status {
                camera_id: 1,
                status,
                ..
            }) if status == expected => return,
            Ok(RecorderEvent::Faulted { message, .. }) => {
                panic!("unexpected recorder fault: {message}")
            }
            Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("recorder event channel disconnected")
            }
        }
        assert!(
            Instant::now() < deadline,
            "recorder did not report {expected:?} within {timeout:?}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_finalized_files_match_segments(camera_directory: &Path, segments: &[RecordingSegment]) {
    for segment in segments {
        let metadata =
            fs::symlink_metadata(&segment.path).expect("read finalized segment metadata");
        assert!(
            metadata.file_type().is_file(),
            "returned segment is not a direct regular file: {}",
            segment.path.display()
        );
        assert_eq!(
            segment
                .path
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("mkv"),
            "returned segment does not use the .mkv extension: {}",
            segment.path.display()
        );
        let file_name = segment
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("finalized segment filename should be UTF-8");
        assert!(
            !file_name.ends_with(".partial.mkv"),
            "returned segment is a diagnostic partial: {}",
            segment.path.display()
        );
    }

    let mut finalized = fs::read_dir(camera_directory)
        .expect("read camera output directory")
        .filter_map(|entry| {
            let path = entry.expect("read camera output entry").path();
            let metadata = fs::symlink_metadata(&path).expect("read camera output metadata");
            (metadata.file_type().is_file()
                && path.extension().and_then(|extension| extension.to_str()) == Some("mkv")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with(".partial.mkv")))
            .then_some(path)
        })
        .collect::<Vec<_>>();
    finalized.sort_by_key(|path| {
        path.file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<i64>().ok())
            .expect("finalized segment should have a UTC millisecond filename")
    });
    assert_eq!(
        finalized,
        segments
            .iter()
            .map(|segment| segment.path.clone())
            .collect::<Vec<_>>(),
        "camera directory finalized files differ from returned segments"
    );
}

fn assert_finalized_matroska_h264(path: &Path) {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-count_packets"])
        .args([
            "-show_entries",
            "stream=codec_name,nb_read_packets:format=format_name",
        ])
        .args(["-of", "default=noprint_wrappers=1"])
        .arg(path)
        .output()
        .expect("start FFprobe for finalized segment");
    let stdout = String::from_utf8(output.stdout).expect("FFprobe stdout should be UTF-8");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "FFprobe failed for {}: {}\n{stderr}",
        path.display(),
        output.status
    );
    assert_eq!(entry(&stdout, "codec_name"), Some("h264"));
    assert!(
        entry(&stdout, "format_name")
            .is_some_and(|formats| formats.split(',').any(|format| format == "matroska")),
        "FFprobe did not report Matroska for {}: {stdout}",
        path.display()
    );
    let packets = entry(&stdout, "nb_read_packets")
        .and_then(|packets| packets.parse::<u64>().ok())
        .unwrap_or_default();
    assert!(packets > 0, "no H.264 packets in {}", path.display());
}

fn assert_recorded_processes_stopped(pids: &Path) {
    let pids = fs::read_to_string(pids).expect("read recorded child process IDs");
    let pids = pids
        .lines()
        .map(|pid| pid.parse::<i32>().expect("parse recorded child process ID"))
        .collect::<Vec<_>>();
    assert!(
        !pids.is_empty(),
        "no instrumented child process was observed"
    );

    for pid in pids {
        let deadline = Instant::now() + CLEANUP_TIMEOUT;
        loop {
            match signal(pid, SIGNAL_EXISTS) {
                Err(error) if error.raw_os_error() == Some(ESRCH) => break,
                Err(error) => panic!("inspect recorder process {pid}: {error}"),
                Ok(()) => {}
            }
            assert!(
                Instant::now() < deadline,
                "recorder process {pid} remained alive after {CLEANUP_TIMEOUT:?}"
            );
            thread::sleep(POLL_INTERVAL);
        }
    }
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

fn wait_for_process_group_empty(process_group: i32, related_pid: i32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        match signal(-process_group, SIGNAL_EXISTS) {
            Err(error) if error.raw_os_error() == Some(ESRCH) => return,
            Err(error) => panic!("inspect camera process group: {error}"),
            Ok(()) => {}
        }
        assert!(
            Instant::now() < deadline,
            "camera process group remained alive after {timeout:?}; related PID {related_pid}"
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
