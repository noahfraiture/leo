#![cfg(unix)]

use std::{
    fs,
    num::NonZeroUsize,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use backend::{
    analysis::{
        AnalysisCheckpoint, AnalysisResponse, AnalysisWarning, ChecklistProgress, Observation,
    },
    recording::{RecorderRuntime, RecorderSettings, RecorderStatus, spawn_for_test},
    session::{SessionController, mark_recording_complete},
};
use dioxus::{
    dioxus_core::NoOpMutations,
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};
use serde_json::json;
use uuid::Uuid;

use crate::{
    App, Bootstrap, InitialSettings, Route, RuntimeAvailability,
    preview::{PreviewFeed, PreviewState},
    session_task::handle_recorder_event,
    settings::{
        CameraSettings, LogLevel, ResolvedSettings, Settings as ApplicationSettings, SettingsStore,
    },
    views::{Settings as SettingsView, SettingsContext, SettingsPageState},
    workflow::Workflow,
};

const START_UTC_MS: i64 = 1_786_552_800_000;

#[derive(Clone)]
struct RenderRootProps {
    workflow: Arc<Mutex<Option<Workflow>>>,
    preview: PreviewState,
    availability: RuntimeAvailability,
    path: String,
}

fn render_root(props: RenderRootProps) -> Element {
    let RenderRootProps {
        workflow: initial,
        preview,
        availability,
        path,
    } = props;
    let workflow = use_signal(move || {
        initial
            .lock()
            .expect("render workflow mutex should not be poisoned")
            .take()
            .expect("render root should take Workflow once")
    });
    let settings =
        use_signal(|| SettingsPageState::new(ApplicationSettings::default(), None, None, None));
    use_context_provider(|| workflow);
    use_context_provider(move || preview);
    use_context_provider(move || SettingsContext {
        state: settings,
        store: None,
    });
    use_context_provider(move || availability);

    rsx! {
        HistoryProvider {
            history: move |_| Rc::new(MemoryHistory::with_initial_path(path.clone())) as Rc<dyn History>,
            Router::<Route> {}
        }
    }
}

#[derive(Clone)]
struct WorkflowRootProps {
    workflow: Arc<Mutex<Option<Workflow>>>,
}

fn render_monitor_sidebar_root(props: WorkflowRootProps) -> Element {
    let initial = props.workflow;
    let workflow = use_signal(move || {
        initial
            .lock()
            .expect("render workflow mutex should not be poisoned")
            .take()
            .expect("render root should take Workflow once")
    });
    use_context_provider(|| workflow);
    rsx! { crate::views::monitor::Sidebar {} }
}

#[derive(Clone)]
struct SettingsRenderRootProps {
    state: Arc<Mutex<Option<SettingsPageState>>>,
    store: Option<SettingsStore>,
}

fn render_settings_root(props: SettingsRenderRootProps) -> Element {
    let SettingsRenderRootProps { state, store } = props;
    let state = use_signal(move || {
        state
            .lock()
            .expect("render settings mutex should not be poisoned")
            .take()
            .expect("render settings root should take state once")
    });
    use_context_provider(move || SettingsContext { state, store });
    rsx! { SettingsView {} }
}

#[derive(Clone)]
struct SettingsRouterRootProps {
    state: Arc<Mutex<Option<SettingsPageState>>>,
    store: Option<SettingsStore>,
    availability: RuntimeAvailability,
}

fn render_settings_router_root(props: SettingsRouterRootProps) -> Element {
    let SettingsRouterRootProps {
        state,
        store,
        availability,
    } = props;
    let state = use_signal(move || {
        state
            .lock()
            .expect("render settings mutex should not be poisoned")
            .take()
            .expect("render settings root should take state once")
    });
    use_context_provider(move || SettingsContext { state, store });
    use_context_provider(move || availability);
    rsx! {
        HistoryProvider {
            history: move |_| Rc::new(MemoryHistory::with_initial_path("/settings")) as Rc<dyn History>,
            Router::<Route> {}
        }
    }
}

fn render_settings(state: SettingsPageState, store: Option<SettingsStore>) -> String {
    let props = SettingsRenderRootProps {
        state: Arc::new(Mutex::new(Some(state))),
        store,
    };
    let mut dom = VirtualDom::new_with_props(render_settings_root, props);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn render_settings_route(
    state: SettingsPageState,
    store: Option<SettingsStore>,
    availability: RuntimeAvailability,
) -> String {
    let props = SettingsRouterRootProps {
        state: Arc::new(Mutex::new(Some(state))),
        store,
        availability,
    };
    let mut dom = VirtualDom::new_with_props(render_settings_router_root, props);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn render_unconfigured(route: Route) -> String {
    render_setup(route, None)
}

fn render_setup(route: Route, load_error: Option<String>) -> String {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let store = SettingsStore::new(
        temporary.path().join("config/settings.json"),
        temporary.path().join("data"),
    )
    .expect("render settings store should be valid");
    render_app(
        Bootstrap::SetupRequired,
        InitialSettings {
            store: Some(store),
            draft: ApplicationSettings::default(),
            saved: None,
            active: None,
            load_error,
            initial_route: route,
        },
    )
}

fn render_loaded_failure(
    route: Route,
    message: &str,
    store: SettingsStore,
    resolved: ResolvedSettings,
) -> String {
    render_app(
        Bootstrap::Failed {
            message: message.into(),
        },
        InitialSettings {
            store: Some(store),
            draft: resolved.settings.clone(),
            saved: Some(resolved.clone()),
            active: Some(resolved),
            load_error: None,
            initial_route: route,
        },
    )
}

fn render_app(bootstrap: Bootstrap, initial_settings: InitialSettings) -> String {
    let mut dom = VirtualDom::new(App)
        .with_root_context(bootstrap)
        .with_root_context(initial_settings);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn settings_snapshot(
    root: &Path,
    name: &str,
    model: &str,
    log_level: LogLevel,
    camera_count: u32,
) -> (SettingsStore, ResolvedSettings) {
    let store = SettingsStore::new(
        root.join(format!("{name}-settings.json")),
        root.join(format!("{name}-default-data")),
    )
    .expect("render settings store should be valid");
    let mut settings = ApplicationSettings::default();
    settings.data_root = Some(root.join(format!("{name}-data")));
    settings.openai.api_key = format!("{name}-secret-key");
    settings.openai.model = model.into();
    settings.openai.base_url = Some(format!("https://{name}.provider.example/v1"));
    settings.log_level = log_level;
    for _ in 0..camera_count {
        let id = settings
            .add_camera()
            .expect("render settings camera should be added");
        settings.cameras.last_mut().unwrap().rtsp_url =
            format!("rtsp://{name}-camera-{id}.example/stream");
    }
    let resolved = store
        .resolve(settings)
        .expect("render settings should resolve");
    (store, resolved)
}

struct Harness {
    _temporary: tempfile::TempDir,
    runtime: Option<RecorderRuntime>,
    workflow: Option<Workflow>,
}

impl Harness {
    fn new() -> Self {
        Self::with_cameras(camera_settings())
    }

    fn with_cameras(cameras: Vec<CameraSettings>) -> Self {
        let temporary = tempfile::tempdir().expect("temporary render root should be created");
        let executable = temporary.path().join("successful-preflight");
        fs::write(&executable, "#!/bin/sh\nexit 0\n")
            .expect("fake preflight executable should be written");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("fake preflight executable should be executable");
        let (runtime, recorder, _events) = spawn_for_test(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            executable.clone(),
            executable,
        )
        .expect("test recorder runtime should start");
        let workflow = Workflow::new(
            cameras,
            temporary.path().join("sessions"),
            recorder,
            Some(crate::test_openai_config()),
            NonZeroUsize::new(5).unwrap(),
            0,
        )
        .expect("render Workflow should initialize");

        Self {
            _temporary: temporary,
            runtime: Some(runtime),
            workflow: Some(workflow),
        }
    }

    fn workflow(&self) -> &Workflow {
        self.workflow.as_ref().expect("Workflow should be retained")
    }

    fn workflow_mut(&mut self) -> &mut Workflow {
        self.workflow.as_mut().expect("Workflow should be retained")
    }

    fn start(&mut self) -> PathBuf {
        self.workflow_mut()
            .begin_start(START_UTC_MS)
            .expect("idle render state should begin starting")
            .directory
    }

    fn activate(&mut self) -> PathBuf {
        let request = self
            .workflow_mut()
            .begin_start(START_UTC_MS)
            .expect("idle render state should begin starting");
        let directory = request.directory.clone();
        let controller = SessionController::create(request.events_path, request.session_cameras)
            .expect("active render controller should be created");
        self.workflow_mut()
            .finish_start(directory.clone(), controller);
        directory
    }

    fn render(self, preview: PreviewState) -> String {
        self.render_at(preview, "/")
    }

    fn render_at(mut self, preview: PreviewState, path: &str) -> String {
        let camera_count = self.workflow().cameras.len();
        let props = RenderRootProps {
            workflow: Arc::new(Mutex::new(self.workflow.take())),
            preview,
            availability: RuntimeAvailability::Ready { camera_count },
            path: path.into(),
        };
        let mut dom = VirtualDom::new_with_props(render_root, props);
        dom.rebuild(&mut NoOpMutations);
        let html = dioxus_ssr::render(&dom);
        drop(dom);
        self.runtime
            .take()
            .expect("render runtime should be retained")
            .shutdown()
            .expect("render runtime should shut down");
        html
    }

    fn render_monitor_sidebar(mut self) -> String {
        let props = WorkflowRootProps {
            workflow: Arc::new(Mutex::new(self.workflow.take())),
        };
        let mut dom = VirtualDom::new_with_props(render_monitor_sidebar_root, props);
        dom.rebuild(&mut NoOpMutations);
        let html = dioxus_ssr::render(&dom);
        drop(dom);
        self.runtime
            .take()
            .expect("render runtime should be retained")
            .shutdown()
            .expect("render runtime should shut down");
        html
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.shutdown();
        }
    }
}

fn render_ready_with_cameras(route: Route, cameras: Vec<CameraSettings>) -> String {
    let preview = if cameras.is_empty() {
        PreviewState::NoCameras
    } else {
        ready_preview()
    };
    Harness::with_cameras(cameras).render_at(preview, &route.to_string())
}

fn camera_settings() -> Vec<CameraSettings> {
    vec![
        CameraSettings {
            id: 17,
            name: "Salon 1".into(),
            rtsp_url: "rtsp://camera-one.example/live".into(),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        },
        CameraSettings {
            id: 42,
            name: "Salon 2".into(),
            rtsp_url: "rtsp://camera-two.example/live".into(),
            initially_included_in_analysis: false,
            sample_every_ms: 2_000,
        },
    ]
}

fn ready_preview() -> PreviewState {
    PreviewState::Ready {
        feeds: vec![
            PreviewFeed {
                camera_id: 17,
                name: "Salon 1".into(),
                video_id: "camera-0-video".into(),
                whep_url: "http://127.0.0.1:8889/camera-0/whep".into(),
            },
            PreviewFeed {
                camera_id: 42,
                name: "Salon 2".into(),
                video_id: "camera-1-video".into(),
                whep_url: "http://127.0.0.1:8889/camera-1/whep".into(),
            },
        ],
        script_url: "http://127.0.0.1:8889/reader.js".into(),
    }
}

fn unavailable_preview() -> PreviewState {
    PreviewState::Unavailable {
        message: "preview bridge did not start".into(),
    }
}

fn write_completed_session(
    root: &Path,
    name: &str,
    session_id: Uuid,
    start_utc_ms: i64,
    end_offset_ms: u64,
) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("recordings"))
        .expect("render session directories should be created");
    let events = [
        json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started",
                "cameras": [
                    {
                        "camera_id": 17,
                        "name": "Salon 1",
                        "enabled": true,
                        "sample_every_ms": 1_000
                    },
                    {
                        "camera_id": 42,
                        "name": "Salon 2",
                        "enabled": false,
                        "sample_every_ms": 2_000
                    }
                ]
            }
        }),
        json!({
            "schema_version": 1,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + i64::try_from(end_offset_ms).unwrap(),
            "session_offset_ms": end_offset_ms,
            "action": { "type": "session_ended" }
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("render event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events)
        .expect("render session events should be written");
    mark_recording_complete(&directory).expect("render session should be marked complete");
    directory
}

fn response(
    timestamp: &str,
    description: &str,
    summary: &str,
    status: &str,
    note: &str,
) -> AnalysisResponse {
    AnalysisResponse {
        observations: vec![Observation {
            timestamp: timestamp.into(),
            description: description.into(),
        }],
        sequence_summary: summary.into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Complete the exercise".into(),
            status: status.into(),
            note: note.into(),
        }],
    }
}

