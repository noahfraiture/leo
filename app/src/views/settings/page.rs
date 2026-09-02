use dioxus::prelude::*;

use super::{
    application::ApplicationSettingsSection,
    camera::CameraSettingsSection,
    provider::ProviderSettingsSection,
    recording::RecordingSettingsSection,
    state::{SettingsContext, SettingsPageState, camera_has_error},
    storage::StorageSettingsSection,
};
use crate::settings::Settings as ApplicationSettings;

const SAVE_ERROR: &str =
    "Settings could not be saved. Check the selected paths and permissions, then try again.";

/// Renders the complete editable application settings form.
#[component]
pub fn Settings() -> Element {
    let SettingsContext { mut state, store } = use_context::<SettingsContext>();
    let save_store = store.clone();

    rsx! {
        article {
            class: "min-w-0 p-2 sm:p-4",
            aria_labelledby: "application-settings-title",
            header { class: "mb-5",
                h1 {
                    id: "application-settings-title",
                    class: "text-2xl font-semibold",
                    "Application settings"
                }
                p { class: "mt-1 text-sm",
                    "Configure Leo, save, then restart to apply every change."
                }
            }

            SettingsNotices {}

            form {
                class: "flex flex-col gap-6",
                novalidate: true,
                onsubmit: move |event| {
                    event.prevent_default();
                    let settings = {
                        let mut page = state.write();
                        let Some(settings) = begin_save(&mut page) else {
                            return;
                        };
                        settings
                    };
                    let mut page = state.write();
                    match save_store.save(&settings) {
                        Ok(()) => page.mark_saved(),
                        Err(_) => page.save_error = Some(SAVE_ERROR.into()),
                    }
                },

                div { class: "grid grid-cols-1 gap-6 xl:grid-cols-2",
                    CameraSettingsSection {}
                    div { class: "flex min-w-0 flex-col gap-6",
                        StorageSettingsSection {}
                        RecordingSettingsSection {}
                        ProviderSettingsSection {}
                    }
                }
                ApplicationSettingsSection {}

                div { class: "flex flex-col items-start gap-2 sm:flex-row sm:items-center",
                    button {
                        class: "btn btn-primary",
                        r#type: "submit",
                        "Save settings"
                    }
                }
            }
        }
    }
}

/// Renders save and restart status independently from the form layout.
#[component]
fn SettingsNotices() -> Element {
    let state = use_context::<SettingsContext>().state;
    let page = state.read();

    rsx! {
        if let Some(error) = &page.save_error {
            div { class: "alert alert-error mb-4", role: "alert", "{error}" }
        }
        if page.restart_required {
            div {
                class: "alert alert-success mb-4",
                role: "status",
                aria_live: "polite",
                "Settings saved. Restart Leo to apply them."
            }
        }
    }
}

fn begin_save(page: &mut SettingsPageState) -> Option<ApplicationSettings> {
    match page.submission() {
        Ok(settings) => {
            page.field_errors.clear();
            page.save_error = None;
            Some(settings)
        }
        Err(errors) => {
            if let Some(camera) = page
                .draft
                .cameras
                .iter()
                .find(|camera| camera_has_error(&errors, camera.id))
            {
                page.selected_camera_id = Some(camera.id);
            }
            page.field_errors = errors;
            page.save_error = None;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{super::state::SettingsField, begin_save};
    use crate::{settings::Settings, views::SettingsPageState};

    #[test]
    fn valid_draft_begins_save() {
        let mut page = SettingsPageState::new(Settings::default());
        assert!(begin_save(&mut page).is_some());
        assert!(page.field_errors.is_empty());
    }

    #[test]
    fn failed_begin_save_selects_the_first_camera_with_an_error() {
        let mut page = SettingsPageState::new(Settings::default());
        page.add_camera();
        page.add_camera();
        page.draft.cameras[0].rtsp_url = "rtsp://camera-one.example/stream".into();
        page.draft.cameras[1].rtsp_url = "http://wrong".into();
        page.selected_camera_id = Some(1);

        assert!(begin_save(&mut page).is_none());

        assert_eq!(page.selected_camera_id, Some(2));
        assert!(
            page.field_errors
                .contains_key(&SettingsField::CameraRtspUrl(2))
        );
    }
}
