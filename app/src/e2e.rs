//! Feature-gated launcher and UI driver for the desktop operator-flow E2E.

use std::{fs, path::PathBuf};

use backend::model::ProviderConnection;
use dioxus::{desktop::DesktopContext, prelude::*};

use crate::settings::{ResolvedSettings, SettingsStore};

/// Launches the desktop against the E2E-owned settings file.
pub fn launch(settings_path: PathBuf) {
    let default_data_root = settings_path
        .parent()
        .expect("E2E settings path should have a parent")
        .join("default-data");
    crate::desktop::launch_with_store(SettingsStore::new(settings_path, default_data_root));
}

const COMPLETE_ANALYSIS_SCENARIO: &str = "complete-analysis";
const REVIEW_TEXT_SCENARIO: &str = "review-text";
const SETTINGS_TEXT_SCENARIO: &str = "settings-text";
const CAMERA_EVENTS_SCENARIO: &str = "camera-events";
const PROFILE_EVENTS_SCENARIO: &str = "profile-events";
const MODEL_SET_EVENTS_SCENARIO: &str = "model-set-events";
const ANALYSIS_RECOVERY_SCENARIO: &str = "analysis-recovery";
const RECORD_WITHOUT_PREVIEW_SCENARIO: &str = "record-without-preview";
const RECORDING_REHEARSAL_SCENARIO: &str = "recording-rehearsal";
const SAVE_PROFILES_SCENARIO: &str = "save-profiles";
const RELOAD_PROFILES_SCENARIO: &str = "reload-profiles";
const ANALYZE_EXISTING_SCENARIO: &str = "analyze-existing";
const EXERCISE_RECOVERY_SCENARIO: &str = "exercise-recovery";
const QUESTION_EVENTS_SCENARIO: &str = "question-events";
const PREPARATION_NAVIGATION_SCENARIO: &str = "preparation-navigation";
const RAPID_EXERCISE_INPUT_SCENARIO: &str = "rapid-exercise-input";
const CHECKLIST_REPLACEMENT_SCENARIO: &str = "checklist-replacement";
const CHECKLIST_PASTE_EVENTS_SCENARIO: &str = "checklist-paste-events";
const CHECKLIST_DELETE_EVENTS_SCENARIO: &str = "checklist-delete-events";
const CHECKLIST_PUBLISH_EVENTS_SCENARIO: &str = "checklist-publish-events";
const DRIVER_SCRIPT: &str = include_str!("e2e/driver.js");

/// Drives the mounted production UI when the E2E result paths are configured.
#[component]
pub fn DesktopE2eDriver() -> Element {
    let desktop = consume_context::<DesktopContext>();
    let resolved = consume_context::<ResolvedSettings>();
    let mut operator = consume_context::<Signal<crate::operator::OperatorState>>();
    use_hook(move || {
        let ready_path = environment_path("LEO_DESKTOP_E2E_READY")?;
        let result_path = environment_path("LEO_DESKTOP_E2E_RESULT")?;
        if let Err(error) = fs::write(&ready_path, b"ready\n") {
            tracing::error!(path = %ready_path.display(), %error, "desktop E2E ready handshake failed");
        }

        let configured_scenario = std::env::var("LEO_DESKTOP_E2E_SCENARIO").ok();
        let Some(scenario) = selected_driver_scenario(configured_scenario.as_deref()) else {
            if let Err(error) = fs::write(&result_path, b"error: unknown desktop E2E scenario\n") {
                tracing::error!(path = %result_path.display(), %error, "desktop E2E result write failed");
            }
            desktop.close();
            return None;
        };

        let real_openai = std::env::var("LEO_E2E_REAL_OPENAI").ok();
        let paid_openai = std::env::var("LEO_RUN_PAID_OPENAI_TEST").ok();
        let proxy_configured = [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
        .into_iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()));
        let no_proxy = std::env::var("NO_PROXY")
            .ok()
            .or_else(|| std::env::var("no_proxy").ok());
        if resolved.settings.workflow.providers.iter().any(|provider| {
            !provider_configuration_is_safe(
                provider,
                real_openai.as_deref(),
                paid_openai.as_deref(),
            ) || !loopback_proxy_is_bypassed(
                &provider.base_url,
                proxy_configured,
                no_proxy.as_deref(),
            )
        }) {
            if let Err(error) = fs::write(
                &result_path,
                b"error: desktop E2E provider safety gate rejected configuration\n",
            ) {
                tracing::error!(path = %result_path.display(), %error, "desktop E2E result write failed");
            }
            desktop.close();
            return None;
        }

        Some(spawn(async move {
            let preview_program =
                serde_json::to_string(include_str!("components/camera/reader.js"))
                    .expect("preview program serializes");
            let script = format!(
                "const previewReaderProgram = {preview_program};\n{}\n{}\n{}\n{}",
                include_str!("e2e/preview_lifecycle.js"),
                include_str!("media/pixel_probe.js"),
                include_str!("e2e/editing.js"),
                DRIVER_SCRIPT
            );
            let mut driver = document::eval(&script);
            let mut result = match driver.send(scenario) {
                Ok(()) => driver
                    .recv::<String>()
                    .await
                    .unwrap_or_else(|error| format!("error: desktop E2E driver failed: {error}")),
                Err(error) => format!("error: desktop E2E driver failed to start: {error}"),
            };
            if result == "inject-metadata-failure" && scenario == RECORD_WITHOUT_PREVIEW_SCENARIO {
                {
                    let mut state = operator.write();
                    if let crate::operator::SessionRunState::Active {
                        controller: Some(controller),
                        ..
                    } = &mut state.session
                    {
                        controller
                            .fail_writes_for_test()
                            .expect("inject metadata write failure into disposable E2E session");
                    }
                }
                result = match driver.send("metadata writes disabled") {
                    Ok(()) => driver
                        .recv::<String>()
                        .await
                        .unwrap_or_else(|error| format!("error: {error}")),
                    Err(error) => format!("error: {error}"),
                };
            }
            if let Some(path) = environment_path("LEO_DESKTOP_E2E_PIXEL_RESULT") {
                let mut inspection =
                    document::eval("dioxus.send(globalThis.leoPlaybackDiagnostics ?? null)");
                if let Ok(value) = inspection.recv::<serde_json::Value>().await {
                    if !value.is_null() {
                        fs::write(
                            path,
                            serde_json::to_vec_pretty(&value).expect("pixel report serializes"),
                        )
                        .expect("write requested pixel diagnostic");
                    }
                }
            }
            if let Err(error) = fs::write(&result_path, result) {
                tracing::error!(path = %result_path.display(), %error, "desktop E2E result write failed");
            }
            desktop.close();
        }))
    });
    rsx! {}
}

