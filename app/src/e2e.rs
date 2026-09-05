//! Feature-gated launcher and UI driver for the desktop operator-flow E2E.

use std::{fs, path::PathBuf};

use backend::analysis::OpenAiConfig;
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
const ANALYSIS_RECOVERY_SCENARIO: &str = "analysis-recovery";
const RECORD_WITHOUT_PREVIEW_SCENARIO: &str = "record-without-preview";
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

        let openai = resolved.settings.openai_config();
        let openai = openai.as_ref();
        let base_url = openai.and_then(|config| config.base_url.as_deref());
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
        if !provider_configuration_is_safe(openai, real_openai.as_deref(), paid_openai.as_deref())
            || !loopback_proxy_is_bypassed(base_url, proxy_configured, no_proxy.as_deref())
        {
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
            let mut driver = document::eval(DRIVER_SCRIPT);
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
        ANALYSIS_RECOVERY_SCENARIO => Some(ANALYSIS_RECOVERY_SCENARIO),
        RECORD_WITHOUT_PREVIEW_SCENARIO => Some(RECORD_WITHOUT_PREVIEW_SCENARIO),
        _ => None,
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn provider_configuration_is_safe(
    openai: Option<&OpenAiConfig>,
    real_openai: Option<&str>,
    paid_openai: Option<&str>,
) -> bool {
    let Some(openai) = openai else {
        return false;
    };
    if openai.api_key.trim().is_empty() {
        return false;
    }
    if let Some(base_url) = openai.base_url.as_deref() {
        return url::Url::parse(base_url).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && match url.host() {
                    Some(url::Host::Ipv4(address)) => address.is_loopback(),
                    Some(url::Host::Ipv6(address)) => address.is_loopback(),
                    _ => false,
                }
        });
    }

    real_openai == Some("1") && paid_openai == Some("1")
}

fn loopback_proxy_is_bypassed(
    base_url: Option<&str>,
    proxy_configured: bool,
    no_proxy: Option<&str>,
) -> bool {
    base_url.is_none()
        || !proxy_configured
        || no_proxy.is_some_and(|value| value.split(',').any(|entry| entry.trim() == "*"))
}

#[cfg(test)]
#[path = "e2e/tests.rs"]
mod tests;
