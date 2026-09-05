//! Shared server-rendering support for view-owning test modules.

use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use backend::{
    recording::{RecorderRuntime, RecorderSettings, test_support},
    session::SessionController,
};
use dioxus::{
    dioxus_core::NoOpMutations,
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};

use crate::{
    Route, RuntimeAvailability,
    operator::OperatorState,
    preview::{PreviewFeed, PreviewState},
    settings::{CameraSettings, Settings as ApplicationSettings, SettingsStore},
    views::{SettingsContext, SettingsPageState},
};

pub const START_UTC_MS: i64 = 1_786_552_800_000;

#[derive(Clone)]
struct RenderRootProps {
    operator: Arc<Mutex<Option<OperatorState>>>,
    preview: PreviewState,
    availability: RuntimeAvailability,
    settings_store: SettingsStore,
    path: String,
}

fn render_root(props: RenderRootProps) -> Element {
    let RenderRootProps {
        operator: initial,
        preview,
        availability,
        settings_store,
        path,
    } = props;
    let operator = use_signal(move || {
        initial
            .lock()
            .expect("render operator mutex should not be poisoned")
            .take()
            .expect("render root should take operator state once")
    });
    let settings = use_signal(|| SettingsPageState::new(ApplicationSettings::default()));
    use_context_provider(|| operator);
    use_context_provider(move || preview);
    use_context_provider(move || SettingsContext {
        state: settings,
        store: settings_store,
    });
    use_context_provider(move || availability);

    rsx! {
        HistoryProvider {
            history: move |_| Rc::new(MemoryHistory::with_initial_path(path.clone())) as Rc<dyn History>,
            Router::<Route> {}
        }
    }
}

/// Owns a test recorder and operator state while a route is prepared and rendered.
pub struct RenderHarness {
    temporary: tempfile::TempDir,
    runtime: Option<RecorderRuntime>,
    operator: Option<OperatorState>,
}

impl RenderHarness {
    pub fn new() -> Self {
        Self::with_cameras(camera_settings())
    }

    pub fn with_cameras(cameras: Vec<CameraSettings>) -> Self {
        let temporary = tempfile::tempdir().expect("temporary render root should be created");
        let (runtime, recorder, _events) = test_support::spawn(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            PathBuf::from("unused-test-ffmpeg"),
            PathBuf::from("unused-test-ffprobe"),
        )
        .expect("test recorder runtime should start");
        let operator = OperatorState::new(
            crate::test_settings(cameras, Some(crate::test_openai_config()), 5, 0),
            temporary.path().join("sessions"),
            recorder,
        )
        .expect("render operator state should initialize");

        Self {
            temporary,
            runtime: Some(runtime),
            operator: Some(operator),
        }
    }

    pub fn operator(&self) -> &OperatorState {
        self.operator
            .as_ref()
            .expect("operator state should be retained")
    }

    pub fn operator_mut(&mut self) -> &mut OperatorState {
        self.operator
            .as_mut()
            .expect("operator state should be retained")
    }

    pub fn activate(&mut self) -> std::path::PathBuf {
        let request = self
            .operator_mut()
            .begin_start(START_UTC_MS)
            .expect("idle render state should begin starting");
        let directory = request.directory.clone();
        let controller = SessionController::create(
            request.events_path,
            request.session_cameras,
            crate::test_monitoring_profiles(),
        )
        .expect("active render controller should be created");
        self.operator_mut()
            .finish_start(directory.clone(), Some(controller));
        directory
    }

    pub fn render(self, preview: PreviewState) -> String {
        self.render_at(preview, "/")
    }

    pub fn render_at(mut self, preview: PreviewState, path: &str) -> String {
        let camera_count = self.operator().cameras.len();
        let settings_store = SettingsStore::new(
            self.temporary.path().join("settings/settings.json"),
            self.temporary.path().join("settings-data"),
        );
        let props = RenderRootProps {
            operator: Arc::new(Mutex::new(self.operator.take())),
            preview,
            availability: RuntimeAvailability::Ready { camera_count },
            settings_store,
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
}

impl Drop for RenderHarness {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            let _ = runtime.shutdown();
        }
    }
}

pub fn camera_settings() -> Vec<CameraSettings> {
    vec![
        CameraSettings {
            id: 17,
            name: "Salon 1".into(),
            rtsp_url: "rtsp://camera-one.example/live".into(),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: 1,
        },
        CameraSettings {
            id: 42,
            name: "Salon 2".into(),
            rtsp_url: "rtsp://camera-two.example/live".into(),
            initially_included_in_analysis: false,
            initial_monitoring_profile_id: 2,
        },
    ]
}

pub fn ready_preview() -> PreviewState {
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

pub fn opening_tag_before<'a>(html: &'a str, element: &str, text: &str) -> &'a str {
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

pub fn opening_tag_with_marker<'a>(html: &'a str, element: &str, marker: &str) -> &'a str {
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

pub fn assert_button_disabled(html: &str, text: &str, disabled: bool) {
    let button = opening_tag_before(html, "button", text);
    assert_eq!(
        button.contains("disabled"),
        disabled,
        "unexpected disabled state for {text:?}: {button}"
    );
}
