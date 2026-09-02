use std::collections::HashMap;

use dioxus::prelude::*;

use super::state::{SettingsContext, SettingsField, SettingsPageState};
use crate::settings::{LogLevel, Settings as ApplicationSettings};

const SAVE_ERROR: &str =
    "Settings could not be saved. Check the selected paths and permissions, then try again.";
/// Renders the complete editable application settings form.
#[component]
pub fn Settings() -> Element {
    let SettingsContext { mut state, store } = use_context::<SettingsContext>();
    let (save_error, restart_required) = {
        let page = state.read();
        (page.save_error.clone(), page.restart_required)
    };
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

            if let Some(error) = save_error {
                div { class: "alert alert-error mb-4", role: "alert", "{error}" }
            }
            if restart_required {
                div {
                    class: "alert alert-success mb-4",
                    role: "status",
                    aria_live: "polite",
                    "Settings saved. Restart Leo to apply them."
                }
            }
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
                        ApplicationSettingsSection {}
                    }
                }

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

fn camera_has_error(errors: &HashMap<SettingsField, String>, camera_id: u32) -> bool {
    [
        SettingsField::CameraName(camera_id),
        SettingsField::CameraRtspUrl(camera_id),
        SettingsField::CameraSampleEvery(camera_id),
    ]
    .iter()
    .any(|field| errors.contains_key(field))
}

/// Renders a small settings-only summary for the route sidebar.
#[component]
pub fn SettingsSidebar() -> Element {
    let SettingsContext { state, .. } = use_context::<SettingsContext>();
    let (camera_count, restart_required) = {
        let page = state.read();
        (page.draft.cameras.len(), page.restart_required)
    };

    rsx! {
        section {
            class: "flex flex-col gap-3",
            aria_labelledby: "settings-sidebar-title",
            h2 {
                id: "settings-sidebar-title",
                class: "text-lg font-semibold",
                "Settings"
            }
            p { class: "text-sm", "Configured cameras: {camera_count}" }
            p { class: "text-sm",
                if restart_required {
                    "Restart required"
                } else {
                    "Ready to save"
                }
            }
        }
    }
}

