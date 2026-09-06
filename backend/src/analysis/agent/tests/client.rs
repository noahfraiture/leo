use rig_core::test_utils::{MockCompletionModel, MockTurn};

use super::{
    Agent, AnalysisResponse, ChecklistProgress, Error, Message, Observation, OpenAiAgent,
    OpenAiConfig,
};

fn openai_config(api_key: &str) -> OpenAiConfig {
    OpenAiConfig {
        api_key: api_key.into(),
        base_url: None,
    }
}

#[test]
fn explicit_provider_configuration_rejects_blank_values() {
    assert!(matches!(
        OpenAiAgent::from_config(openai_config(""), &crate::tests::analysis_profile(5, 0)),
        Err(Error::BlankConfiguration("OpenAI API key"))
    ));
    assert!(matches!(
        OpenAiAgent::from_config(
            openai_config("key"),
            &crate::profiles::AnalysisProfile {
                model: "  ".into(),
                ..crate::tests::analysis_profile(5, 0)
            }
        ),
        Err(Error::BlankConfiguration("OpenAI model"))
    ));
}

#[cfg(feature = "test-support")]
#[test]
fn local_chat_evaluation_rejects_paid_and_remote_endpoints() {
    for base_url in [
        None,
        Some("https://api.openai.com/v1"),
        Some("http://192.0.2.1/v1"),
    ] {
        let config = OpenAiConfig {
            api_key: "local".into(),
            base_url: base_url.map(str::to_owned),
        };
        assert!(matches!(
            super::LocalChatAgent::from_local_config(
                config,
                &crate::tests::analysis_profile(5, 0),
                None
            ),
            Err(Error::LocalEndpoint)
        ));
    }
}

#[test]
fn response_schema_has_the_three_analysis_sections() {
    let schema = serde_json::to_value(rig_core::schemars::schema_for!(AnalysisResponse))
        .expect("schema should serialize");
    let properties = schema["properties"]
        .as_object()
        .expect("response schema should be an object");

    assert!(properties.contains_key("observations"));
    assert!(properties.contains_key("sequence_summary"));
    assert!(properties.contains_key("checklist_progress"));
}

#[tokio::test]
async fn analyze_deserializes_the_structured_response() {
    let expected = AnalysisResponse {
        observations: vec![Observation {
            timestamp: "00:00:03".into(),
            description: "The student reaches for the handle.".into(),
        }],
        sequence_summary: "The exercise has started.".into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Reach for the handle".into(),
            status: "respected".into(),
            note: "Observed at 00:00:03".into(),
        }],
    };
    let model = MockCompletionModel::text(
        serde_json::to_string(&expected).expect("response should serialize"),
    );
    let agent = Agent::new(model);

    let actual = agent
        .analyze(Message::user("prebuilt analysis prompt"), None)
        .await
        .expect("analysis should succeed");

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn analyze_rejects_a_response_without_text() {
    let model = MockCompletionModel::new([MockTurn::tool_call(
        "call-1",
        "unexpected_tool",
        serde_json::json!({}),
    )]);
    let agent = Agent::new(model);

    let result = agent
        .analyze(Message::user("prebuilt analysis prompt"), None)
        .await;

    assert!(matches!(result, Err(Error::MissingTextResponse)));
}
