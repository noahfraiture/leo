use super::state::{AnalysisProfileDraft, MonitoringProfileDraft, SettingsContext};
use backend::profiles::{
    ImageDetailPolicy, validate_analysis_profiles, validate_monitoring_profiles,
};
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
                onclick: move |_| state.write().draft.monitoring_profiles.retain(|p| p.id != id),
                "Remove monitoring profile"
            }
            if referenced {
                p { class: "text-xs", "Change camera assignments before removing this profile." }
            }
        }
    }
}

/// Model and request limits for later analysis; invalid values never disable recording setup.
#[component]
pub fn AnalysisProfilesSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let profiles = state.read().draft.analysis_profiles.clone();
    let default_id = state.read().draft.default_analysis_profile_id;
    let definitions = profiles
        .iter()
        .map(AnalysisProfileDraft::profile)
        .collect::<Vec<_>>();
    let error = validate_analysis_profiles(&definitions)
        .err()
        .map(|error| error.to_string());
    rsx! {
        section { class: "rounded-box border border-base-300 p-4",
            h2 { class: "text-xl font-semibold", "Analysis profiles" }
            if let Some(error) = error {
                p { class: "text-warning mt-2", role: "status",
                    "{error}. Fix before analysis; recording remains available."
                }
            }
            label {
                class: "mt-3 block text-sm",
                r#for: "default-analysis-profile",
                "Default analysis profile"
            }
            select {
                id: "default-analysis-profile",
                class: "select select-bordered w-full",
                value: "{default_id}",
                onchange: move |event| {
                    state.write().draft.default_analysis_profile_id = event
                        .value()
                        .parse()
                        .unwrap_or(0);
                },
                option { value: "0", "Select a profile" }
                for profile in &profiles {
                    option { value: "{profile.id}", "{profile.name}" }
                }
            }
            for profile in profiles {
                AnalysisProfileEditor { key: "{profile.id}", profile }
            }
            button {
                class: "btn btn-sm mt-3",
                r#type: "button",
                onclick: move |_| {
                    let mut page = state.write();
                    let id = page.draft.next_analysis_profile_id;
                    if id == 0 || page.draft.analysis_profiles.iter().any(|p| p.id >= id) {
                        page.save_error = Some(
                            "Correct nextAnalysisProfileId in the settings file before adding a profile."
                                .into(),
                        );
                    } else if let Some(next) = id.checked_add(1) {
                        page.draft.next_analysis_profile_id = next;
                        let mut profile = crate::settings::Settings::default()
                            .analysis_profiles
                            .remove(0);
                        profile.id = id;
                        profile.name = format!("Analysis {id}");
                        page.draft.analysis_profiles.push(profile.into());
                    } else {
                        page.save_error = Some("Analysis profile IDs are exhausted.".into());
                    }
                },
                "Add analysis profile"
            }
        }
    }
}

#[component]
fn AnalysisProfileEditor(profile: AnalysisProfileDraft) -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let id = profile.id;
    let is_default = state.read().draft.default_analysis_profile_id == id;
    let detail = match profile.detail {
        ImageDetailPolicy::ProviderDefault => "default",
        ImageDetailPolicy::Low => "low",
        ImageDetailPolicy::High => "high",
    };
    rsx! {
        div { class: "mt-4 flex flex-col gap-3 border-t border-base-300 pt-4",
            ProfileField {
                id: "analysis-{id}-name",
                label: "Profile name",
                value: profile.name,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.name = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-model",
                label: "Model",
                value: profile.model,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.model = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-max_images",
                label: "Maximum images per prompt",
                value: profile.max_images,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.max_images = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-max_span_ms",
                label: "Maximum prompt span (milliseconds)",
                value: profile.max_span_ms,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.max_span_ms = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-overlap",
                label: "Overlapping frame sets",
                value: profile.overlap,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.overlap = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-maximum_edge",
                label: "Maximum image edge (pixels, blank for original)",
                value: profile.maximum_edge,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.maximum_edge = value;
                    }
                },
            }
            ProfileField {
                id: "analysis-{id}-max_output_tokens",
                label: "Maximum output tokens (optional)",
                value: profile.max_output_tokens,
                oninput: move |value| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.max_output_tokens = value;
                    }
                },
            }
            label { class: "text-sm", r#for: "analysis-{id}-detail", "Image detail" }
            select {
                id: "analysis-{id}-detail",
                class: "select select-bordered w-full",
                value: detail,
                onchange: move |event| {
                    if let Some(p) = state
                        .write()
                        .draft
                        .analysis_profiles
                        .iter_mut()
                        .find(|p| p.id == id)
                    {
                        p.detail = match event.value().as_str() {
                            "low" => ImageDetailPolicy::Low,
                            "high" => ImageDetailPolicy::High,
                            _ => ImageDetailPolicy::ProviderDefault,
                        };
                    }
                },
                option { value: "default", "Provider default" }
                option { value: "low", "Low" }
                option { value: "high", "High" }
            }
            button {
                class: "btn btn-outline btn-sm",
                r#type: "button",
                disabled: is_default,
                onclick: move |_| state.write().draft.analysis_profiles.retain(|p| p.id != id),
                "Remove analysis profile"
            }
            if is_default {
                p { class: "text-xs", "Choose another default before removing this profile." }
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
                value,
                oninput: move |event| oninput.call(event.value()),
            }
        }
    }
}
