use super::*;
use crate::settings::Settings;

#[test]
fn camera_ids_remain_monotonic_after_removing_the_selected_camera() {
    let mut state = SettingsPageState::new(Settings::default());
    assert_eq!(state.add_camera(), Some(1));
    state.remove_selected_camera();
    assert_eq!(state.add_camera(), Some(2));
}

#[test]
fn exhausted_camera_ids_leave_the_settings_draft_unchanged() {
    let settings = Settings {
        next_camera_id: u32::MAX,
        ..Settings::default()
    };
    settings.validate_recording().unwrap();
    let mut state = SettingsPageState::new(settings);
    let original = state.draft.clone();

    assert!(state.add_camera().is_none());

    assert!(state.draft == original);
    assert!(
        state
            .save_error
            .as_deref()
            .is_some_and(|message| message.contains("exhausted"))
    );
}

#[test]
fn removing_a_camera_clears_only_its_field_errors() {
    let mut state = SettingsPageState::new(Settings::default());
    state.add_camera().unwrap();
    let removed_id = state.add_camera().unwrap();
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
    state.add_camera().unwrap();
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

#[test]
fn save_completion_does_not_claim_newer_typing_was_persisted() {
    let mut page = SettingsPageState::new(Settings::default());
    page.draft.recorder_timeout_secs = "30".into();
    let submitted = page.draft.clone();
    page.draft.recorder_timeout_secs = "45".into();

    page.finish_save(submitted.clone());

    assert_eq!(page.draft.recorder_timeout_secs, "45");
    assert!(page.has_unsaved_changes());
    assert!(page.restart_required);
    page.draft = submitted;
    assert!(!page.has_unsaved_changes());
}
