use super::{super::state::SettingsField, begin_save};
use crate::{settings::Settings, views::SettingsPageState};

#[test]
fn valid_draft_begins_save() {
    let mut page = SettingsPageState::new(Settings::default());
    assert!(begin_save(&mut page).is_some());
    assert!(page.field_errors.is_empty());
    assert!(page.saving);
}

#[test]
fn an_in_flight_settings_save_cannot_be_submitted_twice() {
    let mut page = SettingsPageState::new(Settings::default());
    begin_save(&mut page).unwrap();
    assert!(begin_save(&mut page).is_none());
    page.finish_save(page.draft.clone());
    assert!(begin_save(&mut page).is_some());
}

#[test]
fn failed_begin_save_selects_the_first_camera_with_an_error() {
    let mut page = SettingsPageState::new(Settings::default());
    page.add_camera().unwrap();
    page.add_camera().unwrap();
    page.draft.cameras[0].rtsp_url = "rtsp://camera-one.example/stream".into();
    page.draft.cameras[1].rtsp_url = "http://wrong".into();
    page.selected_camera_id = Some(1);

    assert!(begin_save(&mut page).is_none());
    assert!(!page.saving);

    assert_eq!(page.selected_camera_id, Some(2));
    assert!(
        page.field_errors
            .contains_key(&SettingsField::CameraRtspUrl(2))
    );
}
