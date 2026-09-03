use backend::analysis::OpenAiConfig;

use super::{
    ANALYSIS_RECOVERY_SCENARIO, COMPLETE_ANALYSIS_SCENARIO, RECORD_WITHOUT_PREVIEW_SCENARIO,
    loopback_proxy_is_bypassed, provider_configuration_is_safe, selected_driver_scenario,
};

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

#[test]
fn desktop_driver_defaults_to_the_complete_flow_and_rejects_unknown_scenarios() {
    assert_eq!(
        selected_driver_scenario(None),
        Some(COMPLETE_ANALYSIS_SCENARIO)
    );
    assert_eq!(
        selected_driver_scenario(Some(ANALYSIS_RECOVERY_SCENARIO)),
        Some(ANALYSIS_RECOVERY_SCENARIO)
    );
    assert_eq!(
        selected_driver_scenario(Some(RECORD_WITHOUT_PREVIEW_SCENARIO)),
        Some(RECORD_WITHOUT_PREVIEW_SCENARIO)
    );
    assert_eq!(selected_driver_scenario(Some("unknown")), None);
}
