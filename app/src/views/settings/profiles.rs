use super::state::{MonitoringProfileDraft, SettingsContext};
use backend::profiles::validate_monitoring_profiles;
use dioxus::prelude::*;

/// Named monitoring definitions, edited independently from the active recording snapshot.
#[component]
pub fn MonitoringProfilesSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let profiles = state.read().draft.monitoring_profiles.clone();
    let definitions = profiles
        .iter()
        .map(MonitoringProfileDraft::profile)
        .collect::<Vec<_>>();
    let error = validate_monitoring_profiles(&definitions)
        .err()
        .map(|error| error.to_string());
    rsx! {
        section { class: "rounded-box border border-base-300 p-4",
            h2 { class: "text-xl font-semibold", "Monitoring profiles" }
            p { class: "mt-2 text-sm",
                "Profiles select evidence for later analysis. Cameras always record continuously."
            }
            if let Some(error) = error {
                p { class: "text-warning mt-2", role: "status",
                    "{error}. Recording remains available; monitoring metadata needs correction."
                }
            }
            for profile in profiles {
                MonitoringProfileEditor { key: "{profile.id}", profile }
            }
            button {
                class: "btn btn-sm mt-3",
                r#type: "button",
                onclick: move |_| {
                    let mut page = state.write();
                    let id = page.draft.next_monitoring_profile_id;
                    if id == 0 || page.draft.monitoring_profiles.iter().any(|p| p.id >= id) {
                        page.save_error = Some(
                            "Correct nextMonitoringProfileId in the settings file before adding a profile."
                                .into(),
                        );
                    } else if let Some(next) = id.checked_add(1) {
                        page.draft.next_monitoring_profile_id = next;
                        page.draft
                            .monitoring_profiles
                            .push(MonitoringProfileDraft {
                                id,
                                name: format!("Monitoring {id}"),
                                sample_every_ms: "1000".into(),
                            });
                    } else {
                        page.save_error = Some("Monitoring profile IDs are exhausted.".into());
                    }
                },
                "Add monitoring profile"
            }
        }
    }
}

#[component]
fn MonitoringProfileEditor(profile: MonitoringProfileDraft) -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let id = profile.id;
    let referenced = state
        .read()
        .draft
        .cameras
        .iter()
        .any(|camera| camera.initial_monitoring_profile_id == id);
    rsx! {
        div { class: "mt-4 flex flex-col gap-3 border-t border-base-300 pt-4",
            ProfileField {
                id: "monitoring-{id}-name",
                label: "Profile name",
                value: profile.name,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .monitoring_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.name = value;
                    }
                },
            }
            ProfileField {
                id: "monitoring-{id}-cadence",
                label: "Sample every (milliseconds)",
                value: profile.sample_every_ms,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .monitoring_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.sample_every_ms = value;
                    }
                },
            }
            button {
                class: "btn btn-outline btn-sm",
                r#type: "button",
                disabled: referenced,
                onclick: move |_| {
                    let mut page = state.write();
                    if !page.draft.cameras.iter().any(|camera| camera.initial_monitoring_profile_id == id) {
                        page.draft.monitoring_profiles.retain(|profile| profile.id != id);
                    }
                },
                "Remove monitoring profile"
            }
            if referenced {
                p { class: "text-xs", "Change camera assignments before removing this profile." }
            }
        }
    }
}

#[component]
fn ProfileField(
    id: String,
    label: String,
    value: String,
    oninput: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "flex flex-col gap-1",
            label { class: "text-sm font-medium", r#for: id.clone(), "{label}" }
            input {
                id,
                class: "input input-bordered w-full",
                initial_value: value,
                oninput: move |event| oninput.call(event.value()),
            }
        }
    }
}
