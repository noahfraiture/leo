use dioxus::prelude::*;

use super::state::{SettingsContext, SettingsField};

/// Edits recorder process timing configuration.
#[component]
pub fn RecordingSettingsSection() -> Element {
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
