use std::path::Path;

use dioxus::{dioxus_core::NoOpMutations, prelude::*};

use super::{
    bootstrap::{Bootstrap, InitialSettings},
    shell::App,
};
use crate::{
    Route,
    preview::PreviewState,
    settings::{
        CameraSettings, LogLevel, OpenAiSettings, ResolvedSettings,
        Settings as ApplicationSettings, SettingsStore,
    },
    test_support::{RenderHarness, opening_tag_before, opening_tag_with_marker, ready_preview},
};

fn render_app(bootstrap: Bootstrap, settings: InitialSettings) -> String {
    let mut dom = VirtualDom::new(App)
        .with_root_context(bootstrap)
        .with_root_context(settings);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn render_setup(route: Route) -> String {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    render_app(
        Bootstrap::SetupRequired,
        InitialSettings {
            store: SettingsStore::new(
                temporary.path().join("settings.json"),
                temporary.path().join("data"),
            ),
            draft: ApplicationSettings::default(),
            initial_route: route,
        },
    )
}

fn settings_snapshot(root: &Path) -> (SettingsStore, ResolvedSettings) {
    let store = SettingsStore::new(root.join("settings.json"), root.join("default-data"));
    let settings = ApplicationSettings {
        next_camera_id: 2,
        cameras: vec![CameraSettings {
            id: 1,
            name: "Camera 1".into(),
            rtsp_url: "rtsp://loaded-camera.example/stream".into(),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: 1,
        }],
        data_root: Some(root.join("loaded-data")),
        openai: OpenAiSettings {
            api_key: "loaded-secret-key".into(),
            base_url: Some("https://loaded.provider.example/v1".into()),
        },
        analysis_profiles: vec![backend::profiles::AnalysisProfile {
            model: "loaded-model".into(),
            ..ApplicationSettings::default().analysis_profiles.remove(0)
        }],
        log_level: LogLevel::Info,
        ..ApplicationSettings::default()
    };
    let resolved = store
        .resolve(settings)
        .expect("render settings should resolve");
    (store, resolved)
}

#[test]
fn setup_routes_render_without_operational_contexts() {
    let settings = render_setup(Route::Settings {});
    assert!(settings.contains("Application settings"), "{settings}");
    assert!(!settings.contains("Start session"), "{settings}");

    let monitor = render_setup(Route::Monitor {});
    assert!(monitor.contains("Monitor is unavailable"), "{monitor}");
    assert!(monitor.contains("Settings"), "{monitor}");
}

#[test]
fn zero_camera_runtime_keeps_analysis_available_without_recording() {
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

    for label in ["Monitor", "Analyze", "Settings"] {
        assert!(
            opening_tag_before(&html, "a", label).contains("href="),
            "{html}"
        );
    }
    assert!(
        opening_tag_before(&html, "a", "Monitor").contains(r#"aria-current="page""#),
        "{navigation}"
    );
}

#[test]
fn runtime_failure_keeps_guidance_and_saved_settings_available() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let (store, resolved) = settings_snapshot(temporary.path());

    for (route, name) in [
        (Route::Monitor {}, "Monitor"),
        (Route::Analyze {}, "Analyze"),
    ] {
        let html = render_app(
            Bootstrap::Failed {
                message: "Recorder preflight failed.".into(),
            },
            InitialSettings {
                store: store.clone(),
                draft: resolved.settings.clone(),
                initial_route: route,
            },
        );
        assert!(html.contains(&format!("{name} is unavailable")), "{html}");
        assert!(html.contains("Recorder preflight failed."), "{html}");
        assert!(!html.contains("loaded-secret-key"), "{html}");
        assert!(!html.contains("rtsp://"), "{html}");
    }

    let settings = render_app(
        Bootstrap::Failed {
            message: "Recorder preflight failed.".into(),
        },
        InitialSettings {
            store,
            draft: resolved.settings,
            initial_route: Route::Settings {},
        },
    );
    assert!(settings.contains("loaded-model"), "{settings}");
    assert!(
        settings.contains("Recorder preflight failed."),
        "{settings}"
    );
}

#[test]
fn operator_failure_alert_precedes_route_content() {
    for (path, content) in [("/", "Camera monitor"), ("/analyze", "Analyze sessions")] {
        let mut harness = RenderHarness::new();
        harness.operator_mut().message = Some("The requested action failed".into());

        let html = harness.render_at(ready_preview(), path);

        assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
        assert_eq!(html.matches("The requested action failed").count(), 1);
        assert!(
            html.find("The requested action failed").unwrap() < html.find(content).unwrap(),
            "{html}"
        );
    }
}
