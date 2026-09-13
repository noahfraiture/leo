use super::*;
use crate::settings::Settings;

#[test]
fn camera_ids_remain_monotonic_after_removing_the_selected_camera() {
    let mut state = SettingsPageState::new(Settings::default());
    assert_eq!(state.add_camera(), 1);
    state.remove_selected_camera();
    assert_eq!(state.add_camera(), 2);
}

#[test]
fn removing_a_camera_clears_only_its_field_errors() {
    let mut state = SettingsPageState::new(Settings::default());
    state.add_camera();
    let removed_id = state.add_camera();
    for field in [
        SettingsField::CameraName(removed_id),
        SettingsField::CameraRtspUrl(removed_id),
        SettingsField::CameraMonitoringProfile(removed_id),
    ] {
        state.field_errors.insert(field, "invalid".into());
    }
    state
        .field_errors
        .insert(SettingsField::CameraName(1), "keep".into());

    state.remove_selected_camera();

    assert!(
        state
            .field_errors
            .contains_key(&SettingsField::CameraName(1))
    );
    assert!(!state.field_errors.keys().any(|field| matches!(
        field,
        SettingsField::CameraName(id)
            | SettingsField::CameraRtspUrl(id)
            | SettingsField::CameraMonitoringProfile(id)
            if *id == removed_id
    )));
}

#[test]
fn draft_conversion_reports_camera_and_numeric_fields() {
    let mut state = SettingsPageState::new(Settings::default());
    state.add_camera();
    state.draft.cameras[0].rtsp_url = "http://wrong".into();
    state.draft.recorder_timeout_secs.clear();
    let errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("invalid draft should not produce settings"),
    };
    assert!(errors.contains_key(&SettingsField::CameraRtspUrl(1)));
    assert!(errors.contains_key(&SettingsField::RecorderTimeout));
}

#[test]
fn monitoring_drafts_round_trip_millisecond_cadence() {
    let mut state = SettingsPageState::new(Settings::default());
    state.draft.monitoring_profiles[0].sample_every_ms = "500".into();
    let submitted = state.submission().unwrap();
    assert_eq!(submitted.monitoring_profiles[0].sample_every_ms, 500);
}

#[test]
fn invalid_profile_drafts_remain_explicit_while_recording_can_be_saved() {
    let mut state = SettingsPageState::new(Settings::default());
    state.draft.monitoring_profiles[0].sample_every_ms.clear();
    let submitted = state
        .submission()
        .expect("valid recording settings can still be saved");
    submitted.validate_recording().unwrap();
    assert!(submitted.validate_monitoring().is_err());
    assert!(
        state.draft.monitoring_profiles[0]
            .sample_every_ms
            .is_empty()
    );
}
