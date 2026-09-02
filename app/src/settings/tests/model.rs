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
            sample_every_ms: 1_000,
        }],
        ..Settings::default()
    }
}

fn assert_invalid_camera_url(rtsp_url: &str) {
    let errors = settings_with_camera_url(rtsp_url).validate().unwrap_err();
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidCameraUrl { camera_id: 1 })
    );
    assert_eq!(errors.to_string(), "settings validation failed");
}

#[test]
fn defaults_are_an_unconfigured_valid_draft() {
    let settings = Settings::default();
    assert_eq!(SETTINGS_SCHEMA_VERSION, 2);
    assert_eq!(settings.schema_version, 2);
    assert_eq!(settings.next_camera_id, 1);
    assert!(settings.cameras.is_empty());
    assert_eq!(settings.log_level, LogLevel::Info);
    let value = serde_json::to_value(&settings).unwrap();
    assert_eq!(value["analysisFrameSetsPerPrompt"], 5);
    assert_eq!(value["analysisOverlapFrameSets"], 0);
    settings.validate().unwrap();
}

#[test]
fn zero_and_multiple_cameras_are_valid() {
    let mut settings = Settings::default();
    settings.validate().unwrap();
    for id in 1..=3 {
        settings.cameras.push(CameraSettings {
            id,
            name: format!("Camera {id}"),
            rtsp_url: format!("rtsp://camera-{id}/stream"),
            initially_included_in_analysis: true,
            sample_every_ms: 1_000,
        });
    }
    settings.next_camera_id = 4;
    settings.validate().unwrap();
}

#[test]
fn rejects_hostless_camera_url() {
    assert_invalid_camera_url("rtsp:private-hostless-marker");
}

#[test]
fn accepts_credential_bearing_camera_url() {
    settings_with_camera_url("rtsp://user:pass@camera.example/stream")
        .validate()
        .unwrap();
}

#[test]
fn accepts_percent_encoded_camera_url() {
    settings_with_camera_url("rtsp://camera.example/private-%25-marker")
        .validate()
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
                sample_every_ms: 0,
            },
            CameraSettings {
                id: 1,
                name: "Camera 1".into(),
                rtsp_url: "https://camera-1/stream".into(),
                initially_included_in_analysis: false,
                sample_every_ms: 1_500,
            },
            CameraSettings {
                id: 1,
                name: "Camera 1 duplicate".into(),
                rtsp_url: "rtsp://camera-1/stream".into(),
                initially_included_in_analysis: true,
                sample_every_ms: 1_000,
            },
        ],
        data_root: Some(PathBuf::from("relative/data")),
        recorder_timeout_secs: 0,
        analysis_frame_sets_per_prompt: 0,
        analysis_overlap_frame_sets: 1,
        openai: OpenAiSettings {
            api_key: String::new(),
            model: String::new(),
            base_url: Some("ftp://provider.example/v1".into()),
        },
        log_level: LogLevel::Info,
    };

    let errors = settings.validate().unwrap_err();
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
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidSamplingCadence { camera_id: 1 })
    );
    assert!(errors.0.contains(&ValidationError::InvalidRecorderTimeout));
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidAnalysisFrameSetsPerPrompt)
    );
    assert!(
        errors
            .0
            .contains(&ValidationError::InvalidAnalysisOverlapFrameSets)
    );
    assert!(errors.0.contains(&ValidationError::DataRootNotAbsolute {
        path: PathBuf::from("relative/data"),
    }));
    assert!(errors.0.contains(&ValidationError::InvalidOpenAiBaseUrl));
}

#[test]
fn camera_ids_must_be_below_the_next_allocation() {
    let mut settings = Settings::default();
    settings.cameras.push(CameraSettings {
        id: 1,
        name: "Camera 1".into(),
        rtsp_url: "rtsp://camera-1/stream".into(),
        initially_included_in_analysis: true,
        sample_every_ms: 1_000,
    });

    assert!(
        settings
            .validate()
            .unwrap_err()
            .0
            .contains(&ValidationError::CameraIdNotBelowNext {
                camera_id: 1,
                next_camera_id: 1,
            })
    );
}

#[test]
fn overlap_must_be_smaller_than_frame_sets_per_prompt() {
    let settings = Settings {
        analysis_frame_sets_per_prompt: 5,
        analysis_overlap_frame_sets: 5,
        ..Settings::default()
    };

    assert_eq!(
        settings.validate().unwrap_err().0,
        vec![ValidationError::InvalidAnalysisOverlapFrameSets]
    );
}

#[test]
fn openai_config_requires_both_nonblank_values() {
    let mut settings = Settings::default();
    assert!(settings.openai_config().is_none());

    settings.openai.api_key = "key".into();
    settings.openai.model = " \t".into();
    assert!(settings.openai_config().is_none());

    settings.openai.model = "model".into();
    settings.openai.base_url = Some("https://provider.example/v1".into());
    let config = settings.openai_config().unwrap();
    assert_eq!(config.api_key, "key");
    assert_eq!(config.model, "model");
    assert_eq!(
        config.base_url.as_deref(),
        Some("https://provider.example/v1")
    );
}
