use std::path::PathBuf;

use super::*;
use crate::settings::ValidationError;

fn settings_with_camera_url(rtsp_url: &str) -> Settings {
    Settings {
        next_camera_id: 2,
        cameras: vec![CameraSettings {
            id: 1,
            name: "Camera 1".into(),
            rtsp_url: rtsp_url.into(),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: 1,
        }],
        ..Settings::default()
    }
}

fn assert_invalid_camera_url(rtsp_url: &str) {
    let errors = settings_with_camera_url(rtsp_url)
        .validate_recording()
        .unwrap_err();
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidCameraUrl { camera_id: 1 })
    );
    assert!(!errors.to_string().contains(rtsp_url));
}

#[test]
fn defaults_are_an_unconfigured_valid_draft() {
    let settings = Settings::default();
    assert_eq!(SETTINGS_SCHEMA_VERSION, 3);
    assert_eq!(settings.schema_version, 3);
    assert_eq!(settings.next_camera_id, 1);
    assert!(settings.cameras.is_empty());
    assert_eq!(settings.log_level, LogLevel::Info);
    let value = serde_json::to_value(&settings).unwrap();
    assert!(value.get("analysisFrameSetsPerPrompt").is_none());
    assert_eq!(value["monitoringProfiles"][0]["sampleEveryMs"], 1000);
    assert!(settings.validate_analysis().is_err());
    settings.validate_recording().unwrap();
}

#[test]
fn zero_and_multiple_cameras_are_valid() {
    let mut settings = Settings::default();
    settings.validate_recording().unwrap();
    for id in 1..=3 {
        settings.cameras.push(CameraSettings {
            id,
            name: format!("Camera {id}"),
            rtsp_url: format!("rtsp://camera-{id}/stream"),
            initially_included_in_analysis: true,
            initial_monitoring_profile_id: 1,
        });
    }
    settings.next_camera_id = 4;
    settings.validate_recording().unwrap();
}

#[test]
fn rejects_hostless_camera_url() {
    assert_invalid_camera_url("rtsp:private-hostless-marker");
}

#[test]
fn accepts_credential_bearing_camera_url() {
    settings_with_camera_url("rtsp://user:pass@camera.example/stream")
        .validate_recording()
        .unwrap();
}

#[test]
fn accepts_percent_encoded_camera_url() {
    settings_with_camera_url("rtsp://camera.example/private-%25-marker")
        .validate_recording()
        .unwrap();
}

#[test]
fn validation_returns_all_field_errors() {
    let settings = Settings {
        schema_version: SETTINGS_SCHEMA_VERSION + 1,
        next_camera_id: 0,
        cameras: vec![
            CameraSettings {
                id: 0,
                name: " ".into(),
                rtsp_url: "".into(),
                initially_included_in_analysis: true,
                initial_monitoring_profile_id: 0,
            },
            CameraSettings {
                id: 1,
                name: "Camera 1".into(),
                rtsp_url: "https://camera-1/stream".into(),
                initially_included_in_analysis: false,
                initial_monitoring_profile_id: 99,
            },
            CameraSettings {
                id: 1,
                name: "Camera 1 duplicate".into(),
                rtsp_url: "rtsp://camera-1/stream".into(),
                initially_included_in_analysis: true,
                initial_monitoring_profile_id: 1,
            },
        ],
        data_root: Some(PathBuf::from("relative/data")),
        recorder_timeout_secs: 0,

        openai: OpenAiSettings {
            api_key: String::new(),
            base_url: Some("ftp://provider.example/v1".into()),
        },
        log_level: LogLevel::Info,
        ..Settings::default()
    };

    let errors = settings.validate_recording().unwrap_err();
    assert!(
        errors
            .0
            .contains(&ValidationError::UnsupportedSchemaVersion {
                expected: SETTINGS_SCHEMA_VERSION,
                actual: SETTINGS_SCHEMA_VERSION + 1,
            })
    );
    assert!(errors.0.contains(&ValidationError::InvalidNextCameraId));
    assert!(
        errors
            .0
            .contains(&ValidationError::ZeroCameraId { camera_index: 0 })
    );
    assert!(
        errors
            .0
            .contains(&ValidationError::DuplicateCameraId { camera_id: 1 })
    );
    assert!(
        errors
            .0
            .contains(&ValidationError::BlankCameraName { camera_id: 0 })
    );
    assert!(
        errors
            .0
            .contains(&ValidationError::BlankCameraUrl { camera_id: 0 })
    );
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidCameraUrl { camera_id: 1 })
    );
    assert!(errors.0.contains(&ValidationError::InvalidRecorderTimeout));
    assert!(errors.0.contains(&ValidationError::DataRootNotAbsolute {
        path: PathBuf::from("relative/data"),
    }));
    assert!(settings.validate_monitoring().is_err());
    assert!(settings.validate_analysis().is_err());
}

#[test]
fn camera_ids_must_be_below_the_next_allocation() {
    let mut settings = Settings::default();
    settings.cameras.push(CameraSettings {
        id: 1,
        name: "Camera 1".into(),
        rtsp_url: "rtsp://camera-1/stream".into(),
        initially_included_in_analysis: true,
        initial_monitoring_profile_id: 1,
    });

    assert!(settings.validate_recording().unwrap_err().0.contains(
        &ValidationError::CameraIdNotBelowNext {
            camera_id: 1,
            next_camera_id: 1,
        }
    ));
}

#[test]
fn invalid_monitoring_or_analysis_does_not_invalidate_recording() {
    let mut settings = settings_with_camera_url("rtsp://camera.example/live");
    settings.cameras[0].initial_monitoring_profile_id = 99;
    settings.analysis_profiles[0].overlap_frame_sets = usize::MAX;
    settings.validate_recording().unwrap();
    assert!(settings.validate_monitoring().is_err());
    assert!(settings.validate_analysis().is_err());
}

#[test]
fn provider_credentials_validate_independently_from_the_model() {
    let mut settings = Settings::default();
    assert!(settings.openai_config().is_none());
    settings.openai.api_key = "key".into();
    settings.openai.base_url = Some("https://provider.example/v1".into());
    assert_eq!(settings.openai_config().unwrap().api_key, "key");
    assert!(settings.validate_analysis().is_err());
    settings.analysis_profiles[0].model = "model".into();
    settings.validate_analysis().unwrap();
    settings.openai.base_url = Some("ftp://provider.example".into());
    assert!(settings.openai_config().is_none());
}
