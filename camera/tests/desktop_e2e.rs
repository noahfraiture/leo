#![cfg(all(unix, target_os = "macos"))]

use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{Json, Router, extract::State, routing::post};
use backend::{
    analysis::AnalysisCheckpoint,
    recording::list_segments,
    session::{OperatorAction, list_sessions},
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const CAMERA_READY_TIMEOUT: Duration = Duration::from_secs(20);
const DRIVER_READY_TIMEOUT: Duration = Duration::from_secs(15);
const E2E_TIMEOUT: Duration = Duration::from_secs(120);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const ESRCH: i32 = 3;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

#[test]
#[ignore = "requires a macOS GUI session, MediaMTX, FFmpeg, and FFprobe"]
fn desktop_operator_flow_records_two_cameras_and_analyzes() {
    let real_openai = env::var("LEO_E2E_REAL_OPENAI").as_deref() == Ok("1");
    let real_provider = if real_openai {
        assert_eq!(
            env::var("LEO_E2E_REAL_OPENAI").as_deref(),
            Ok("1"),
            "real OpenAI E2E requires LEO_E2E_REAL_OPENAI=1"
        );
        assert_eq!(
            env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
            Ok("1"),
            "real OpenAI E2E requires LEO_RUN_PAID_OPENAI_TEST=1"
        );
        Some((
            required_environment("OPENAI_API_KEY"),
            required_environment("ANALYSIS_MODEL"),
        ))
    } else {
        None
    };

    assert_preview_ports_available();
    let settings_directory = tempfile::tempdir().expect("create desktop E2E settings directory");
    let temporary = if real_openai {
        None
    } else {
        Some(tempfile::tempdir().expect("create desktop E2E root"))
    };
    let root = temporary
        .as_ref()
        .map(|temporary| temporary.path().to_owned())
        .unwrap_or_else(real_openai_root);
    if real_openai {
        eprintln!("real OpenAI E2E artifacts: {}", root.display());
    }
    let logs = root.join("process-logs");
    fs::create_dir(&logs).expect("create process log directory");
    let data_root = root.join("data");
    fs::create_dir(&data_root).expect("create E2E data root");

    let (camera_1_rtsp, mut camera_1) = start_camera("camera 1", &fixture("salon-1.mp4"), &logs);
    let (camera_2_rtsp, mut camera_2) = start_camera("camera 2", &fixture("salon-2.mp4"), &logs);

    let mock_openai = (!real_openai).then(MockOpenAi::start);
    let mock_base_url = mock_openai
        .as_ref()
        .map(|mock| format!("http://{}/v1", mock.address));
    let settings_path = if let Some((api_key, model)) = &real_provider {
        write_desktop_settings(
            settings_directory.path(),
            &data_root,
            [camera_1_rtsp, camera_2_rtsp],
            api_key,
            model,
            None,
        )
    } else {
        write_desktop_settings(
            settings_directory.path(),
            &data_root,
            [camera_1_rtsp, camera_2_rtsp],
            "local-e2e-key",
            "local-e2e-model",
            Some(
                mock_base_url
                    .as_deref()
                    .expect("mock mode should have a provider URL"),
            ),
        )
    };
    drop(real_provider);
    let driver_ready = root.join("driver-ready");
    let driver_result = root.join("driver-result");
    let mut app = Command::new(env!("CARGO_BIN_EXE_desktop-e2e-app"));
    app.arg(&settings_path)
        .env("LEO_DESKTOP_E2E_READY", &driver_ready)
        .env("LEO_DESKTOP_E2E_RESULT", &driver_result);
    for variable in [
        "LEO_CAMERA_CONFIG",
        "LEO_DATA_DIR",
        "LEO_RECORDER_TIMEOUT_SECS",
        "RUST_LOG",
        "OPENAI_API_KEY",
        "ANALYSIS_MODEL",
        "OPENAI_BASE_URL",
    ] {
        app.env_remove(variable);
    }
    if mock_openai.is_some() {
        for variable in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            app.env_remove(variable);
        }
        app.env("NO_PROXY", "*").env("no_proxy", "*");
    }
    let mut app = ProcessGuard::spawn("desktop app", &mut app, &logs);

    wait_for_file(
        &mut app,
        &driver_ready,
        DRIVER_READY_TIMEOUT,
        "desktop E2E driver",
    );
    wait_for_file(&mut app, &driver_result, E2E_TIMEOUT, "desktop E2E result");
    let status = app
        .wait_until(SHUTDOWN_TIMEOUT)
        .expect("wait for desktop app")
        .unwrap_or_else(|| panic!("desktop app did not exit\n{}", app.diagnostics()));
    assert!(
        status.success(),
        "desktop app exited unsuccessfully: {status}\n{}",
        app.diagnostics()
    );
    app.assert_process_group_exited(SHUTDOWN_TIMEOUT);
    let result = fs::read_to_string(&driver_result).expect("read desktop E2E result");
    let rendered_summary = result
        .strip_prefix("ok\n")
        .unwrap_or_else(|| panic!("{result}\n{}", app.diagnostics()))
        .trim();
    assert!(!rendered_summary.is_empty(), "rendered summary was empty");

    let camera_1_status = camera_1.stop(SHUTDOWN_TIMEOUT);
    let camera_2_status = camera_2.stop(SHUTDOWN_TIMEOUT);
    assert!(camera_1_status.success(), "camera 1: {camera_1_status}");
    assert!(camera_2_status.success(), "camera 2: {camera_2_status}");
    camera_1.assert_process_group_exited(SHUTDOWN_TIMEOUT);
    camera_2.assert_process_group_exited(SHUTDOWN_TIMEOUT);

    let sessions = list_sessions(&data_root.join("sessions")).expect("list E2E sessions");
    assert_eq!(sessions.len(), 1, "expected one completed E2E session");
    let stored = &sessions[0];
    assert!(stored.session.actions.iter().any(|(_, action)| {
        matches!(
            action,
            OperatorAction::SetSamplingInterval {
                camera_id: 1,
                sample_every
            } if *sample_every == Duration::from_secs(2)
        )
    }));

    let marker = fs::symlink_metadata(stored.directory.join("recording-complete"))
        .expect("read E2E completion marker");
    assert!(marker.file_type().is_file());
    assert_eq!(marker.len(), 0);

    let segments = list_segments(&stored.directory.join("recordings"), &[1, 2])
        .expect("discover E2E recording segments");
    assert!(segments.iter().any(|segment| segment.camera_id == 1));
    assert!(segments.iter().any(|segment| segment.camera_id == 2));

    let checkpoint =
        AnalysisCheckpoint::read(&stored.directory.join("analysis.json"), stored.session.id)
            .expect("read E2E analysis checkpoint");
    assert!(checkpoint.total_batches > 0);
    assert_eq!(checkpoint.responses.len(), checkpoint.total_batches);
    if let Some(mock) = &mock_openai {
        assert_eq!(rendered_summary, "E2E mock analysis complete.");
        assert!(
            checkpoint
                .responses
                .iter()
                .all(|response| response.sequence_summary == "E2E mock analysis complete.")
        );
        let requests = mock.requests.lock().expect("mock requests mutex");
        assert!(
            !requests.is_empty(),
            "mock OpenAI server received no requests"
        );
        for camera_id in [1, 2] {
            assert!(
                requests
                    .iter()
                    .any(|request| request_contains_camera_frame(request, camera_id)),
                "mock requests contained no JPEG frame for camera {camera_id}:\n{}",
                redacted_requests(&requests),
            );
        }
    }

    let application_log = read_application_log(&data_root.join("logs"));
    assert!(
        !application_log.contains("A Copy Value created"),
        "Dioxus reported a signal ownership violation:\n{application_log}"
    );
    assert!(application_log.contains("recorder runtime stopped"));
    assert!(application_log.contains("preview stopped"));
    assert!(!application_log.contains("recorder runtime shutdown failed"));
    assert!(!application_log.contains("preview stop failed"));
}