#[component]
fn CameraSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let (cameras, selected_camera_id, selected) = {
        let page = state.read();
        (
            page.draft
                .cameras
                .iter()
                .cloned()
                .map(|camera| {
                    let has_error = camera_has_error(&page.field_errors, camera.id);
                    (camera, has_error)
                })
                .collect::<Vec<_>>(),
            page.selected_camera_id,
            page.selected_camera_id.and_then(|id| {
                page.draft
                    .cameras
                    .iter()
                    .find(|camera| camera.id == id)
                    .cloned()
                    .map(|camera| {
                        let name_error = page
                            .field_errors
                            .get(&SettingsField::CameraName(id))
                            .cloned();
                        let url_error = page
                            .field_errors
                            .get(&SettingsField::CameraRtspUrl(id))
                            .cloned();
                        let cadence_error = page
                            .field_errors
                            .get(&SettingsField::CameraSampleEvery(id))
                            .cloned();
                        (camera, name_error, url_error, cadence_error)
                    })
            }),
        )
    };

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-cameras-title",
            div { class: "flex flex-wrap items-center justify-between gap-2",
                h2 {
                    id: "settings-cameras-title",
                    class: "text-xl font-semibold",
                    "Cameras"
                }
                button {
                    class: "btn btn-sm",
                    r#type: "button",
                    onclick: move |_| {
                        state.write().add_camera();
                    },
                    "Add camera"
                }
            }

            if cameras.is_empty() {
                p { class: "mt-3 text-sm", "No cameras are configured." }
            } else {
                ol { class: "mt-3 flex flex-col gap-2",
                    for (camera, has_error) in cameras {
                        li { key: "{camera.id}",
                            button {
                                class: if selected_camera_id == Some(camera.id) {
                                    "btn h-auto w-full justify-start border-primary bg-base-100 p-3 text-left"
                                } else {
                                    "btn btn-ghost h-auto w-full justify-start p-3 text-left"
                                },
                                r#type: "button",
                                aria_pressed: selected_camera_id == Some(camera.id),
                                onclick: move |_| state.write().selected_camera_id = Some(camera.id),
                                span { class: "min-w-0",
                                    span { class: "block font-medium", "{camera.name}" }
                                    span { class: "block text-xs font-normal", "Camera ID {camera.id}" }
                                    if has_error {
                                        span { class: "badge badge-error badge-outline mt-1", "Needs attention" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some((camera, name_error, url_error, cadence_error)) = selected {
                div { class: "mt-5 flex min-w-0 flex-col gap-4 border-t border-base-300 pt-5",
                    div { class: "flex flex-wrap items-center justify-between gap-2",
                        div {
                            h3 { class: "font-semibold", "Selected camera" }
                            p { class: "text-sm", "Immutable ID: {camera.id}" }
                        }
                        button {
                            class: "btn btn-error btn-outline btn-sm",
                            r#type: "button",
                            onclick: move |_| state.write().remove_selected_camera(),
                            "Remove camera"
                        }
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-name",
                            "Name"
                        }
                        input {
                            id: "camera-{camera.id}-name",
                            class: "input input-bordered w-full",
                            r#type: "text",
                            value: camera.name,
                            aria_invalid: name_error.as_ref().map(|_| "true"),
                            aria_describedby: name_error
                                .as_ref()
                                .map(|_| format!("camera-{}-name-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.name = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = name_error {
                            p {
                                id: "camera-{camera.id}-name-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-rtsp-url",
                            "RTSP URL"
                        }
                        input {
                            id: "camera-{camera.id}-rtsp-url",
                            class: "input input-bordered w-full",
                            r#type: "text",
                            value: camera.rtsp_url,
                            aria_invalid: url_error.as_ref().map(|_| "true"),
                            aria_describedby: url_error
                                .as_ref()
                                .map(|_| format!("camera-{}-rtsp-url-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.rtsp_url = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = url_error {
                            p {
                                id: "camera-{camera.id}-rtsp-url-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }

                    label { class: "flex items-center gap-2 text-sm",
                        input {
                            class: "checkbox checkbox-sm",
                            r#type: "checkbox",
                            checked: camera.initially_included_in_analysis,
                            onchange: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.initially_included_in_analysis = event.checked();
                                }
                            },
                        }
                        "Initially include in analysis"
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-sample-every",
                            "Sample every (seconds)"
                        }
                        input {
                            id: "camera-{camera.id}-sample-every",
                            class: "input input-bordered w-full",
                            r#type: "number",
                            min: "1",
                            step: "1",
                            value: camera.sample_every_secs,
                            aria_invalid: cadence_error.as_ref().map(|_| "true"),
                            aria_describedby: cadence_error
                                .as_ref()
                                .map(|_| format!("camera-{}-sample-every-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.sample_every_secs = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = cadence_error {
                            p {
                                id: "camera-{camera.id}-sample-every-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StorageSettingsSection() -> Element {
    let SettingsContext { mut state, store } = use_context::<SettingsContext>();
    let mut file_input_key = use_signal(|| 0_u64);
    let (custom_root, data_root_error) = {
        let page = state.read();
        (
            page.draft.data_root.clone(),
            page.field_errors.get(&SettingsField::DataRoot).cloned(),
        )
    };
    let resolved_root = custom_root
        .clone()
        .unwrap_or_else(|| store.default_data_root.clone());
    let resolved_root = resolved_root.display().to_string();
    let root_source = if custom_root.is_some() {
        "Selected folder"
    } else {
        "Platform default"
    };

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-storage-title",
            h2 {
                id: "settings-storage-title",
                class: "text-xl font-semibold",
                "Storage"
            }
            p { class: "mt-3 text-sm font-medium", "{root_source}" }
            p { class: "break-all font-mono text-sm", "{resolved_root}" }
            div { class: "mt-4 flex flex-col gap-2",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-data-root-picker",
                    "Choose data root"
                }
                input {
                    key: "{file_input_key}",
                    id: "settings-data-root-picker",
                    class: "file-input file-input-bordered w-full",
                    r#type: "file",
                    directory: true,
                    aria_invalid: data_root_error.as_ref().map(|_| "true"),
                    aria_describedby: data_root_error
                        .as_ref()
                        .map(|_| "settings-data-root-error"),
                    onchange: move |event| {
                        let selected = event
                            .files()
                            .into_iter()
                            .next()
                            .map(|file| file.path());
                        if let Some(path) = selected {
                            state.write().draft.data_root = Some(path);
                        }
                    },
                }
                if let Some(ref error) = data_root_error {
                    p {
                        id: "settings-data-root-error",
                        class: "text-error text-sm",
                        "{error}"
                    }
                }
                button {
                    class: "btn btn-outline btn-sm self-start",
                    r#type: "button",
                    disabled: custom_root.is_none(),
                    onclick: move |_| {
                        state.write().draft.data_root = None;
                        let next = file_input_key().wrapping_add(1);
                        file_input_key.set(next);
                    },
                    "Clear selected folder"
                }
            }
            p { class: "mt-3 text-sm",
                "Changing the data root does not move existing sessions and applies after restart."
            }
        }
    }
}

#[component]
fn RecordingSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let (timeout, error) = {
        let page = state.read();
        (
            page.draft.recorder_timeout_secs.clone(),
            page.field_errors
                .get(&SettingsField::RecorderTimeout)
                .cloned(),
        )
    };

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-recording-title",
            h2 {
                id: "settings-recording-title",
                class: "text-xl font-semibold",
                "Recording"
            }
            div { class: "mt-3 flex flex-col gap-1",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-recorder-timeout",
                    "Recorder timeout (seconds)"
                }
                input {
                    id: "settings-recorder-timeout",
                    class: "input input-bordered w-full",
                    r#type: "number",
                    min: "1",
                    step: "1",
                    value: timeout,
                    aria_invalid: error.as_ref().map(|_| "true"),
                    aria_describedby: error
                        .as_ref()
                        .map(|_| "settings-recorder-timeout-error"),
                    oninput: move |event| {
                        state.write().draft.recorder_timeout_secs = event.value();
                    },
                }
                if let Some(ref error) = error {
                    p {
                        id: "settings-recorder-timeout-error",
                        class: "text-error text-sm",
                        "{error}"
                    }
                }
            }
        }
    }
}

#[component]
fn ProviderSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let mut reveal_key = use_signal(|| false);
    let (
        api_key,
        model,
        base_url,
        frame_sets,
        overlap,
        base_url_error,
        frame_sets_error,
        overlap_error,
    ) = {
        let page = state.read();
        (
            page.draft.openai.api_key.clone(),
            page.draft.openai.model.clone(),
            page.draft.openai.base_url.clone(),
            page.draft.analysis_frame_sets_per_prompt.clone(),
            page.draft.analysis_overlap_frame_sets.clone(),
            page.field_errors
                .get(&SettingsField::OpenAiBaseUrl)
                .cloned(),
            page.field_errors
                .get(&SettingsField::AnalysisFrameSetsPerPrompt)
                .cloned(),
            page.field_errors
                .get(&SettingsField::AnalysisOverlapFrameSets)
                .cloned(),
        )
    };
    let key_type = if reveal_key() { "text" } else { "password" };
    let reveal_label = if reveal_key() {
        "Hide API key"
    } else {
        "Show API key"
    };

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-provider-title",
            h2 {
                id: "settings-provider-title",
                class: "text-xl font-semibold",
                "Analysis provider"
            }
            p { class: "mt-2 text-sm",
                "A blank API key or model disables Analyze."
            }

            div { class: "mt-3 grid grid-cols-1 gap-4 sm:grid-cols-2",
                div { class: "flex flex-col gap-1",
                    label {
                        class: "text-sm font-medium",
                        r#for: "settings-analysis-frame-sets",
                        "Frame sets per prompt"
                    }
                    input {
                        id: "settings-analysis-frame-sets",
                        class: "input input-bordered w-full",
                        r#type: "number",
                        min: "1",
                        step: "1",
                        value: frame_sets,
                        aria_invalid: frame_sets_error.as_ref().map(|_| "true"),
                        aria_describedby: frame_sets_error
                            .as_ref()
                            .map(|_| "settings-analysis-frame-sets-error"),
                        oninput: move |event| {
                            state.write().draft.analysis_frame_sets_per_prompt = event.value();
                        },
                    }
                    if let Some(ref error) = frame_sets_error {
                        p {
                            id: "settings-analysis-frame-sets-error",
                            class: "text-error text-sm",
                            "{error}"
                        }
                    }
                }

                div { class: "flex flex-col gap-1",
                    label {
                        class: "text-sm font-medium",
                        r#for: "settings-analysis-overlap",
                        "Overlapping frame sets"
                    }
                    input {
                        id: "settings-analysis-overlap",
                        class: "input input-bordered w-full",
                        r#type: "number",
                        min: "0",
                        step: "1",
                        value: overlap,
                        aria_invalid: overlap_error.as_ref().map(|_| "true"),
                        aria_describedby: overlap_error
                            .as_ref()
                            .map(|_| "settings-analysis-overlap-error"),
                        oninput: move |event| {
                            state.write().draft.analysis_overlap_frame_sets = event.value();
                        },
                    }
                    if let Some(ref error) = overlap_error {
                        p {
                            id: "settings-analysis-overlap-error",
                            class: "text-error text-sm",
                            "{error}"
                        }
                    }
                }
            }
            p { class: "mt-2 text-sm", "Each frame set can contain one image per camera." }
            p { class: "mt-1 text-sm", "Overlap repeats images and may increase provider cost." }

            div { class: "mt-4 flex flex-col gap-1",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-openai-key",
                    "OpenAI API key"
                }
                input {
                    id: "settings-openai-key",
                    class: "input input-bordered w-full",
                    r#type: key_type,
                    autocomplete: "off",
                    value: api_key,
                    oninput: move |event| state.write().draft.openai.api_key = event.value(),
                }
                div { class: "flex flex-wrap gap-2",
                    button {
                        class: "btn btn-outline btn-sm",
                        r#type: "button",
                        onclick: move |_| reveal_key.toggle(),
                        "{reveal_label}"
                    }
                    button {
                        class: "btn btn-outline btn-sm",
                        r#type: "button",
                        onclick: move |_| state.write().draft.openai.api_key.clear(),
                        "Clear API key"
                    }
                }
            }

            div { class: "mt-4 flex flex-col gap-1",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-openai-model",
                    "Model"
                }
                input {
                    id: "settings-openai-model",
                    class: "input input-bordered w-full",
                    r#type: "text",
                    value: model,
                    oninput: move |event| state.write().draft.openai.model = event.value(),
                }
            }

            div { class: "mt-4 flex flex-col gap-1",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-openai-base-url",
                    "Base URL (optional)"
                }
                input {
                    id: "settings-openai-base-url",
                    class: "input input-bordered w-full",
                    r#type: "url",
                    value: base_url,
                    aria_invalid: base_url_error.as_ref().map(|_| "true"),
                    aria_describedby: base_url_error
                        .as_ref()
                        .map(|_| "settings-openai-base-url-error"),
                    oninput: move |event| state.write().draft.openai.base_url = event.value(),
                }
                if let Some(ref error) = base_url_error {
                    p {
                        id: "settings-openai-base-url-error",
                        class: "text-error text-sm",
                        "{error}"
                    }
                }
            }
        }
    }
}

#[component]
fn ApplicationSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let level = state.read().draft.log_level;

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-application-title",
            h2 {
                id: "settings-application-title",
                class: "text-xl font-semibold",
                "Application"
            }
            div { class: "mt-3 flex flex-col gap-1",
                label {
                    class: "text-sm font-medium",
                    r#for: "settings-log-level",
                    "Log level"
                }
                select {
                    id: "settings-log-level",
                    class: "select select-bordered w-full",
                    value: level.as_str(),
                    oninput: move |event| {
                        if let Some(level) = log_level(&event.value()) {
                            state.write().draft.log_level = level;
                        }
                    },
                    for level in [
                        LogLevel::Error,
                        LogLevel::Warn,
                        LogLevel::Info,
                        LogLevel::Debug,
                        LogLevel::Trace,
                    ] {
                        option { value: level.as_str(), "{level.as_str()}" }
                    }
                }
            }
        }
    }
}

fn log_level(value: &str) -> Option<LogLevel> {
    match value {
        "error" => Some(LogLevel::Error),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsField, begin_save};
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