fn checkpoint(
    session_id: Uuid,
    total_batches: usize,
    warnings: Vec<AnalysisWarning>,
    responses: Vec<AnalysisResponse>,
) -> AnalysisCheckpoint {
    AnalysisCheckpoint {
        schema_version: 2,
        session_id,
        checklist: "Persisted correct-sequence checklist".into(),
        plan_fingerprint: "0123456789abcdef".into(),
        total_batches,
        warnings,
        responses,
    }
}

fn write_checkpoint(directory: &Path, checkpoint: &AnalysisCheckpoint) {
    fs::write(
        directory.join("analysis.json"),
        serde_json::to_vec_pretty(checkpoint).expect("render checkpoint should serialize"),
    )
    .expect("render checkpoint should be written");
}

fn prepare_session(
    harness: &mut Harness,
    session_id: Uuid,
    checkpoint: Option<&AnalysisCheckpoint>,
) -> PathBuf {
    let directory = write_completed_session(
        &harness.workflow().session_root,
        &format!("session-{session_id}"),
        session_id,
        START_UTC_MS,
        4_000,
    );
    if let Some(checkpoint) = checkpoint {
        write_checkpoint(&directory, checkpoint);
    }
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");
    harness.workflow_mut().selected_session_id = Some(session_id);
    directory
}

