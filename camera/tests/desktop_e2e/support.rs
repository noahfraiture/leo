//! Mechanical process, fixture, settings, and loopback-provider support for desktop scenarios.

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
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::{Value, json};
use tokio::sync::oneshot;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const CAMERA_READY_TIMEOUT: Duration = Duration::from_secs(20);
const DRIVER_READY_TIMEOUT: Duration = Duration::from_secs(15);
const E2E_TIMEOUT: Duration = Duration::from_secs(120);
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const ESRCH: i32 = 3;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

pub fn write_desktop_settings(
    directory: &Path,
    data_root: &Path,
    camera_addresses: [SocketAddr; 2],
    analysis_frame_sets_per_prompt: usize,
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
                "initialMonitoringProfileId": id
            })
        })
        .collect::<Vec<_>>();
    let settings = json!({
        "schemaVersion": 3,
        "nextCameraId": 3,
        "cameras": cameras,
        "dataRoot": data_root,
        "recorderTimeoutSecs": 10,
        "monitoringProfiles": [
            {"id": 1, "name": "Standard", "sampleEveryMs": 1000},
            {"id": 2, "name": "Stable", "sampleEveryMs": 2000}
        ],
        "nextMonitoringProfileId": 3,
        "analysisProfiles": [{
            "id": 1, "name": "Fixture", "model": model,
            "maxImagesPerPrompt": analysis_frame_sets_per_prompt * 2,
            "maxPromptSpanMs": (analysis_frame_sets_per_prompt.saturating_sub(1) * 1000).max(1),
            "overlapFrameSets": 0, "imageSize": "original", "imageDetail": "providerDefault", "maxOutputTokens": null
        }],
        "nextAnalysisProfileId": 2,
        "defaultAnalysisProfileId": 1,
        "openai": {
            "apiKey": api_key,
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

pub fn required_environment(name: &str) -> String {
    let value = env::var(name).unwrap_or_else(|_| panic!("real OpenAI E2E requires {name}"));
    assert!(!value.trim().is_empty(), "real OpenAI E2E requires {name}");
    value
}

pub fn fixture(name: &str) -> PathBuf {
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

pub fn real_openai_root() -> PathBuf {
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

pub fn start_camera(name: &'static str, fixture: &Path, logs: &Path) -> (SocketAddr, ProcessGuard) {
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

pub fn start_desktop_app(
    settings_path: &Path,
    driver_ready: &Path,
    driver_result: &Path,
    scenario: &str,
    uses_mock_provider: bool,
    logs: &Path,
) -> ProcessGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_desktop-e2e-app"));
    command
        .arg(settings_path)
        .env("LEO_DESKTOP_E2E_READY", driver_ready)
        .env("LEO_DESKTOP_E2E_RESULT", driver_result)
        .env("LEO_DESKTOP_E2E_SCENARIO", scenario);
    for variable in [
        "LEO_CAMERA_CONFIG",
        "LEO_DATA_DIR",
        "LEO_RECORDER_TIMEOUT_SECS",
        "RUST_LOG",
        "OPENAI_API_KEY",
        "ANALYSIS_MODEL",
        "OPENAI_BASE_URL",
    ] {
        command.env_remove(variable);
    }
    if uses_mock_provider {
        for variable in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(variable);
        }
        command.env("NO_PROXY", "*").env("no_proxy", "*");
    }
    ProcessGuard::spawn("desktop app", &mut command, logs)
}

pub fn wait_for_desktop_result(
    app: &mut ProcessGuard,
    driver_ready: &Path,
    driver_result: &Path,
) -> String {
    wait_for_file(
        app,
        driver_ready,
        DRIVER_READY_TIMEOUT,
        "desktop E2E driver",
    );
    wait_for_file(app, driver_result, E2E_TIMEOUT, "desktop E2E result");
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
    fs::read_to_string(driver_result).expect("read desktop E2E result")
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

pub fn wait_for_file(
    process: &mut ProcessGuard,
    path: &Path,
    timeout: Duration,
    description: &str,
) {
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

pub fn assert_preview_ports_available() {
    TcpListener::bind(("127.0.0.1", 8889)).expect("preview TCP port 8889 is already occupied");
    UdpSocket::bind(("127.0.0.1", 8189)).expect("preview UDP port 8189 is already occupied");
}

pub fn read_application_log(logs: &Path) -> String {
    fs::read_dir(logs)
        .expect("read application log directory")
        .map(|entry| fs::read_to_string(entry.expect("read log entry").path()).expect("read log"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn request_contains_camera_frame(request: &Value, camera_id: u32) -> bool {
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

pub fn redacted_requests(requests: &[Value]) -> String {
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

pub struct MockOpenAi {
    pub address: SocketAddr,
    pub requests: Arc<Mutex<Vec<Value>>>,
    pub failed_requests: Arc<AtomicUsize>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl MockOpenAi {
    pub fn start() -> Self {
        Self::start_with_failure(None)
    }

    pub fn fail_once_on_request(request_number: usize) -> Self {
        assert!(request_number > 0, "mock request numbers start at one");
        Self::start_with_failure(Some(request_number))
    }

    fn start_with_failure(fail_on_request: Option<usize>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind mock OpenAI server");
        listener
            .set_nonblocking(true)
            .expect("make mock OpenAI listener nonblocking");
        let address = listener.local_addr().expect("read mock OpenAI address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failed_requests = Arc::new(AtomicUsize::new(0));
        let state = MockOpenAiState {
            requests: Arc::clone(&requests),
            fail_on_request,
            failed_requests: Arc::clone(&failed_requests),
        };
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
            failed_requests,
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

#[derive(Clone)]
struct MockOpenAiState {
    requests: Arc<Mutex<Vec<Value>>>,
    fail_on_request: Option<usize>,
    failed_requests: Arc<AtomicUsize>,
}

async fn mock_response(
    State(state): State<MockOpenAiState>,
    Json(request): Json<Value>,
) -> Response {
    let request_number = {
        let mut requests = state.requests.lock().expect("mock requests mutex");
        requests.push(request);
        requests.len()
    };
    if state.fail_on_request == Some(request_number) {
        state.failed_requests.fetch_add(1, Ordering::SeqCst);
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": {
                    "message": "transient mock provider failure",
                    "type": "server_error"
                }
            })),
        )
            .into_response();
    }
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
    .into_response()
}

pub struct ProcessGuard {
    name: &'static str,
    child: Child,
    pub process_group: i32,
    status: Option<ExitStatus>,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl ProcessGuard {
    pub fn spawn(name: &'static str, command: &mut Command, logs: &Path) -> Self {
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

    pub fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }

    pub fn wait_until(&mut self, timeout: Duration) -> io::Result<Option<ExitStatus>> {
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

    pub fn stop(&mut self, timeout: Duration) -> ExitStatus {
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

    pub fn assert_process_group_exited(&self, timeout: Duration) {
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

    pub fn diagnostics(&self) -> String {
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

pub fn process_group_exists(process_group: i32) -> io::Result<bool> {
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
