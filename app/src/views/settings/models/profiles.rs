use crate::views::SettingsContext;
use backend::model::ModelProfile;
use dioxus::prelude::*;

use super::numbers::NumberField;

/// Models carry their estimated local memory allowance; hosted models use zero.
#[component]
pub fn Models() -> Element {
    let mut state = use_context::<SettingsContext>().state;
    let settings = state.read().draft.workflow.clone();
    rsx! {
        div { class: "flex flex-col gap-3",
            h3 { class: "font-semibold", "2. Models" }
            for (i, model) in settings.models.iter().enumerate() {
                div {
                    key: "{model.id}",
                    class: "rounded-box border border-base-300 p-3 grid grid-cols-1 gap-2",
                    label {
                        "Display name"
                        input {
                            class: "input input-bordered w-full",
                            value: "{model.name}",
                            oninput: move |e| state.write().draft.workflow.models[i].name = e.value(),
                        }
                    }
                    label {
                        "Connection"
                        select {
                            class: "select select-bordered w-full",
                            value: "{model.provider_id}",
                            onchange: move |e| state.write().draft.workflow.models[i].provider_id = e.value(),
                            option { value: "", selected: model.provider_id.is_empty(), "Select connection" }
                            for p in &settings.providers {
                                // Select the option when it mounts, before the native select has a value.
                                option { key: "{p.id}", value: "{p.id}", selected: p.id == model.provider_id, "{p.name}" }
                            }
                        }
                    }
                    label {
                        "Model identifier"
                        input {
                            class: "input input-bordered w-full",
                            value: "{model.model}",
                            placeholder: "Identifier from your model server",
                            oninput: move |e| state.write().draft.workflow.models[i].model = e.value(),
                        }
                    }
                    NumberField {
                        name: "memory",
                        model_id: model.id.clone(),
                        label: "Local memory allowance (MiB, hosted models = 0)",
                        value: Some(u64::from(model.memory_cost_mib)),
                        minimum: 0,
                        maximum: 49152,
                        onvalid: move |value| {
                            if let Some(value) = value {
                                state.write().draft.workflow.models[i].memory_cost_mib = value as u32;
                            }
                        },
                    }
                    p { class: "text-xs opacity-70",
                        "Include room for image processing and model working memory. 1024 MiB = 1 GiB."
                    }
                    NumberField {
                        name: "images",
                        model_id: model.id.clone(),
                        label: "Maximum images per request",
                        value: Some(model.image_limit as u64),
                        minimum: 1,
                        maximum: usize::MAX as u64,
                        onvalid: move |value| {
                            if let Some(value) = value {
                                state.write().draft.workflow.models[i].image_limit = value as usize;
                            }
                        },
                    }
                    NumberField {
                        name: "tokens",
                        model_id: model.id.clone(),
                        label: "Maximum output tokens",
                        value: model.max_output_tokens,
                        minimum: 1,
                        maximum: u64::MAX,
                        optional: true,
                        onvalid: move |value| state.write().draft.workflow.models[i].max_output_tokens = value,
                    }
                    label {
                        "Reasoning effort (optional)"
                        input {
                            class: "input input-bordered w-full",
                            value: model.reasoning_effort.clone().unwrap_or_default(),
                            placeholder: "For example: low",
                            onchange: move |e| {
                                state.write().draft.workflow.models[i].reasoning_effort = (!e
                                    .value()
                                    .trim()
                                    .is_empty())
                                    .then(|| e.value());
                            },
                        }
                    }
                    if model.resolve(&settings.providers).is_err() {
                        p { class: "text-warning text-sm",
                            "Complete the model identifier, name, connection and image limit."
                        }
                    }
                    button {
                        class: "btn btn-ghost btn-sm justify-self-start",
                        r#type: "button",
                        onclick: move |_| {
                            let mut page = state.write();
                            let id = page.draft.workflow.models[i].id.clone();
                            page.remove_workflow_model(&id);
                        },
                        "Remove model"
                    }
                }
            }
            button {
                class: "btn btn-outline btn-sm self-start",
                r#type: "button",
                onclick: move |_| {
                    let mut page = state.write();
                    let provider_id = page
                        .draft
                        .workflow
                        .providers
                        .first()
                        .map(|p| p.id.clone())
                        .unwrap_or_default();
                    page.draft
                        .workflow
                        .models
                        .push(ModelProfile {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: "New model".into(),
                            provider_id,
                            model: String::new(),
                            memory_cost_mib: 0,
                            image_limit: 16,
                            max_output_tokens: Some(6000),
                            reasoning_effort: None,
                        });
                },
                "Add model"
            }
        }
    }
}
