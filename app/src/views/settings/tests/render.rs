//! Render-level coverage for the settings form's operator-visible contract.

use std::sync::{Arc, Mutex};

use dioxus::{dioxus_core::NoOpMutations, prelude::*};

use super::{Settings, SettingsContext, SettingsPageState};
use crate::{
    settings::{CameraSettings, OpenAiSettings, Settings as ApplicationSettings, SettingsStore},
    test_support::opening_tag_with_marker,
};

#[derive(Clone)]
struct RenderProps {
    state: Arc<Mutex<Option<SettingsPageState>>>,
    store: SettingsStore,
}

fn render_root(props: RenderProps) -> Element {
    let RenderProps { state, store } = props;
    let state = use_signal(move || {
        state
            .lock()
            .expect("render settings mutex should not be poisoned")
            .take()
            .expect("render settings root should take state once")
    });
    use_context_provider(move || SettingsContext { state, store });
    rsx! { Settings {} }
}

fn render_settings(state: SettingsPageState, store: Option<SettingsStore>) -> String {
    let props = RenderProps {
        state: Arc::new(Mutex::new(Some(state))),
        store: store.unwrap_or_else(render_settings_store),
    };
    let mut dom = VirtualDom::new_with_props(render_root, props);
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

fn render_settings_store() -> SettingsStore {
    let root = std::env::temp_dir().join("leo-render-settings");
    SettingsStore::new(root.join("settings.json"), root.join("data"))
}

#[test]
fn form_composes_all_sections_and_keeps_editable_secrets_local_to_inputs() {
    let temporary = tempfile::tempdir().expect("temporary settings root should be created");
    let store = SettingsStore::new(
        temporary.path().join("config/settings.json"),
        temporary.path().join("default-data"),
    );
    let default_data_root = store.default_data_root.display().to_string();
    let settings = ApplicationSettings {
        next_camera_id: 2,
        cameras: vec![CameraSettings {
            id: 1,
            name: "Camera 1".into(),
            rtsp_url: "rtsp://render-camera.example/stream".into(),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        }],
        openai: OpenAiSettings {
            api_key: "render-secret-key".into(),
            model: String::new(),
            base_url: None,
        },
        ..ApplicationSettings::default()
    };
    let state = SettingsPageState::new(settings);

    let html = render_settings(state, Some(store));

    for heading in [
        "Application settings",
        "Cameras",
        "Storage",
        "Recording",
        "Analysis provider",
        "Application",
    ] {
        assert!(html.contains(heading), "missing {heading:?} in {html}");
    }
    assert!(html.contains("type=\"password\""), "{html}");
    assert!(html.contains("webkitdirectory"), "{html}");
    assert!(html.contains(&default_data_root), "{html}");
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
fn camera_field_error_is_accessibly_described() {
    let mut state = SettingsPageState::new(ApplicationSettings::default());
    state.add_camera();
    state.draft.cameras[0].rtsp_url = "http://wrong".into();
    state.field_errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("camera URL should be invalid"),
    };

    let html = render_settings(state, None);

    let input = opening_tag_with_marker(&html, "input", r#"id="camera-1-rtsp-url""#);
    assert!(input.contains(r#"aria-invalid="true""#), "{input}");
    assert!(
        input.contains(r#"aria-describedby="camera-1-rtsp-url-error""#),
        "{input}"
    );
    assert!(html.contains("Enter a valid RTSP URL."), "{html}");
}

#[test]
fn camera_list_marks_each_camera_with_a_validation_error() {
    let mut state = SettingsPageState::new(ApplicationSettings::default());
    for _ in 0..3 {
        let id = state.add_camera();
        state.draft.cameras[usize::try_from(id - 1).unwrap()].rtsp_url =
            format!("rtsp://camera-{id}.example/stream");
    }
    state.draft.cameras[1].name.clear();
    state.draft.cameras[2].rtsp_url = "http://wrong".into();
    state.selected_camera_id = Some(1);
    state.field_errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("cameras should be invalid"),
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
