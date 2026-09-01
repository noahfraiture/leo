//! Feature-gated driver for the real desktop operator-flow E2E.

use std::{fs, path::PathBuf};

use backend::analysis::OpenAiConfig;
use dioxus::{desktop::DesktopContext, prelude::*};

use crate::settings::ResolvedSettings;

const DRIVER_SCRIPT: &str = r#"
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

const waitFor = async (predicate, description, timeout = 30000) => {
    const started = Date.now();
    while (Date.now() - started < timeout) {
        const value = predicate();
        if (value) return value;
        await sleep(100);
    }
    const body = document.body?.innerText?.slice(0, 4000) ?? "document body unavailable";
    throw new Error(`Timed out waiting for ${description}. Body: ${body}`);
};

const button = (label) => Array.from(document.querySelectorAll("button"))
    .find((candidate) => candidate.textContent.trim() === label && !candidate.disabled);

const input = (element, value) => {
    const setter = Object.getOwnPropertyDescriptor(
        Object.getPrototypeOf(element),
        "value",
    ).set;
    setter.call(element, value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
};

(async () => {
    await waitFor(() => ["camera-0-video", "camera-1-video"].every((id) => {
        const video = document.getElementById(id);
        const tracks = video?.srcObject?.getVideoTracks?.() ?? [];
        return video && video.videoWidth > 0 && tracks.some((track) => track.readyState === "live");
    }), "two live previews", 45000);

    (await waitFor(() => button("Start session"), "Start session button")).click();
    await waitFor(() => document.body.innerText.includes("Session active"), "active session", 45000);
    await waitFor(
        () => document.querySelectorAll('[aria-label$="recorder status: Recording"]').length === 2,
        "two recording statuses",
    );

    const cadence = await waitFor(
        () => document.getElementById("sampling-interval-1"),
        "camera one cadence input",
    );
    input(cadence, "2");
    cadence.closest("form").dispatchEvent(new Event("submit", {
        bubbles: true,
        cancelable: true,
    }));
    await sleep(3000);

    (await waitFor(() => button("Stop session"), "Stop session button")).click();
    await waitFor(() => document.body.innerText.includes("Session idle"), "idle session", 45000);

    const analyze = await waitFor(
        () => Array.from(document.querySelectorAll("a"))
            .find((candidate) => candidate.textContent.trim() === "Analyze"),
        "Analyze navigation",
    );
    analyze.click();
    await waitFor(() => document.getElementById("completed-sessions-title"), "completed sessions");

    let checklist = document.getElementById("analysis-checklist");
    if (!checklist) {
        const row = await waitFor(
            () => document.querySelector('button[aria-label^="Session "]'),
            "completed session row",
        );
        row.click();
        checklist = await waitFor(
            () => document.getElementById("analysis-checklist"),
            "analysis checklist",
        );
    }
    input(checklist, "Keep movement controlled");
    (await waitFor(() => button("Analyze"), "Analyze action")).click();

    await waitFor(
        () => document.querySelector('button[aria-label*="status: Complete"]'),
        "completed analysis",
        90000,
    );
    await waitFor(() => document.getElementById("analysis-results-title"), "analysis results");
    const renderedSummary = await waitFor(
        () => document.querySelector('[aria-labelledby="sequence-summary-title"] p')
            ?.textContent.trim() || null,
        "rendered sequence summary",
    );
    dioxus.send(`ok\n${renderedSummary}`);
})().catch((error) => {
    dioxus.send(`error: ${error?.stack ?? error}`);
});
"#;

/// Drives the mounted production UI when the E2E result paths are configured.
#[component]
pub fn DesktopE2eDriver() -> Element {
    let desktop = consume_context::<DesktopContext>();
    let resolved = consume_context::<ResolvedSettings>();
    use_hook(move || {
        let ready_path = environment_path("LEO_DESKTOP_E2E_READY")?;
        let result_path = environment_path("LEO_DESKTOP_E2E_RESULT")?;
        if let Err(error) = fs::write(&ready_path, b"ready\n") {
            tracing::error!(path = %ready_path.display(), %error, "desktop E2E ready handshake failed");
        }

        let openai = resolved.openai.as_ref();
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
            let result = document::eval(DRIVER_SCRIPT)
                .recv::<String>()
                .await
                .unwrap_or_else(|error| format!("error: desktop E2E driver failed: {error}"));
            if let Err(error) = fs::write(&result_path, result) {
                tracing::error!(path = %result_path.display(), %error, "desktop E2E result write failed");
            }
            desktop.close();
        }))
    });
    rsx! {}
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
    if openai.api_key.trim().is_empty() || openai.model.trim().is_empty() {
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
mod tests {
    use backend::analysis::OpenAiConfig;

    use super::{loopback_proxy_is_bypassed, provider_configuration_is_safe};

    fn openai_config(api_key: &str, model: &str, base_url: Option<&str>) -> OpenAiConfig {
        OpenAiConfig {
            api_key: api_key.into(),
            model: model.into(),
            base_url: base_url.map(str::to_owned),
        }
    }

    #[test]
    fn active_provider_configuration_requires_loopback_or_both_paid_gates() {
        assert!(provider_configuration_is_safe(
            Some(&openai_config(
                "key",
                "model",
                Some("http://127.42.0.1:3000/v1"),
            )),
            None,
            None,
        ));
        assert!(provider_configuration_is_safe(
            Some(&openai_config("key", "model", Some("http://[::1]:3000/v1"))),
            None,
            None,
        ));
        assert!(provider_configuration_is_safe(
            Some(&openai_config("key", "model", None)),
            Some("1"),
            Some("1"),
        ));

        for configuration in [
            (None, None, None),
            (
                Some(openai_config(
                    "key",
                    "model",
                    Some("https://api.openai.com/v1"),
                )),
                None,
                None,
            ),
            (
                Some(openai_config(
                    "key",
                    "model",
                    Some("http://localhost:3000/v1"),
                )),
                None,
                None,
            ),
            (Some(openai_config("key", "model", None)), Some("1"), None),
            (
                Some(openai_config(" ", "model", None)),
                Some("1"),
                Some("1"),
            ),
            (Some(openai_config("key", " ", None)), Some("1"), Some("1")),
            (
                Some(openai_config(
                    "key",
                    "model",
                    Some("https://api.openai.com/v1"),
                )),
                Some("1"),
                Some("1"),
            ),
            (
                Some(openai_config(
                    " ",
                    "model",
                    Some("http://127.0.0.1:3000/v1"),
                )),
                None,
                None,
            ),
        ] {
            assert!(!provider_configuration_is_safe(
                configuration.0.as_ref(),
                configuration.1,
                configuration.2,
            ));
        }
    }

    #[test]
    fn loopback_provider_rejects_a_proxy_without_a_global_bypass() {
        let base_url = Some("http://127.0.0.1:3000/v1");

        assert!(loopback_proxy_is_bypassed(base_url, false, None));
        assert!(loopback_proxy_is_bypassed(
            base_url,
            true,
            Some("*, .local")
        ));
        assert!(loopback_proxy_is_bypassed(None, true, None));
        assert!(!loopback_proxy_is_bypassed(base_url, true, None));
        assert!(!loopback_proxy_is_bypassed(
            base_url,
            true,
            Some("127.0.0.1, localhost"),
        ));
    }
}
