use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{Error, SettingsStore};
use crate::settings::{CameraSettings, LogLevel, OpenAiSettings, Settings};

fn store(root: &Path) -> SettingsStore {
    SettingsStore::new(root.join("config/settings.json"), root.join("default-data"))
}

fn populated_settings(data_root: PathBuf) -> Settings {
    Settings {
        next_camera_id: 2,
        cameras: vec![CameraSettings {
            id: 1,
            name: "Room 1".into(),
            rtsp_url: "rtsp://operator:password@camera.example/stream".into(),
            initially_included_in_analysis: false,
            sample_every_ms: 2_000,
        }],
        data_root: Some(data_root),
        recorder_timeout_secs: 23,
        analysis_frame_sets_per_prompt: 7,
        analysis_overlap_frame_sets: 2,
        openai: OpenAiSettings {
            api_key: "test-secret-key".into(),
            model: "test-model".into(),
            base_url: Some("https://provider.example/v1".into()),
        },
        log_level: LogLevel::Trace,
        ..Settings::default()
    }
}

#[test]
fn missing_file_is_first_launch_without_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());

    assert!(store.load().unwrap().is_none());
    assert!(!store.settings_path.parent().unwrap().exists());
    assert!(!store.default_data_root.exists());
}

#[test]
fn settings_round_trip_and_resolve_runtime_values() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    let settings = populated_settings(root.path().join("selected-data"));

    store.save(&settings).unwrap();
    let resolved = store.load().unwrap().expect("saved settings should load");

    assert!(resolved.settings == settings);
    let data_root = root.path().join("selected-data");
    assert_eq!(resolved.sessions_root, data_root.join("sessions"));
    assert_eq!(resolved.logs_root, data_root.join("logs"));
    assert_eq!(
        resolved.recorder_settings.io_timeout,
        Duration::from_secs(23)
    );
    assert_eq!(resolved.analysis_frame_sets_per_prompt.get(), 7);
    assert_eq!(resolved.analysis_overlap_frame_sets, 2);
    let bytes = fs::read(&store.settings_path).unwrap();
    assert!(bytes.ends_with(b"}\n"));
}

#[test]
fn invalid_file_fails_without_creating_runtime_directories() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
    fs::write(&store.settings_path, b"{not-json\n").unwrap();

    assert!(matches!(store.load(), Err(Error::ParseSettings { .. })));
    assert!(!store.default_data_root.exists());
}

#[test]
fn invalid_save_does_not_overwrite_the_existing_file() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
    fs::write(&store.settings_path, b"old settings\n").unwrap();
    let settings = Settings {
        recorder_timeout_secs: 0,
        ..Settings::default()
    };

    assert!(matches!(
        store.save(&settings),
        Err(Error::InvalidSettings(_))
    ));
    assert_eq!(fs::read(&store.settings_path).unwrap(), b"old settings\n");
}

#[test]
fn saved_settings_are_owner_only() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    store.save(&Settings::default()).unwrap();

    let mode = fs::metadata(&store.settings_path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}