fn opening_tag_before<'a>(html: &'a str, element: &str, text: &str) -> &'a str {
    let text_index = html
        .find(text)
        .unwrap_or_else(|| panic!("expected rendered text {text:?} in {html}"));
    let start = html[..text_index]
        .rfind(&format!("<{element}"))
        .unwrap_or_else(|| panic!("expected <{element}> before {text:?} in {html}"));
    let end = html[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .expect("opening element should end");
    &html[start..end]
}

fn assert_button_disabled(html: &str, text: &str, disabled: bool) {
    let button = opening_tag_before(html, "button", text);
    assert_eq!(
        button.contains("disabled"),
        disabled,
        "unexpected disabled state for {text:?}: {button}"
    );
}

fn opening_tag_with_marker<'a>(html: &'a str, element: &str, marker: &str) -> &'a str {
    let marker_index = html
        .find(marker)
        .unwrap_or_else(|| panic!("expected marker {marker:?} in {html}"));
    let start = html[..marker_index]
        .rfind(&format!("<{element}"))
        .unwrap_or_else(|| panic!("expected <{element}> containing {marker:?} in {html}"));
    let end = html[start..]
        .find('>')
        .map(|offset| start + offset + 1)
        .expect("opening element should end");
    &html[start..end]
}

fn section_with_heading<'a>(html: &'a str, heading: &str) -> &'a str {
    let heading_index = html
        .find(&format!(">{heading}<"))
        .unwrap_or_else(|| panic!("expected heading {heading:?} in {html}"));
    let start = html[..heading_index]
        .rfind("<section")
        .unwrap_or_else(|| panic!("expected section before {heading:?} in {html}"));
    let end = html[heading_index..]
        .find("</section>")
        .map(|offset| heading_index + offset + "</section>".len())
        .expect("diagnostic section should end");
    &html[start..end]
}