#[test]
fn desktop_settings_file_is_strict_private_and_complete() {
    let settings_directory = tempfile::tempdir().expect("create settings directory");
    let data_directory = tempfile::tempdir().expect("create data directory");
    let data_root = data_directory.path().join("data");
    fs::create_dir(&data_root).expect("create data root");
    let camera_addresses = [
        "127.0.0.1:8554".parse().expect("parse camera 1 address"),
        "127.0.0.1:8555".parse().expect("parse camera 2 address"),
    ];
    let mock_base_url = "http://127.0.0.1:3000/v1";

    let path = write_desktop_settings(
        settings_directory.path(),
        &data_root,
        camera_addresses,
        "local-e2e-key",
        "local-e2e-model",
        Some(mock_base_url),
    );

    let bytes = fs::read(&path).expect("read desktop settings");
    assert!(bytes.ends_with(b"\n"), "settings should end with a newline");
    let settings: Value = serde_json::from_slice(&bytes).expect("parse desktop settings");
    let expected = json!({
        "schemaVersion": 2,
        "nextCameraId": 3,
        "cameras": [
            {
                "id": 1,
                "name": "Salon 1",
                "rtspUrl": format!("rtsp://{}/axis-media/media.amp", camera_addresses[0]),
                "initiallyIncludedInAnalysis": true,
                "sampleEveryMs": 1_000
            },
            {
                "id": 2,
                "name": "Salon 2",
                "rtspUrl": format!("rtsp://{}/axis-media/media.amp", camera_addresses[1]),
                "initiallyIncludedInAnalysis": true,
                "sampleEveryMs": 1_000
            }
        ],
        "dataRoot": data_root,
        "recorderTimeoutSecs": 10,
        "analysisFrameSetsPerPrompt": 5,
        "analysisOverlapFrameSets": 0,
        "openai": {
            "apiKey": "local-e2e-key",
            "model": "local-e2e-model",
            "baseUrl": mock_base_url
        },
        "logLevel": "info"
    });
    assert!(
        settings == expected,
        "settings should match the complete strict E2E schema"
    );
    assert!(
        settings
            .get("dataRoot")
            .and_then(Value::as_str)
            .is_some_and(|path| Path::new(path).is_absolute() && Path::new(path) == data_root),
        "settings should contain the absolute E2E data root"
    );

    let mode = fs::metadata(path)
        .expect("read settings metadata")
        .permissions()
        .mode()
        & 0o777;
    assert!(mode == 0o600, "settings should have mode 0o600");
}

