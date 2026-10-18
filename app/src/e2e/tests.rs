use backend::model::{ProviderConnection, Transport};

use super::{
    ANALYSIS_RECOVERY_SCENARIO, COMPLETE_ANALYSIS_SCENARIO, RECORD_WITHOUT_PREVIEW_SCENARIO,
    loopback_proxy_is_bypassed, provider_configuration_is_safe, selected_driver_scenario,
};

fn provider(base_url: &str, key: &str) -> ProviderConnection {
    ProviderConnection {
        id: "test".into(),
        name: "Test".into(),
        base_url: base_url.into(),
        transport: Transport::Responses,
        api_key: key.into(),
    }
}

#[test]
fn model_connections_require_loopback_or_explicit_direct_openai_test_opt_in() {
    for endpoint in ["http://127.42.0.1:3000/v1", "http://[::1]:3000/v1"] {
        assert!(provider_configuration_is_safe(
            &provider(endpoint, ""),
            None,
            None
        ));
    }
    let openai = provider("https://api.openai.com/v1", "test-key");
    assert!(provider_configuration_is_safe(
        &openai,
        Some("1"),
        Some("1")
    ));
    assert!(!provider_configuration_is_safe(&openai, Some("1"), None));
    assert!(!provider_configuration_is_safe(&openai, None, Some("1")));
    for endpoint in [
        "http://localhost:3000/v1",
        "https://example.com/v1",
        "https://api.openai.com/v1/other",
        "https://secret@api.openai.com/v1",
    ] {
        assert!(!provider_configuration_is_safe(
            &provider(endpoint, "key"),
            Some("1"),
            Some("1")
        ));
    }
    assert!(!provider_configuration_is_safe(
        &provider("https://api.openai.com/v1", ""),
        Some("1"),
        Some("1")
    ));
}

#[test]
fn loopback_requests_cannot_escape_through_an_inherited_proxy() {
    let local = "http://127.0.0.1:3000/v1";
    assert!(loopback_proxy_is_bypassed(local, false, None));
    assert!(loopback_proxy_is_bypassed(local, true, Some("*, .local")));
    assert!(loopback_proxy_is_bypassed(
        "https://api.openai.com/v1",
        true,
        None
    ));
    assert!(!loopback_proxy_is_bypassed(local, true, None));
    assert!(!loopback_proxy_is_bypassed(
        local,
        true,
        Some("127.0.0.1, localhost")
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
    assert_eq!(
        selected_driver_scenario(Some("camera-events")),
        Some("camera-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("profile-events")),
        Some("profile-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("model-set-events")),
        Some("model-set-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("settings-text")),
        Some("settings-text")
    );
    assert_eq!(
        selected_driver_scenario(Some("review-text")),
        Some("review-text")
    );
    assert_eq!(
        selected_driver_scenario(Some("question-events")),
        Some("question-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("preparation-navigation")),
        Some("preparation-navigation")
    );
    assert_eq!(
        selected_driver_scenario(Some("rapid-exercise-input")),
        Some("rapid-exercise-input")
    );
    assert_eq!(
        selected_driver_scenario(Some("checklist-replacement")),
        Some("checklist-replacement")
    );
    assert_eq!(
        selected_driver_scenario(Some("checklist-paste-events")),
        Some("checklist-paste-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("checklist-delete-events")),
        Some("checklist-delete-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("checklist-publish-events")),
        Some("checklist-publish-events")
    );
    assert_eq!(
        selected_driver_scenario(Some("exercise-recovery")),
        Some("exercise-recovery")
    );
}
