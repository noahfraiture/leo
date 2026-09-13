use super::*;

fn provider() -> ProviderConnection {
    ProviderConnection {
        id: "provider".into(),
        name: "Local server".into(),
        base_url: "http://localhost:1234/v1/".into(),
        transport: Transport::ChatCompletions,
        api_key: "private-test-key".into(),
    }
}

fn profile() -> ModelProfile {
    ModelProfile {
        id: "reader".into(),
        name: "Reader".into(),
        provider_id: "provider".into(),
        model: "example-model".into(),
        memory_cost_mib: 8192,
        image_limit: 16,
        max_output_tokens: None,
        reasoning_effort: None,
    }
}

#[test]
fn snapshot_and_debug_do_not_contain_provider_secrets() {
    let provider = provider();
    let snapshot = provider.snapshot().unwrap();
    assert_eq!(snapshot.base_url, "http://localhost:1234/v1");
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains(&provider.api_key)
    );
    assert!(!format!("{provider:?}").contains(&provider.api_key));
}

#[test]
fn endpoint_rejects_embedded_credentials_and_secret_query_parameters() {
    for endpoint in [
        "https://user:secret@example.com/v1",
        "https://example.com/v1?api_key=secret",
        "https://example.com/v1#secret",
        "file:///tmp/model",
    ] {
        let provider = ProviderConnection {
            base_url: endpoint.into(),
            ..provider()
        };
        assert!(matches!(provider.snapshot(), Err(Error::InvalidEndpoint)));
        assert!(!format!("{provider:?}").contains("secret"));
    }
}

#[test]
fn valid_set_ignores_an_unrelated_invalid_model() {
    let valid = profile();
    let invalid = ModelProfile {
        id: "unrelated".into(),
        provider_id: "absent".into(),
        ..profile()
    };
    let set = ModelSet {
        id: "set".into(),
        name: "Local set".into(),
        reader_ids: vec![valid.id.clone()],
        reviewer_id: valid.id.clone(),
        formatter_id: valid.id.clone(),
    };
    let resolved = set.resolve(&[valid, invalid], &[provider()]).unwrap();
    assert_eq!(resolved.readers[0], resolved.reviewer);
    assert_eq!(resolved.formatter.profile.id, "reader");
}

#[test]
fn repeated_reader_does_not_masquerade_as_an_independent_reader() {
    let set = ModelSet {
        id: "set".into(),
        name: "Local set".into(),
        reader_ids: vec!["reader".into(), "reader".into()],
        reviewer_id: "reader".into(),
        formatter_id: "reader".into(),
    };
    assert!(matches!(
        set.resolve(&[profile()], &[provider()]),
        Err(Error::InvalidReaders)
    ));
}

#[test]
fn duplicate_selected_provider_is_not_resolved_arbitrarily() {
    assert!(matches!(
        profile().resolve(&[provider(), provider()]),
        Err(Error::UnknownProvider)
    ));
}