#[test]
fn process_group_probe_detects_a_live_descendant_after_the_leader_exits() {
    let temporary = tempfile::tempdir().expect("create process group test root");
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & exit 0"]);
    let mut process = ProcessGuard::spawn("process group probe", &mut command, temporary.path());
    let status = process
        .wait_until(Duration::from_secs(2))
        .expect("wait for process group leader")
        .expect("process group leader should exit");

    assert!(status.success());
    assert!(
        process_group_exists(process.process_group).expect("probe process group"),
        "background descendant should keep the process group alive"
    );
}

#[test]
fn camera_frame_request_requires_a_source_followed_by_a_jpeg() {
    let request = json!({
        "input": [
            {"content": [{"type": "input_text", "text": "Frame source: camera 1 (Front) at 00:00:00.000"}]},
            {"content": [{"type": "input_image", "image_url": "data:image/jpeg;base64,abc"}]},
            {"content": [{"type": "input_text", "text": "Frame source: camera 2 (Side) at 00:00:00.000"}]}
        ]
    });

    assert!(request_contains_camera_frame(&request, 1));
    assert!(!request_contains_camera_frame(&request, 2));
}

#[test]
fn request_diagnostics_redact_every_image_url() {
    let diagnostics = redacted_requests(&[json!({
        "input": [{
            "content": [{
                "type": "input_image",
                "image_url": "sensitive image without the expected prefix"
            }]
        }]
    })]);

    assert!(!diagnostics.contains("sensitive image"));
    assert!(diagnostics.contains("<redacted image>"));
}