fn selected_driver_scenario(configured: Option<&str>) -> Option<&'static str> {
    match configured.unwrap_or(COMPLETE_ANALYSIS_SCENARIO) {
        COMPLETE_ANALYSIS_SCENARIO => Some(COMPLETE_ANALYSIS_SCENARIO),
        REVIEW_TEXT_SCENARIO => Some(REVIEW_TEXT_SCENARIO),
        SETTINGS_TEXT_SCENARIO => Some(SETTINGS_TEXT_SCENARIO),
        CAMERA_EVENTS_SCENARIO => Some(CAMERA_EVENTS_SCENARIO),
        PROFILE_EVENTS_SCENARIO => Some(PROFILE_EVENTS_SCENARIO),
        MODEL_SET_EVENTS_SCENARIO => Some(MODEL_SET_EVENTS_SCENARIO),
        ANALYSIS_RECOVERY_SCENARIO => Some(ANALYSIS_RECOVERY_SCENARIO),
        RECORD_WITHOUT_PREVIEW_SCENARIO => Some(RECORD_WITHOUT_PREVIEW_SCENARIO),
        RECORDING_REHEARSAL_SCENARIO => Some(RECORDING_REHEARSAL_SCENARIO),
        SAVE_PROFILES_SCENARIO => Some(SAVE_PROFILES_SCENARIO),
        RELOAD_PROFILES_SCENARIO => Some(RELOAD_PROFILES_SCENARIO),
        ANALYZE_EXISTING_SCENARIO => Some(ANALYZE_EXISTING_SCENARIO),
        EXERCISE_RECOVERY_SCENARIO => Some(EXERCISE_RECOVERY_SCENARIO),
        QUESTION_EVENTS_SCENARIO => Some(QUESTION_EVENTS_SCENARIO),
        PREPARATION_NAVIGATION_SCENARIO => Some(PREPARATION_NAVIGATION_SCENARIO),
        RAPID_EXERCISE_INPUT_SCENARIO => Some(RAPID_EXERCISE_INPUT_SCENARIO),
        CHECKLIST_REPLACEMENT_SCENARIO => Some(CHECKLIST_REPLACEMENT_SCENARIO),
        CHECKLIST_PASTE_EVENTS_SCENARIO => Some(CHECKLIST_PASTE_EVENTS_SCENARIO),
        CHECKLIST_DELETE_EVENTS_SCENARIO => Some(CHECKLIST_DELETE_EVENTS_SCENARIO),
        CHECKLIST_PUBLISH_EVENTS_SCENARIO => Some(CHECKLIST_PUBLISH_EVENTS_SCENARIO),
        _ => None,
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn provider_configuration_is_safe(
    provider: &ProviderConnection,
    real_openai: Option<&str>,
    paid_openai: Option<&str>,
) -> bool {
    if provider.snapshot().is_err() {
        return false;
    }
    let Ok(url) = url::Url::parse(&provider.base_url) else {
        return false;
    };
    let local = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    local
        || (url.as_str().trim_end_matches('/') == "https://api.openai.com/v1"
            && !provider.api_key.trim().is_empty()
            && real_openai == Some("1")
            && paid_openai == Some("1"))
}

fn loopback_proxy_is_bypassed(
    base_url: &str,
    proxy_configured: bool,
    no_proxy: Option<&str>,
) -> bool {
    base_url.trim_end_matches('/') == "https://api.openai.com/v1"
        || !proxy_configured
        || no_proxy.is_some_and(|value| value.split(',').any(|entry| entry.trim() == "*"))
}

#[cfg(test)]
#[path = "e2e/tests.rs"]
mod tests;
