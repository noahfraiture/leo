use dioxus::prelude::*;

use super::state::{SettingsContext, SettingsField};

/// Edits and explains the effective application data root.
#[component]
pub fn StorageSettingsSection() -> Element {
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
        .unwrap_or_else(|| store.default_data_root.clone())
        .display()
        .to_string();
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
