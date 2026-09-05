use dioxus::prelude::*;

use super::state::SettingsContext;

/// Edits provider credentials independently from profiles and recording setup.
#[component]
pub fn ProviderSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let mut reveal_key = use_signal(|| false);
    let (api_key, base_url) = {
        let page = state.read();
        (
            page.draft.openai.api_key.clone(),
            page.draft.openai.base_url.clone(),
        )
    };
    let base_url_error = (!base_url.trim().is_empty()
        && !url::Url::parse(&base_url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host()))
    .then(|| "Enter an absolute HTTP or HTTPS URL. Recording remains available.".to_owned());
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
                "Provider credentials"
            }
            p { class: "mt-2 text-sm",
                "Missing credentials disable analysis. Recording remains available."
            }

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