fn assert_analysis_action(html: &str, label: &str, disabled: bool) {
    let button = opening_tag_with_marker(html, "button", r#"id="analysis-action""#);
    assert_eq!(
        button.contains("disabled"),
        disabled,
        "unexpected analysis action state: {button}"
    );
    assert!(html.contains(&format!(">{label}</button>")), "{html}");
}

fn assert_row_status(html: &str, session_id: Uuid, status: &str) {
    let button = opening_tag_with_marker(
        html,
        "button",
        &format!("Session {session_id}, UTC milliseconds:"),
    );
    assert!(
        button.contains(&format!(", status: {status}\"")),
        "missing {status:?} row for {session_id}: {button}"
    );
}

fn assert_progress(html: &str, completed: usize, total: usize) {
    let marker = format!(r#"aria-label="Analysis progress: {completed} of {total} batches""#);
    let progress = opening_tag_with_marker(html, "progress", &marker);
    assert!(
        progress.contains(&format!(r#"value="{completed}""#)),
        "{progress}"
    );
    assert!(
        progress.contains(&format!(r#"max="{total}""#)),
        "{progress}"
    );
}

fn assert_stable_previews(html: &str) {
    assert_eq!(html.matches("<video").count(), 2, "{html}");
    assert_eq!(html.matches("camera-0-video").count(), 1, "{html}");
    assert_eq!(html.matches("camera-1-video").count(), 1, "{html}");
    assert_eq!(html.matches("<script").count(), 1, "{html}");
    assert_eq!(html.matches("reader.js").count(), 1, "{html}");
    assert!(html.contains("defer"), "{html}");
    assert_eq!(
        html.matches(r#"aria-label="Selected Salon "#).count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches(r#"aria-label="Select Salon "#).count(),
        1,
        "{html}"
    );
    assert_eq!(html.matches("aria-pressed=true").count(), 1, "{html}");
    let selected_button = opening_tag_before(html, "button", ">Selected</button>");
    assert!(
        selected_button.contains(r#"aria-label="Selected Salon "#),
        "{selected_button}"
    );
    assert_eq!(
        html.matches("aria-pressed=true>Selected</button>").count(),
        1,
        "{html}"
    );
}

fn assert_no_fake_claims(html: &str) {
    for fake in ["LIVE", "14:42:18", "CAM 04", "Camera options"] {
        assert!(!html.contains(fake), "found fake claim {fake:?} in {html}");
    }
}

#[test]
fn missing_settings_render_the_shell_and_open_settings() {
    let html = render_unconfigured(Route::Settings {});
    assert!(html.contains("Settings"));
    assert!(html.contains("Application settings"));
    assert!(html.contains("Configure Leo, save, then restart"));
    assert!(!html.contains("Start session"));
}

#[test]
fn unavailable_monitor_renders_guidance_without_workflow_context() {
    let html = render_unconfigured(Route::Monitor {});
    assert!(html.contains("Monitor is unavailable"));
    assert!(html.contains("Settings"));
}

#[test]
fn invalid_settings_render_the_shell_and_recovery_form() {
    let message = "Application settings could not be loaded. Check the settings file and configured data directories.";
    let html = render_setup(Route::Settings {}, Some(message.into()));

    assert!(html.contains("Application settings"), "{html}");
    assert!(html.contains(message), "{html}");
    assert!(html.contains("Save settings"), "{html}");
    assert!(!html.contains("Start session"), "{html}");
}

#[test]
fn zero_camera_runtime_hides_start_and_keeps_analyze_available() {
    let html = render_ready_with_cameras(Route::Monitor {}, Vec::new());
    assert!(html.contains("No cameras are configured"));
    assert!(!html.contains("Start session"));
    let analyze = render_ready_with_cameras(Route::Analyze {}, Vec::new());
    assert!(analyze.contains("Completed sessions"));
}

#[test]
fn primary_navigation_wraps_and_links_all_routes() {
    let html = Harness::new().render(ready_preview());
    let navigation = opening_tag_with_marker(&html, "nav", r#"id="navbutton""#);

    assert!(navigation.contains("flex-wrap"), "{navigation}");
    for label in ["Monitor", "Analyze", "Settings"] {
        assert!(
            opening_tag_before(&html, "a", label).contains("href="),
            "{html}"
        );
    }
}

#[test]
fn monitor_sidebar_with_no_cameras_never_renders_start() {
    let html = Harness::with_cameras(Vec::new()).render_monitor_sidebar();

    assert!(html.contains("No cameras are configured"), "{html}");
    assert!(!html.contains("Start session"), "{html}");
}

#[test]
fn ready_settings_route_does_not_require_operational_contexts() {
    let state = SettingsPageState::new(ApplicationSettings::default(), None, None, None);

    let html = render_settings_route(state, None, RuntimeAvailability::Ready { camera_count: 2 });

    assert!(html.contains("Application settings"), "{html}");
    assert!(html.contains("Settings"), "{html}");
}

#[test]
fn loaded_runtime_failure_keeps_settings_and_saved_active_snapshots() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let (store, resolved) = settings_snapshot(
        temporary.path(),
        "loaded",
        "loaded-model",
        LogLevel::Info,
        1,
    );
    let monitor = render_loaded_failure(
        Route::Monitor {},
        "Recorder preflight failed.",
        store.clone(),
        resolved.clone(),
    );
    let analyze = render_loaded_failure(
        Route::Analyze {},
        "Recorder preflight failed.",
        store.clone(),
        resolved.clone(),
    );

    for (html, route_name) in [(&monitor, "Monitor"), (&analyze, "Analyze")] {
        assert!(
            html.contains(&format!("{route_name} is unavailable")),
            "{html}"
        );
        assert!(html.contains("Recorder preflight failed."), "{html}");
        assert!(html.contains("Settings"), "{html}");
        assert!(!html.contains("loaded-secret-key"), "{html}");
        assert!(!html.contains("rtsp://"), "{html}");
    }

    let settings = render_loaded_failure(
        Route::Settings {},
        "Recorder preflight failed.",
        store,
        resolved,
    );
    assert!(settings.contains("Application settings"), "{settings}");
    assert!(settings.contains("Saved on disk"), "{settings}");
    assert!(settings.contains("Active at startup"), "{settings}");
    assert!(settings.contains("loaded-model"), "{settings}");
}

#[test]
fn settings_form_renders_all_sections_and_diagnostics() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let store = SettingsStore::new(
        temporary.path().join("config/settings.json"),
        temporary.path().join("default-data"),
    )
    .expect("render settings store should be valid");
    let settings_path = store.settings_path.display().to_string();
    let default_data_root = store.default_data_root.display().to_string();
    let mut settings = ApplicationSettings::default();
    settings
        .add_camera()
        .expect("render settings camera should be added");
    settings.cameras[0].rtsp_url = "rtsp://render-camera.example/stream".into();
    settings.openai.api_key = "render-secret-key".into();
    let state = SettingsPageState::new(settings, None, None, None);

    let html = render_settings(state, Some(store));

    assert!(html.contains("Application settings"), "{html}");
    assert!(html.contains("Cameras"), "{html}");
    assert!(html.contains("Storage"), "{html}");
    assert!(html.contains("Recording"), "{html}");
    assert!(html.contains("Analysis provider"), "{html}");
    assert!(html.contains("Diagnostics"), "{html}");
    assert!(html.contains("type=\"password\""), "{html}");
    assert!(html.contains("webkitdirectory"), "{html}");
    assert!(html.contains("Save settings"), "{html}");
    assert!(html.contains(&settings_path), "{html}");
    assert!(html.contains(&default_data_root), "{html}");
    assert!(!html.contains("Move up"), "{html}");
    assert!(!html.contains("Move down"), "{html}");
    assert_eq!(html.matches("render-secret-key").count(), 1, "{html}");
    assert_eq!(
        html.matches("rtsp://render-camera.example/stream").count(),
        1,
        "{html}"
    );
    assert!(
        opening_tag_with_marker(&html, "input", r#"id="settings-openai-key""#)
            .contains("render-secret-key"),
        "{html}"
    );
    assert!(
        opening_tag_with_marker(&html, "input", r#"id="camera-1-rtsp-url""#)
            .contains("rtsp://render-camera.example/stream"),
        "{html}"
    );
}

#[test]
fn settings_form_renders_analysis_batching_controls_and_help() {
    let state = SettingsPageState::new(ApplicationSettings::default(), None, None, None);

    let html = render_settings(state, None);

    let frame_sets =
        opening_tag_with_marker(&html, "input", r#"id="settings-analysis-frame-sets""#);
    assert!(frame_sets.contains(r#"type="number""#), "{frame_sets}");
    assert!(frame_sets.contains(r#"min="1""#), "{frame_sets}");
    assert!(frame_sets.contains(r#"step="1""#), "{frame_sets}");
    assert!(frame_sets.contains(r#"value="5""#), "{frame_sets}");

    let overlap = opening_tag_with_marker(&html, "input", r#"id="settings-analysis-overlap""#);
    assert!(overlap.contains(r#"type="number""#), "{overlap}");
    assert!(overlap.contains(r#"min="0""#), "{overlap}");
    assert!(overlap.contains(r#"step="1""#), "{overlap}");
    assert!(overlap.contains(r#"value="0""#), "{overlap}");
    assert!(
        html.contains("Each frame set can contain one image per camera."),
        "{html}"
    );
    assert!(
        html.contains("Overlap repeats images and may increase provider cost."),
        "{html}"
    );
}

#[test]
fn settings_overlap_error_is_accessibly_described() {
    let mut state = SettingsPageState::new(ApplicationSettings::default(), None, None, None);
    state.draft.analysis_overlap_frame_sets = "5".into();
    state.field_errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("invalid overlap should have a field error"),
    };

    let html = render_settings(state, None);

    let overlap = opening_tag_with_marker(&html, "input", r#"id="settings-analysis-overlap""#);
    assert!(overlap.contains(r#"aria-invalid="true""#), "{overlap}");
    assert!(
        overlap.contains(r#"aria-describedby="settings-analysis-overlap-error""#),
        "{overlap}"
    );
    assert!(
        html.contains(r#"id="settings-analysis-overlap-error""#),
        "{html}"
    );
    assert!(
        html.contains("smaller than frame sets per prompt"),
        "{html}"
    );
}

#[test]
fn settings_form_field_errors_have_accessible_descriptions() {
    let mut state = SettingsPageState::new(ApplicationSettings::default(), None, None, None);
    state
        .add_camera()
        .expect("render settings camera should be added");
    state.draft.cameras[0].rtsp_url = "http://wrong".into();
    state.field_errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("invalid render settings should have field errors"),
    };

    let html = render_settings(state, None);

    let input = opening_tag_with_marker(&html, "input", r#"id="camera-1-rtsp-url""#);
    assert!(input.contains(r#"aria-invalid="true""#), "{input}");
    assert!(
        input.contains(r#"aria-describedby="camera-1-rtsp-url-error""#),
        "{input}"
    );
    assert!(html.contains(r#"id="camera-1-rtsp-url-error""#), "{html}");
    assert!(html.contains("Enter a valid RTSP URL."), "{html}");
}

#[test]
fn settings_camera_list_marks_every_camera_with_an_error() {
    let mut state = SettingsPageState::new(ApplicationSettings::default(), None, None, None);
    for _ in 0..3 {
        let id = state
            .add_camera()
            .expect("render settings camera should be added");
        state.draft.cameras[usize::try_from(id - 1).unwrap()].rtsp_url =
            format!("rtsp://camera-{id}.example/stream");
    }
    state.draft.cameras[1].name.clear();
    state.draft.cameras[2].rtsp_url = "http://wrong".into();
    state.selected_camera_id = Some(1);
    state.field_errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("invalid render settings should have field errors"),
    };

    let html = render_settings(state, None);

    assert_eq!(html.matches("Needs attention").count(), 2, "{html}");
    for (camera_id, expected) in [(1, false), (2, true), (3, true)] {
        let marker = format!("Camera ID {camera_id}");
        let marker_index = html
            .find(&marker)
            .unwrap_or_else(|| panic!("expected {marker:?} in {html}"));
        let button_start = html[..marker_index]
            .rfind("<button")
            .expect("camera marker should be inside a button");
        let button_end = html[marker_index..]
            .find("</button>")
            .map(|offset| marker_index + offset)
            .expect("camera button should end");
        assert_eq!(
            html[button_start..button_end].contains("Needs attention"),
            expected,
            "unexpected camera error marker for ID {camera_id}: {html}"
        );
    }
}

#[test]
fn settings_diagnostics_distinguish_draft_saved_and_active_snapshots() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let (draft_store, draft) =
        settings_snapshot(temporary.path(), "draft", "draft-model", LogLevel::Debug, 1);
    let (_, saved) = settings_snapshot(temporary.path(), "saved", "saved-model", LogLevel::Warn, 2);
    let (_, active) = settings_snapshot(
        temporary.path(),
        "active",
        "active-model",
        LogLevel::Trace,
        3,
    );
    let state = SettingsPageState::new(
        draft.settings.clone(),
        Some(saved.clone()),
        Some(active.clone()),
        None,
    );

    let html = render_settings(state, Some(draft_store));

    for (heading, resolved, model, log_level, camera_ids) in [
        ("Draft", &draft, "draft-model", "debug", "1"),
        ("Saved on disk", &saved, "saved-model", "warn", "1, 2"),
        (
            "Active at startup",
            &active,
            "active-model",
            "trace",
            "1, 2, 3",
        ),
    ] {
        let summary = section_with_heading(&html, heading);
        for path in [
            &resolved.settings_path,
            &resolved.data_root,
            &resolved.sessions_root,
            &resolved.logs_root,
        ] {
            assert!(summary.contains(&path.display().to_string()), "{summary}");
        }
        assert!(summary.contains(model), "{summary}");
        assert!(
            summary.contains(resolved.settings.openai.base_url.as_deref().unwrap()),
            "{summary}"
        );
        assert!(summary.contains("Configured"), "{summary}");
        assert!(summary.contains(log_level), "{summary}");
        assert!(summary.contains(camera_ids), "{summary}");
        assert!(summary.contains("10 seconds"), "{summary}");
        assert!(!summary.contains("secret-key"), "{summary}");
        assert!(!summary.contains("rtsp://"), "{summary}");
    }
    assert_eq!(html.matches("draft-secret-key").count(), 1, "{html}");
    for hidden in [
        "saved-secret-key",
        "active-secret-key",
        "rtsp://saved-camera-1.example/stream",
        "rtsp://active-camera-1.example/stream",
    ] {
        assert!(
            !html.contains(hidden),
            "found protected value {hidden:?} in {html}"
        );
    }
}

#[test]
fn settings_diagnostics_include_draft_saved_and_active_batching_values() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let store = SettingsStore::new(
        temporary.path().join("settings.json"),
        temporary.path().join("default-data"),
    )
    .expect("render settings store should be valid");
    let resolved = |frame_sets, overlap| {
        store
            .resolve(ApplicationSettings {
                analysis_frame_sets_per_prompt: frame_sets,
                analysis_overlap_frame_sets: overlap,
                ..ApplicationSettings::default()
            })
            .expect("batching settings should resolve")
    };
    let draft = resolved(71, 11);
    let saved = resolved(72, 12);
    let active = resolved(73, 13);
    let state = SettingsPageState::new(
        draft.settings,
        Some(saved.clone()),
        Some(active.clone()),
        None,
    );

    let html = render_settings(state, Some(store));

    for (heading, frame_sets, overlap) in [
        ("Draft", 71, 11),
        ("Saved on disk", 72, 12),
        ("Active at startup", 73, 13),
    ] {
        let summary = section_with_heading(&html, heading);
        assert!(summary.contains("Frame sets per prompt"), "{summary}");
        assert!(summary.contains("Overlapping frame sets"), "{summary}");
        assert!(summary.contains(&format!(">{frame_sets}<")), "{summary}");
        assert!(summary.contains(&format!(">{overlap}<")), "{summary}");
    }
}

#[test]
fn idle_renders_start_selection_cadence_root_and_no_fake_claims() {
    let harness = Harness::new();
    let session_root = harness.workflow().session_root.display().to_string();

    let html = harness.render(ready_preview());

    assert!(html.contains("Session idle"), "{html}");
    assert!(html.contains(r#"role="status""#), "{html}");
    assert!(html.contains(r#"aria-live="polite""#), "{html}");
    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains("Selected camera"), "{html}");
    assert!(html.contains("Salon 1"), "{html}");
    assert!(
        html.contains("Initial sampling interval: 1 second"),
        "{html}"
    );
    assert!(html.contains(&session_root), "{html}");
    assert!(html.contains("<nav"), "{html}");
    assert!(
        html.contains(r#"aria-label="Primary navigation""#),
        "{html}"
    );
    assert!(
        opening_tag_before(&html, "a", "Monitor").contains(r#"aria-current="page""#),
        "{html}"
    );
    assert_stable_previews(&html);
    assert_no_fake_claims(&html);
}

#[test]
fn no_cameras_renders_configuration_guidance() {
    let html = Harness::with_cameras(Vec::new()).render(PreviewState::NoCameras);

    assert!(html.contains("No cameras are configured"), "{html}");
}

#[test]
fn starting_renders_per_camera_readiness_and_disables_start_without_cancel() {
    let mut harness = Harness::new();
    harness.start();
    harness.workflow_mut().cameras[0].recorder_status = RecorderStatus::Recording;

    let html = harness.render(ready_preview());

    assert!(html.contains("Starting session"), "{html}");
    assert_button_disabled(&html, "Start session", true);
    assert!(
        html.contains(r#"aria-label="Salon 1 recorder status: Recording""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Salon 2 recorder status: Starting""#),
        "{html}"
    );
    assert!(!html.contains("Cancel"), "{html}");
    assert!(html.contains("Staging directory"), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn active_recording_keeps_excluded_preview_mounted_and_enables_controls() {
    let mut harness = Harness::new();
    let directory = harness.activate();
    harness
        .workflow_mut()
        .select_camera(42)
        .expect("excluded camera should be selected");

    let html = harness.render(ready_preview());

    assert!(html.contains("Session active"), "{html}");
    assert!(html.contains("Elapsed time:"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_button_disabled(&html, "Stop session", false);
    assert!(
        html.contains(r#"aria-label="Analysis participation: Excluded""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Recorder status: Recording""#),
        "{html}"
    );
    assert!(html.contains("Include in analysis"), "{html}");
    assert!(html.contains("Sampling interval (seconds)"), "{html}");
    assert!(html.contains(r#"type="number""#), "{html}");
    assert!(html.contains(r#"min="1""#), "{html}");
    assert!(html.contains(r#"step="1""#), "{html}");
    assert!(html.contains("Apply cadence"), "{html}");
    assert_stable_previews(&html);
    assert_no_fake_claims(&html);
}

#[test]
fn active_reconnecting_keeps_both_previews_and_health_separate_from_participation() {
    let mut harness = Harness::new();
    harness.activate();
    harness.workflow_mut().cameras[1].recorder_status = RecorderStatus::Reconnecting;

    let html = harness.render(ready_preview());

    assert!(
        html.contains(r#"aria-label="Analysis participation: Excluded""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Recorder status: Reconnecting""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Salon 2 recorder status: Reconnecting""#),
        "{html}"
    );
    assert_button_disabled(&html, "Stop session", false);
    assert_stable_previews(&html);
}

#[test]
fn stopping_disables_session_controls_and_reports_finalization() {
    let mut harness = Harness::new();
    let directory = harness.activate();
    let _request = harness
        .workflow_mut()
        .begin_stop()
        .expect("active render state should begin stopping");

    let html = harness.render(ready_preview());

    assert!(html.contains("Finalizing session"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert!(html.contains("Session directory"), "{html}");
    assert_button_disabled(&html, "Stop session", true);
    assert!(!html.contains("Apply cadence"), "{html}");
    assert!(!html.contains("Include in analysis"), "{html}");
    assert!(!html.contains("Exclude from analysis"), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn recorder_fault_renders_one_shared_alert_and_nonassertive_guidance() {
    let mut harness = Harness::new();
    let directory = harness.activate();
    let _request = handle_recorder_event(
        harness.workflow_mut(),
        backend::recording::RecorderEvent::Faulted {
            camera_id: Some(42),
            message: "Camera 2 recorder storage failed".into(),
        },
    )
    .expect("active recorder fault should claim cleanup");

    let html = harness.render(ready_preview());

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(
        html.matches(r#"aria-live="assertive""#).count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches("Camera 2 recorder storage failed").count(),
        1,
        "{html}"
    );
    assert!(html.contains(r#"class="alert alert-error m-2""#), "{html}");
    assert!(html.contains("Recorder cleanup was attempted"), "{html}");
    assert!(html.contains("restart Leo"), "{html}");
    assert!(html.contains("inspect"), "{html}");
    assert!(html.contains("Faulted session directory"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_eq!(
        html.matches(r#"aria-label="Recorder status: Idle""#)
            .count(),
        1,
        "{html}"
    );
    assert!(!html.contains("Apply cadence"), "{html}");
    assert!(!html.contains("Include in analysis"), "{html}");
    assert!(!html.contains("Exclude from analysis"), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn stop_cleanup_fault_updates_shared_alert_and_sets_all_recorder_health_idle() {
    let mut harness = Harness::new();
    let directory = harness.activate();
    let _request = harness
        .workflow_mut()
        .begin_stop()
        .expect("active render state should begin stopping");
    harness.workflow_mut().finish_fault(
        directory.clone(),
        "Recorder Stop failed during cleanup".into(),
    );

    let html = harness.render(ready_preview());

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(
        html.matches("Recorder Stop failed during cleanup").count(),
        1,
        "{html}"
    );
    assert!(html.contains(r#"class="alert alert-error m-2""#), "{html}");
    assert_eq!(
        html.matches(r#"aria-label="Recorder status: Idle""#)
            .count(),
        2,
        "{html}"
    );
    assert!(html.contains("Faulted session directory"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
}

#[test]
fn analyze_without_sessions_renders_refresh_and_an_empty_state() {
    let harness = Harness::new();

    let html = harness.render_at(ready_preview(), "/analyze");

    assert!(html.contains("Completed sessions"), "{html}");
    assert_button_disabled(&html, "Refresh sessions", false);
    assert!(html.contains("No completed sessions found."), "{html}");
    assert!(
        html.contains("Select a completed session to analyze."),
        "{html}"
    );
    assert!(!html.contains("analysis-action"), "{html}");
    assert!(!html.contains("Analyze body"), "{html}");
}

#[test]
fn analyze_lists_newest_first_and_renders_the_selected_session_recap() {
    let mut harness = Harness::new();
    let root = harness.workflow().session_root.clone();
    let older_id = Uuid::from_u128(101);
    let newer_id = Uuid::from_u128(102);
    write_completed_session(&root, "older", older_id, START_UTC_MS, 2_000);
    let newer_directory =
        write_completed_session(&root, "newer", newer_id, START_UTC_MS + 10_000, 4_000);
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert!(html.contains("lg:w-80"), "{html}");
    assert!(
        html.contains(&format!(
            r#"aria-label="Session {older_id}, UTC milliseconds: {START_UTC_MS}, status: Not started""#
        )),
        "{html}"
    );
    assert!(
        html.contains(&format!(
            r#"aria-label="Session {newer_id}, UTC milliseconds: {}, status: Not started""#,
            START_UTC_MS + 10_000
        )),
        "{html}"
    );
    assert_row_status(&html, older_id, "Not started");
    assert_row_status(&html, newer_id, "Not started");
    let newer_row = html
        .find(&format!(
            "Session {newer_id}, UTC milliseconds: {}, status: Not started",
            START_UTC_MS + 10_000
        ))
        .expect("newer session row should render");
    let older_row = html
        .find(&format!(
            "Session {older_id}, UTC milliseconds: {START_UTC_MS}, status: Not started"
        ))
        .expect("older session row should render");
    assert!(
        newer_row < older_row,
        "sessions must render newest first: {html}"
    );
    assert!(
        opening_tag_with_marker(
            &html,
            "button",
            &format!(
                "Session {newer_id}, UTC milliseconds: {}, status: Not started",
                START_UTC_MS + 10_000
            )
        )
        .contains("aria-pressed=true"),
        "{html}"
    );
    assert!(html.contains(&newer_id.to_string()), "{html}");
    assert!(
        html.contains(&format!("UTC milliseconds: {}", START_UTC_MS + 10_000)),
        "{html}"
    );
    assert!(html.contains("Duration: 4000 ms"), "{html}");
    assert!(html.contains("Camera count: 2"), "{html}");
    assert!(
        html.contains(&newer_directory.display().to_string()),
        "{html}"
    );
    for (name, path) in [
        ("events.jsonl", newer_directory.join("events.jsonl")),
        ("recordings/", newer_directory.join("recordings")),
        (
            "recording-complete",
            newer_directory.join("recording-complete"),
        ),
        ("analysis.json", newer_directory.join("analysis.json")),
    ] {
        assert!(html.contains(name), "missing {name:?} in {html}");
        assert!(html.contains(&path.display().to_string()), "{html}");
    }
    let textarea = opening_tag_with_marker(&html, "textarea", r#"id="analysis-checklist""#);
    assert!(!textarea.contains("readonly"), "{textarea}");
    assert_analysis_action(&html, "Analyze", false);
    assert!(!html.contains("session_started"), "{html}");
    assert_eq!(html.matches("<video").count(), 0, "{html}");
}

#[test]
fn analyze_renders_invalid_checkpoint_without_enabling_replacement() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(103);
    let directory = prepare_session(&mut harness, session_id, None);
    fs::write(directory.join("analysis.json"), b"not JSON")
        .expect("invalid render checkpoint should be written");
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("invalid checkpoint should remain a row result");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Invalid checkpoint");
    assert!(html.contains("Invalid analysis checkpoint"), "{html}");
    assert!(
        html.contains("analysis checkpoint is not valid JSON"),
        "{html}"
    );
    assert!(html.contains("analysis.json"), "{html}");
    assert_analysis_action(&html, "Analyze", true);
    assert!(!html.contains("not JSON"), "{html}");
}

#[test]
fn analyze_disables_new_analysis_when_model_configuration_is_missing() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(104);
    prepare_session(&mut harness, session_id, None);
    harness.workflow_mut().model_config_error =
        Some("Analysis requires an OpenAI API key and model in Settings.".into());

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Not started");
    assert!(
        html.contains("Analysis requires an OpenAI API key and model in Settings."),
        "{html}"
    );
    assert_analysis_action(&html, "Analyze", true);
}

#[test]
fn analyze_treats_a_zero_response_checkpoint_as_in_progress() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(105);
    let saved = checkpoint(session_id, 2, Vec::new(), Vec::new());
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "In progress");
    let textarea = opening_tag_with_marker(&html, "textarea", r#"id="analysis-checklist""#);
    assert!(textarea.contains("readonly"), "{textarea}");
    assert!(
        html.contains("Persisted correct-sequence checklist"),
        "{html}"
    );
    assert_progress(&html, 0, 2);
    assert!(html.contains("No completed batches yet."), "{html}");
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn analyze_renders_partial_observations_in_order_and_latest_cumulative_state() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(106);
    let saved = checkpoint(
        session_id,
        3,
        Vec::new(),
        vec![
            response(
                "00:00:01.000",
                "First visible movement",
                "Old sequence summary",
                "old status",
                "Old checklist note",
            ),
            response(
                "00:00:02.000",
                "Second visible movement",
                "Latest sequence summary",
                "respected",
                "Latest checklist note",
            ),
        ],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "In progress");
    assert_progress(&html, 2, 3);
    let first = html
        .find("First visible movement")
        .expect("first observation should render");
    let second = html
        .find("Second visible movement")
        .expect("second observation should render");
    assert!(
        first < second,
        "observations must retain response order: {html}"
    );
    assert!(html.contains("00:00:01.000"), "{html}");
    assert!(html.contains("00:00:02.000"), "{html}");
    assert!(html.contains("Latest sequence summary"), "{html}");
    assert!(html.contains("respected"), "{html}");
    assert!(html.contains("Latest checklist note"), "{html}");
    assert!(!html.contains("Old sequence summary"), "{html}");
    assert!(!html.contains("old status"), "{html}");
    assert!(!html.contains("Old checklist note"), "{html}");
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn analyze_renders_complete_results_and_disables_resume() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(107);
    let saved = checkpoint(
        session_id,
        2,
        Vec::new(),
        vec![
            response(
                "00:00:01.000",
                "Exercise started",
                "Exercise in progress",
                "not yet",
                "Waiting for completion",
            ),
            response(
                "00:00:03.000",
                "Exercise completed",
                "Exercise completed correctly",
                "respected",
                "All steps were visible",
            ),
        ],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Complete");
    assert_progress(&html, 2, 2);
    assert!(html.contains("Exercise started"), "{html}");
    assert!(html.contains("Exercise completed"), "{html}");
    assert!(html.contains("Exercise completed correctly"), "{html}");
    assert!(html.contains("All steps were visible"), "{html}");
    assert!(!html.contains("Exercise in progress"), "{html}");
    assert!(!html.contains("Waiting for completion"), "{html}");
    assert_analysis_action(&html, "Resume", true);
}

#[test]
fn analyze_renders_every_recording_gap_before_complete_model_results() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(108);
    let saved = checkpoint(
        session_id,
        1,
        vec![
            AnalysisWarning::RecordingGap {
                camera_id: 17,
                start_offset_ms: 500,
                end_offset_ms: 1_000,
            },
            AnalysisWarning::RecordingGap {
                camera_id: 42,
                start_offset_ms: 2_000,
                end_offset_ms: 2_500,
            },
        ],
        vec![response(
            "00:00:03.000",
            "Visible result after gaps",
            "Latest warning summary",
            "uncertain",
            "Recording gaps limit confidence",
        )],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Complete with warning");
    assert_progress(&html, 1, 1);
    let first_gap = "Recording gap: camera 17, 500 ms to 1000 ms";
    let second_gap = "Recording gap: camera 42, 2000 ms to 2500 ms";
    assert!(html.contains(first_gap), "{html}");
    assert!(html.contains(second_gap), "{html}");
    let first_gap_index = html.find(first_gap).unwrap();
    let second_gap_index = html.find(second_gap).unwrap();
    let result_index = html.find("Visible result after gaps").unwrap();
    assert!(first_gap_index < result_index, "{html}");
    assert!(second_gap_index < result_index, "{html}");
    assert_analysis_action(&html, "Resume", true);
}

#[test]
fn analyze_running_state_disables_only_the_analysis_action_not_navigation() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(109);
    prepare_session(&mut harness, session_id, None);
    harness.workflow_mut().running_analysis_id = Some(session_id);
    harness.workflow_mut().analysis_error = Some((session_id, "stale failure".into()));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Running");
    assert_analysis_action(&html, "Analyze", true);
    assert!(html.contains("Analysis is running."), "{html}");
    assert!(!html.contains("stale failure"), "{html}");
    let monitor_link = opening_tag_before(&html, "a", "Monitor");
    let analyze_link = opening_tag_before(&html, "a", "Analyze");
    assert!(!monitor_link.contains("disabled"), "{monitor_link}");
    assert!(!analyze_link.contains("disabled"), "{analyze_link}");
    assert!(analyze_link.contains(r#"aria-current="page""#), "{html}");
}

#[test]
fn analyze_failed_state_shows_the_session_failure_and_allows_resume() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(110);
    let saved = checkpoint(
        session_id,
        2,
        Vec::new(),
        vec![response(
            "00:00:01.000",
            "Saved observation",
            "Saved partial summary",
            "not yet",
            "One batch remains",
        )],
    );
    prepare_session(&mut harness, session_id, Some(&saved));
    harness.workflow_mut().analysis_error =
        Some((session_id, "The model request failed safely.".into()));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Failed");
    assert!(html.contains("The model request failed safely."), "{html}");
    assert_progress(&html, 1, 2);
    assert!(html.contains("Saved observation"), "{html}");
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn analyze_disables_the_action_while_a_recording_session_is_active() {
    let mut harness = Harness::new();
    let session_id = Uuid::from_u128(111);
    prepare_session(&mut harness, session_id, None);
    harness.activate();

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Not started");
    assert_analysis_action(&html, "Analyze", true);
    assert!(
        html.contains("Analysis is unavailable while recording is active."),
        "{html}"
    );
}

#[test]
fn recorder_fault_message_remains_visible_on_analyze_route() {
    let mut harness = Harness::new();
    harness.activate();
    let _request = handle_recorder_event(
        harness.workflow_mut(),
        backend::recording::RecorderEvent::Faulted {
            camera_id: None,
            message: "Recorder runtime failed".into(),
        },
    )
    .expect("global recorder fault should claim cleanup");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(html.matches("Recorder runtime failed").count(), 1, "{html}");
    assert!(html.contains("No completed sessions found."), "{html}");
    assert!(
        opening_tag_before(&html, "a", "Analyze").contains(r#"aria-current="page""#),
        "{html}"
    );
}

#[test]
fn preview_unavailable_keeps_idle_recording_control_and_actionable_alert() {
    let harness = Harness::new();

    let html = harness.render(unavailable_preview());

    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains(r#"role="alert""#), "{html}");
    assert!(html.contains("Live preview is unavailable"), "{html}");
    assert!(html.contains("preview bridge did not start"), "{html}");
    assert!(html.contains("restart the app"), "{html}");
    assert_eq!(
        html.matches(r#"aria-label="Selected Salon "#).count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches(r#"aria-label="Select Salon "#).count(),
        1,
        "{html}"
    );
    assert!(html.contains(">Salon 1 (Selected)</button>"), "{html}");
    assert!(html.contains(">Salon 2 (Select)</button>"), "{html}");
    assert_eq!(html.matches("<video").count(), 0, "{html}");
    assert_eq!(html.matches("reader.js").count(), 0, "{html}");
}

#[test]
fn shared_workflow_message_renders_once_above_routed_content() {
    let mut harness = Harness::new();
    harness.workflow_mut().message = Some("The requested action failed".into());

    let html = harness.render(ready_preview());

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(
        html.matches("The requested action failed").count(),
        1,
        "{html}"
    );
    let alert = html
        .find("The requested action failed")
        .expect("shared alert should render");
    let monitor = html.find("Camera monitor").expect("Monitor should render");
    assert!(
        alert < monitor,
        "shared alert must precede routed content: {html}"
    );
}