fn write_desktop_settings(
    directory: &Path,
    data_root: &Path,
    camera_addresses: [SocketAddr; 2],
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
) -> PathBuf {
    assert!(
        directory.is_absolute(),
        "settings directory must be absolute"
    );
    assert!(
        data_root.is_absolute() && data_root.is_dir(),
        "data root must be an existing absolute directory"
    );
    let cameras = camera_addresses
        .into_iter()
        .enumerate()
        .map(|(index, address)| {
            let id = index + 1;
            json!({
                "id": id,
                "name": format!("Salon {id}"),
                "rtspUrl": format!("rtsp://{address}/axis-media/media.amp"),
                "initiallyIncludedInAnalysis": true,
                "sampleEveryMs": 1_000
            })
        })
        .collect::<Vec<_>>();
    let settings = json!({
        "schemaVersion": 2,
        "nextCameraId": 3,
        "cameras": cameras,
        "dataRoot": data_root,
        "recorderTimeoutSecs": 10,
        "analysisFrameSetsPerPrompt": 5,
        "analysisOverlapFrameSets": 0,
        "openai": {
            "apiKey": api_key,
            "model": model,
            "baseUrl": base_url
        },
        "logLevel": "info"
    });
    let path = directory.join("settings.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("create private desktop settings");
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .expect("set private desktop settings permissions");
    serde_json::to_writer_pretty(&mut file, &settings).expect("serialize desktop settings");
    file.write_all(b"\n").expect("finish desktop settings");
    path
}

fn required_environment(name: &str) -> String {
    let value = env::var(name).unwrap_or_else(|_| panic!("real OpenAI E2E requires {name}"));
    assert!(!value.trim().is_empty(), "real OpenAI E2E requires {name}");
    value
}

fn fixture(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    assert!(path.is_file(), "missing E2E fixture: {}", path.display());
    path
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("camera crate should have a workspace parent")
        .to_owned()
}

fn real_openai_root() -> PathBuf {
    let root = match env::var_os("LEO_E2E_OUTPUT_DIR") {
        Some(path) => {
            let path = PathBuf::from(path);
            assert!(path.is_absolute(), "LEO_E2E_OUTPUT_DIR must be absolute");
            path
        }
        None => {
            let utc_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should follow Unix epoch")
                .as_millis();
            workspace_root()
                .join("target/desktop-e2e-real")
                .join(utc_ms.to_string())
        }
    };
    assert!(
        !root.exists(),
        "real OpenAI E2E output already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create persistent real OpenAI E2E root");
    root
}

fn start_camera(name: &'static str, fixture: &Path, logs: &Path) -> (SocketAddr, ProcessGuard) {
    let (http_address, rtsp_address) = reserve_addresses();
    let mut command = Command::new(env!("CARGO_BIN_EXE_camera"));
    command
        .args(["--address", &http_address.to_string()])
        .args(["--rtsp-address", &rtsp_address.to_string()])
        .args([
            "--video",
            fixture.to_str().expect("fixture path should be UTF-8"),
        ]);
    for variable in ["OPENAI_API_KEY", "ANALYSIS_MODEL", "OPENAI_BASE_URL"] {
        command.env_remove(variable);
    }
    let mut camera = ProcessGuard::spawn(name, &mut command, logs);
    wait_for_health(&mut camera, http_address);
    (rtsp_address, camera)
}

fn reserve_addresses() -> (SocketAddr, SocketAddr) {
    let http = TcpListener::bind(("127.0.0.1", 0)).expect("reserve camera HTTP port");
    let rtsp = TcpListener::bind(("127.0.0.1", 0)).expect("reserve camera RTSP port");
    let http_address = http.local_addr().expect("read camera HTTP address");
    let rtsp_address = rtsp.local_addr().expect("read camera RTSP address");
    assert_ne!(http_address.port(), rtsp_address.port());
    (http_address, rtsp_address)
}

fn wait_for_health(camera: &mut ProcessGuard, address: SocketAddr) {
    let deadline = Instant::now() + CAMERA_READY_TIMEOUT;
    loop {
        if health_is_ok(address).unwrap_or(false) {
            return;
        }
        if let Some(status) = camera.poll().expect("poll camera during startup") {
            panic!(
                "{} exited before health was ready: {status}\n{}",
                camera.name,
                camera.diagnostics()
            );
        }
        assert!(
            Instant::now() < deadline,
            "{} health was not ready within {CAMERA_READY_TIMEOUT:?}\n{}",
            camera.name,
            camera.diagnostics()
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

fn wait_for_file(process: &mut ProcessGuard, path: &Path, timeout: Duration, description: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        if path.is_file() {
            return;
        }
        if let Some(status) = process.poll().expect("poll process while waiting for file") {
            panic!(
                "{} exited before {description}: {status}\n{}",
                process.name,
                process.diagnostics()
            );
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}\n{}",
            process.diagnostics()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_preview_ports_available() {
    TcpListener::bind(("127.0.0.1", 8889)).expect("preview TCP port 8889 is already occupied");
    UdpSocket::bind(("127.0.0.1", 8189)).expect("preview UDP port 8189 is already occupied");
}

fn read_application_log(logs: &Path) -> String {
    fs::read_dir(logs)
        .expect("read application log directory")
        .map(|entry| fs::read_to_string(entry.expect("read log entry").path()).expect("read log"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn request_contains_camera_frame(request: &Value, camera_id: u32) -> bool {
    let source = format!("Frame source: camera {camera_id} ");
    let Some(messages) = request.get("input").and_then(Value::as_array) else {
        return false;
    };
    let content = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .collect::<Vec<_>>();
    content.windows(2).any(|pair| {
        pair[0].get("type").and_then(Value::as_str) == Some("input_text")
            && pair[0]
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|text| text.contains(&source))
            && pair[1].get("type").and_then(Value::as_str) == Some("input_image")
            && pair[1]
                .get("image_url")
                .and_then(Value::as_str)
                .is_some_and(|url| url.starts_with("data:image/jpeg;base64,"))
    })
}

fn redacted_requests(requests: &[Value]) -> String {
    fn redact(value: &Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.iter().map(redact).collect()),
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| {
                        let value = if key == "image_url" {
                            Value::String("<redacted image>".into())
                        } else {
                            redact(value)
                        };
                        (key.clone(), value)
                    })
                    .collect(),
            ),
            value => value.clone(),
        }
    }

    serde_json::to_string_pretty(
        &requests
            .iter()
            .map(|request| redact(request.get("input").unwrap_or(&Value::Null)))
            .collect::<Vec<_>>(),
    )
    .expect("serialize redacted mock requests")
}

struct MockOpenAi {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<Value>>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl MockOpenAi {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock OpenAI server");
        listener
            .set_nonblocking(true)
            .expect("make mock OpenAI listener nonblocking");
        let address = listener.local_addr().expect("read mock OpenAI address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::clone(&requests);
        let (shutdown, stopped) = oneshot::channel();
        let thread = thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("start mock OpenAI runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener)
                    .expect("adopt mock OpenAI listener");
                let router = Router::new()
                    .route("/v1/responses", post(mock_response))
                    .with_state(state);
                axum::serve(listener, router)
                    .with_graceful_shutdown(async move {
                        let _ = stopped.await;
                    })
                    .await
                    .expect("serve mock OpenAI responses");
            });
        });
        Self {
            address,
            requests,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }
}

impl Drop for MockOpenAi {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn mock_response(
    State(requests): State<Arc<Mutex<Vec<Value>>>>,
    Json(request): Json<Value>,
) -> Json<Value> {
    requests.lock().expect("mock requests mutex").push(request);
    let response = json!({
        "observations": [{
            "timestamp": "00:00:00.000",
            "description": "The fixture student begins the exercise."
        }],
        "sequence_summary": "E2E mock analysis complete.",
        "checklist_progress": [{
            "item": "Keep movement controlled",
            "status": "respected",
            "note": "Deterministic local E2E response."
        }]
    });
    Json(json!({
        "id": "resp_e2e",
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "error": null,
        "incomplete_details": null,
        "instructions": null,
        "max_output_tokens": null,
        "model": "local-e2e-model",
        "usage": null,
        "output": [{
            "type": "message",
            "id": "msg_e2e",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "annotations": [],
                "text": serde_json::to_string(&response).expect("serialize E2E response")
            }]
        }],
        "tools": []
    }))
}

struct ProcessGuard {
    name: &'static str,
    child: Child,
    process_group: i32,
    status: Option<ExitStatus>,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl ProcessGuard {
    fn spawn(name: &'static str, command: &mut Command, logs: &Path) -> Self {
        let stem = name.replace(' ', "-");
        let stdout = logs.join(format!("{stem}.stdout"));
        let stderr = logs.join(format!("{stem}.stderr"));
        command
            .process_group(0)
            .stdout(Stdio::from(
                File::create(&stdout).expect("create process stdout"),
            ))
            .stderr(Stdio::from(
                File::create(&stderr).expect("create process stderr"),
            ));
        let child = command
            .spawn()
            .unwrap_or_else(|error| panic!("start {name}: {error}"));
        let process_group = child.id() as i32;
        Self {
            name,
            child,
            process_group,
            status: None,
            stdout,
            stderr,
        }
    }

    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
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

    fn stop(&mut self, timeout: Duration) -> ExitStatus {
        if let Some(status) = self.poll().expect("poll process before stop") {
            panic!(
                "{} exited before stop: {status}\n{}",
                self.name,
                self.diagnostics()
            );
        }
        signal(self.child.id() as i32, SIGINT).expect("interrupt process");
        self.wait_until(timeout)
            .expect("wait for process stop")
            .unwrap_or_else(|| panic!("{} did not stop\n{}", self.name, self.diagnostics()))
    }

    fn assert_process_group_exited(&self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            match process_group_exists(self.process_group) {
                Ok(false) => return,
                Ok(true) => {}
                Err(error) => panic!("probe {} process group: {error}", self.name),
            }
            assert!(
                Instant::now() < deadline,
                "{} left a child process running\n{}",
                self.name,
                self.diagnostics()
            );
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn diagnostics(&self) -> String {
        format!(
            "{} stdout:\n{}\n{} stderr:\n{}",
            self.name,
            fs::read_to_string(&self.stdout).unwrap_or_default(),
            self.name,
            fs::read_to_string(&self.stderr).unwrap_or_default()
        )
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        if self.poll().ok().flatten().is_none() {
            let _ = signal(self.child.id() as i32, SIGINT);
            let _ = self.wait_until(Duration::from_secs(2));
        }
        if process_group_exists(self.process_group).unwrap_or(true) {
            let _ = signal(-self.process_group, SIGKILL);
            if self.status.is_none() {
                let _ = self.wait_until(Duration::from_secs(5));
            }
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

fn process_group_exists(process_group: i32) -> io::Result<bool> {
    // SAFETY: signal 0 checks the process group without delivering a signal.
    if unsafe { kill(-process_group, 0) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ESRCH) {
        Ok(false)
    } else {
        Err(error)
    }
}
