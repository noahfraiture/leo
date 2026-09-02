use dioxus::prelude::*;

use super::state::SettingsContext;
use crate::settings::LogLevel;

/// Edits application-wide behavior that does not belong to a runtime subsystem.
#[component]
pub fn ApplicationSettingsSection() -> Element {
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
                        if let Some(level) = parse_log_level(&event.value()) {
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

fn parse_log_level(value: &str) -> Option<LogLevel> {
    match value {
        "error" => Some(LogLevel::Error),
        "warn" => Some(LogLevel::Warn),
        "info" => Some(LogLevel::Info),
        "debug" => Some(LogLevel::Debug),
        "trace" => Some(LogLevel::Trace),
        _ => None,
    }
}
