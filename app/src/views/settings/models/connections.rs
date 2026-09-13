use crate::views::SettingsContext;
use backend::model::{ProviderConnection, Transport};
use dioxus::prelude::*;

/// Named provider endpoints with explicit protocol selection and masked private credentials.
#[component]
pub fn Connections() -> Element {
    let mut state = use_context::<SettingsContext>().state;
    let providers = state.read().draft.workflow.providers.clone();
    rsx! {
        div { class: "flex flex-col gap-3",
            h3 { class: "font-semibold", "1. Connections" }
            for (i, provider) in providers.iter().enumerate() {
                div {
                    key: "{provider.id}",
                    class: "rounded-box border border-base-300 p-3 grid grid-cols-1 gap-2",
                    label { class: "form-control",
                        "Connection name"
                        input {
                            class: "input input-bordered w-full",
                            value: "{provider.name}",
                            oninput: move |e| state.write().draft.workflow.providers[i].name = e.value(),
                        }
                    }
                    label {
                        "Server address"
                        input {
                            class: "input input-bordered w-full",
                            r#type: "url",
                            value: "{provider.base_url}",
                            placeholder: "http://localhost:1234/v1",
                            oninput: move |e| state.write().draft.workflow.providers[i].base_url = e.value(),
                        }
                    }
                    label {
                        "API format"
                        select {
                            class: "select select-bordered w-full",
                            value: if provider.transport == Transport::Responses { "responses" } else { "chat" },
                            onchange: move |e| {
                                state.write().draft.workflow.providers[i].transport = if e.value() == "responses"
                                {
                                    Transport::Responses
                                } else {
                                    Transport::ChatCompletions
                                };
                            },
                            option { value: "chat", "Chat Completions (LM Studio / OpenRouter)" }
                            option { value: "responses", "Responses (OpenAI)" }
                        }
                    }
                    label {
                        "API key (leave empty for an unauthenticated local server)"
                        input {
                            id: "provider-{provider.id}-key",
                            class: "input input-bordered w-full",
                            r#type: "password",
                            autocomplete: "off",
                            value: "{provider.api_key}",
                            oninput: move |e| state.write().draft.workflow.providers[i].api_key = e.value(),
                        }
                    }
                    if provider.snapshot().is_err() {
                        p { class: "text-warning text-sm",
                            "Enter a name and an HTTP or HTTPS base address without credentials or query parameters."
                        }
                    }
                    button {
                        class: "btn btn-ghost btn-sm justify-self-start",
                        r#type: "button",
                        onclick: move |_| {
                            state.write().draft.workflow.providers.remove(i);
                        },
                        "Remove connection"
                    }
                }
            }
            button {
                class: "btn btn-outline btn-sm self-start",
                r#type: "button",
                onclick: move |_| {
                    state
                        .write()
                        .draft
                        .workflow
                        .providers
                        .push(ProviderConnection {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: "New connection".into(),
                            base_url: "http://localhost:1234/v1".into(),
                            transport: Transport::ChatCompletions,
                            api_key: String::new(),
                        })
                },
                "Add connection"
            }
        }
    }
}
