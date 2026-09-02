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
        SettingsField::CameraSampleEvery(removed_id),
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
            | SettingsField::CameraSampleEvery(id)
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
fn draft_round_trips_analysis_batching_fields() {
    let settings = Settings {
        analysis_frame_sets_per_prompt: 7,
        analysis_overlap_frame_sets: 2,
        ..Settings::default()
    };
    let mut state = SettingsPageState::new(settings);

    assert_eq!(state.draft.analysis_frame_sets_per_prompt, "7");
    assert_eq!(state.draft.analysis_overlap_frame_sets, "2");

    state.draft.analysis_frame_sets_per_prompt = "9".into();
    state.draft.analysis_overlap_frame_sets = "3".into();
    let submitted = state.submission().expect("batching draft should be valid");
    assert_eq!(submitted.analysis_frame_sets_per_prompt, 9);
    assert_eq!(submitted.analysis_overlap_frame_sets, 3);
}

#[test]
fn draft_maps_analysis_batching_errors_to_their_fields() {
    let mut state = SettingsPageState::new(Settings::default());
    state.draft.analysis_frame_sets_per_prompt.clear();
    state.draft.analysis_overlap_frame_sets = "-1".into();

    let errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("invalid batching draft should fail"),
    };
    assert!(errors.contains_key(&SettingsField::AnalysisFrameSetsPerPrompt));
    assert!(errors.contains_key(&SettingsField::AnalysisOverlapFrameSets));

    state.draft.analysis_frame_sets_per_prompt = "5".into();
    state.draft.analysis_overlap_frame_sets = "5".into();
    let errors = match state.submission() {
        Err(errors) => errors,
        Ok(_) => panic!("overlapping batching draft should fail"),
    };
    assert_eq!(
        errors
            .get(&SettingsField::AnalysisOverlapFrameSets)
            .map(String::as_str),
        Some(
            "Enter a nonnegative whole number within runtime limits and smaller than frame sets per prompt."
        )
    );
}
