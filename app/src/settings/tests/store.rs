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
            initial_monitoring_profile_id: 2,
        }],
        data_root: Some(data_root),
        recorder_timeout_secs: 23,
        monitoring_profiles: crate::test_monitoring_profiles(),
        next_monitoring_profile_id: 4,
        analysis_profiles: vec![crate::test_analysis_profile(7, 2)],
        openai: OpenAiSettings {
            api_key: "test-secret-key".into(),
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
    assert_eq!(
        resolved.settings.analysis_profiles[0].max_images_per_prompt,
        7
    );
    assert_eq!(resolved.settings.analysis_profiles[0].overlap_frame_sets, 2);
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

#[test]
fn malformed_optional_sections_keep_recording_usable_and_leave_the_source_file_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    let valid = populated_settings(root.path().join("data"));
    for key in [
        "monitoringProfiles",
        "analysisProfiles",
        "openai",
        "initiallyIncludedInAnalysis",
    ] {
        let mut value = serde_json::to_value(&valid).unwrap();
        if key == "initiallyIncludedInAnalysis" {
            value["cameras"][0][key] = serde_json::json!("invalid");
        } else {
            value[key] = serde_json::json!({"bad": "private-value"});
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::create_dir_all(store.settings_path.parent().unwrap()).unwrap();
        fs::write(&store.settings_path, &bytes).unwrap();
        let loaded = store.load().unwrap().unwrap();
        loaded.settings.validate_recording().unwrap();
        let warning = if key == "monitoringProfiles" || key == "initiallyIncludedInAnalysis" {
            loaded.monitoring_error
        } else {
            loaded.analysis_error
        };
        assert!(warning.unwrap().contains(key));
        assert_eq!(fs::read(&store.settings_path).unwrap(), bytes);
    }
}

#[test]
fn diagnostic_logging_failure_does_not_block_recording_storage() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path());
    fs::create_dir_all(&store.default_data_root).unwrap();
    fs::write(store.default_data_root.join("logs"), b"occupied").unwrap();
    store.save(&Settings::default()).unwrap();
    assert!(store.load().unwrap().unwrap().sessions_root.is_dir());
}
