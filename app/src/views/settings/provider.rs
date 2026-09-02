use dioxus::prelude::*;

use super::state::{SettingsContext, SettingsField};

/// Edits provider credentials, endpoint, model, and batching controls.
#[component]
pub fn ProviderSettingsSection() -> Element {
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
