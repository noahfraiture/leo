use dioxus::prelude::*;

use super::{
    application::ApplicationSettingsSection,
    camera::CameraSettingsSection,
    profiles::{AnalysisProfilesSection, MonitoringProfilesSection},
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
                p { class: "mt-1 text-sm", "Configure Leo, save, then restart to apply every change." }
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
                        MonitoringProfilesSection {}
                        AnalysisProfilesSection {}
                        ProviderSettingsSection {}
                    }
                }
                ApplicationSettingsSection {}

                div { class: "flex flex-col items-start gap-2 sm:flex-row sm:items-center",
                    button { class: "btn btn-primary", r#type: "submit", "Save settings" }
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

    let startup_error =
        try_consume_context::<crate::desktop::RuntimeAvailability>().and_then(|availability| {
            match availability {
                crate::desktop::RuntimeAvailability::Failed { message } => Some(message),
                _ => None,
            }
        });
    rsx! {
        if let Some(error) = startup_error {
            div { class: "alert alert-warning mb-4", role: "alert", "{error}" }
        }
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
#[path = "tests/page.rs"]
mod tests;
