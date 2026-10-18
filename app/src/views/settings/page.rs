use dioxus::prelude::*;

use super::{
    application::ApplicationSettingsSection,
    camera::CameraSettingsSection,
    models::ModelSettingsSection,
    profiles::MonitoringProfilesSection,
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
                    let (settings, submitted) = {
                        let mut page = state.write();
                        let Some(settings) = begin_save(&mut page) else {
                            return;
                        };
                        (settings, page.draft.clone())
                    };
                    let store = save_store.clone();
                    dioxus::dioxus_core::spawn_forever(async move {
                        let result = tokio::task::spawn_blocking(move || store.save(&settings)).await;
                        let mut page = state.write();
                        match result {
                            Ok(Ok(())) => page.finish_save(submitted),
                            _ => {
                                page.saving = false;
                                page.save_error = Some(SAVE_ERROR.into());
                            }
                        }
                    });
                },

                fieldset {
                    class: "flex min-w-0 flex-col gap-6",
                    disabled: state.read().saving,
                    div { class: "grid grid-cols-1 gap-6 xl:grid-cols-2",
                        CameraSettingsSection {}
                        div { class: "flex min-w-0 flex-col gap-6",
                            StorageSettingsSection {}
                            RecordingSettingsSection {}
                            MonitoringProfilesSection {}
                            ModelSettingsSection {}
                        }
                    }
                    ApplicationSettingsSection {}

                    div { class: "flex flex-col items-start gap-2 sm:flex-row sm:items-center",
                        button {
                            class: "btn btn-primary",
                            r#type: "submit",
                            disabled: state.read().saving,
                            if state.read().saving { "Saving settings..." } else { "Save settings" }
                        }
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
    let model_settings_error = try_consume_context::<crate::settings::ResolvedSettings>()
        .and_then(|settings| settings.analysis_error);

    let startup_error =
        try_consume_context::<crate::desktop::RuntimeAvailability>().and_then(|availability| {
            match availability {
                crate::desktop::RuntimeAvailability::Failed { message } => Some(message),
                _ => None,
            }
        });
    rsx! {
        if let Some(error) = model_settings_error {
            div { class: "alert alert-warning mb-4", role: "alert", "{error}" }
        }
        if let Some(error) = startup_error {
            div { class: "alert alert-warning mb-4", role: "alert", "{error}" }
        }
        if let Some(error) = &page.save_error {
            div { class: "alert alert-error mb-4", role: "alert", "{error}" }
        }
        if !page.saving && page.has_unsaved_changes() {
            div { class: "alert alert-warning mb-4", role: "status", aria_live: "polite",
                if page.restart_required {
                    "Unsaved changes. Restarting applies the previously saved settings; save again to keep your latest edits."
                } else {
                    "Unsaved changes. Save settings to keep your latest edits."
                }
            }
        } else if !page.saving && page.restart_required {
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
    if page.saving {
        return None;
    }
    match page.submission() {
        Ok(settings) => {
            page.field_errors.clear();
            page.save_error = None;
            page.saving = true;
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
