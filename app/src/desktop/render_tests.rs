use std::{
    path::Path,
    rc::Rc,
    sync::{Arc, Mutex},
};

use dioxus::{
    dioxus_core::NoOpMutations,
    history::{History, MemoryHistory},
    prelude::*,
    router::components::HistoryProvider,
};

use super::{
    bootstrap::{Bootstrap, InitialSettings},
    shell::App,
};
use crate::{
    Route, RuntimeAvailability,
    preview::PreviewState,
    settings::{
        CameraSettings, LogLevel, OpenAiSettings, ResolvedSettings,
        Settings as ApplicationSettings, SettingsStore,
    },
    test_support::{
        RenderHarness, assert_button_disabled, opening_tag_before, opening_tag_with_marker,
        ready_preview,
    },
    views::{SettingsContext, SettingsPageState},
};

#[derive(Clone)]
struct SettingsRouterRootProps {
    state: Arc<Mutex<Option<SettingsPageState>>>,
    store: SettingsStore,
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

fn render_settings_route(
    state: SettingsPageState,
    store: Option<SettingsStore>,
    availability: RuntimeAvailability,
) -> String {
    let props = SettingsRouterRootProps {
        state: Arc::new(Mutex::new(Some(state))),
        store: store.unwrap_or_else(|| {
            let root = std::env::temp_dir().join("leo-desktop-render-settings");
            SettingsStore::new(root.join("settings.json"), root.join("data"))
        }),
        availability,
    };
    let mut dom = VirtualDom::new_with_props(render_settings_router_root, props);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn render_setup(route: Route) -> String {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let store = SettingsStore::new(
        temporary.path().join("config/settings.json"),
        temporary.path().join("data"),
    );
    render_app(
        Bootstrap::SetupRequired,
        InitialSettings {
            store,
            draft: ApplicationSettings::default(),
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
            store,
            draft: resolved.settings.clone(),
            initial_route: route,
        },
    )
}

fn render_app(bootstrap: Bootstrap, settings: InitialSettings) -> String {
    let mut dom = VirtualDom::new(App)
        .with_root_context(bootstrap)
        .with_root_context(settings);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn settings_snapshot(
    root: &Path,
    name: &str,
    model: &str,
    log_level: LogLevel,
) -> (SettingsStore, ResolvedSettings) {
    let store = SettingsStore::new(
        root.join(format!("{name}-settings.json")),
        root.join(format!("{name}-default-data")),
    );
    let settings = ApplicationSettings {
        next_camera_id: 2,
        cameras: vec![CameraSettings {
            id: 1,
            name: "Camera 1".into(),
            rtsp_url: format!("rtsp://{name}-camera.example/stream"),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        }],
        data_root: Some(root.join(format!("{name}-data"))),
        openai: OpenAiSettings {
            api_key: format!("{name}-secret-key"),
            model: model.into(),
            base_url: Some(format!("https://{name}.provider.example/v1")),
        },
        log_level,
        ..ApplicationSettings::default()
    };
    let resolved = store
        .resolve(settings)
        .expect("render settings should resolve");
    (store, resolved)
}

#[test]
fn setup_shell_opens_settings_without_operational_controls() {
    let html = render_setup(Route::Settings {});

    assert!(html.contains("Application settings"), "{html}");
    assert!(html.contains("Configure Leo, save, then restart"), "{html}");
    assert!(!html.contains("Start session"), "{html}");
}

#[test]
fn setup_shell_renders_unavailable_route_without_workflow_context() {
    let html = render_setup(Route::Monitor {});

    assert!(html.contains("Monitor is unavailable"), "{html}");
    assert!(html.contains("Settings"), "{html}");
}

#[test]
fn zero_camera_runtime_routes_monitor_to_guidance_and_keeps_analyze_available() {
    let monitor = RenderHarness::with_cameras(Vec::new()).render(PreviewState::NoCameras);
    assert!(monitor.contains("No cameras are configured"), "{monitor}");
    assert!(!monitor.contains("Start session"), "{monitor}");

    let analyze =
        RenderHarness::with_cameras(Vec::new()).render_at(PreviewState::NoCameras, "/analyze");
    assert!(analyze.contains("Completed sessions"), "{analyze}");
}

#[test]
fn primary_navigation_links_every_route_and_marks_the_current_one() {
    let html = RenderHarness::new().render(ready_preview());
    let navigation = opening_tag_with_marker(&html, "nav", r#"id="navbutton""#);

    assert!(navigation.contains("flex-wrap"), "{navigation}");
    for label in ["Monitor", "Analyze", "Settings"] {
        assert!(
            opening_tag_before(&html, "a", label).contains("href="),
            "{html}"
        );
    }
    assert!(
        opening_tag_before(&html, "a", "Monitor").contains(r#"aria-current="page""#),
        "{html}"
    );
}

#[test]
fn ready_settings_route_needs_no_operational_contexts() {
    let state = SettingsPageState::new(ApplicationSettings::default());

    let html = render_settings_route(state, None, RuntimeAvailability::Ready { camera_count: 2 });

    assert!(html.contains("Application settings"), "{html}");
    assert!(html.contains("Settings"), "{html}");
}

#[test]
fn active_recording_does_not_disable_settings_save() {
    let mut harness = RenderHarness::new();
    harness.activate();

    let html = harness.render_at(ready_preview(), "/settings");

    assert_button_disabled(&html, "Save settings", false);
}

#[test]
fn loaded_runtime_failure_keeps_route_guidance_and_settings_editable() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let (store, resolved) =
        settings_snapshot(temporary.path(), "loaded", "loaded-model", LogLevel::Info);

    for (route, route_name) in [
        (Route::Monitor {}, "Monitor"),
        (Route::Analyze {}, "Analyze"),
    ] {
        let html = render_loaded_failure(
            route,
            "Recorder preflight failed.",
            store.clone(),
            resolved.clone(),
        );
        assert!(
            html.contains(&format!("{route_name} is unavailable")),
            "{html}"
        );
        assert!(html.contains("Recorder preflight failed."), "{html}");
        assert!(!html.contains("loaded-secret-key"), "{html}");
        assert!(!html.contains("rtsp://"), "{html}");
    }

    let settings = render_loaded_failure(
        Route::Settings {},
        "Recorder preflight failed.",
        store,
        resolved,
    );
    assert!(settings.contains("loaded-model"), "{settings}");
}

#[test]
fn workflow_alert_is_shared_by_operational_routes_and_precedes_content() {
    for (path, content) in [("/", "Camera monitor"), ("/analyze", "Analyze sessions")] {
        let mut harness = RenderHarness::new();
        harness.workflow_mut().message = Some("The requested action failed".into());

        let html = harness.render_at(ready_preview(), path);

        assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
        assert_eq!(
            html.matches("The requested action failed").count(),
            1,
            "{html}"
        );
        assert!(
            html.find("The requested action failed").unwrap() < html.find(content).unwrap(),
            "shared alert must precede routed content: {html}"
        );
    }
}
