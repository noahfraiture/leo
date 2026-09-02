use rig_core::test_utils::{MockCompletionModel, MockTurn};

use super::{
    Agent, AnalysisResponse, ChecklistProgress, Error, Message, Observation, OpenAiAgent,
    OpenAiConfig,
};

fn openai_config(api_key: &str, model: &str) -> OpenAiConfig {
    OpenAiConfig {
        api_key: api_key.into(),
        model: model.into(),
        base_url: None,
    }
}

#[test]
fn explicit_provider_configuration_rejects_blank_values() {
    assert!(matches!(
        OpenAiAgent::from_config(openai_config("", "model")),
        Err(Error::BlankConfiguration("OpenAI API key"))
    ));
    assert!(matches!(
        OpenAiAgent::from_config(openai_config("key", "  ")),
        Err(Error::BlankConfiguration("OpenAI model"))
    ));
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
        .analyze(Message::user("prebuilt analysis prompt"))
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
        .analyze(Message::user("prebuilt analysis prompt"))
        .await;

    assert!(matches!(result, Err(Error::MissingTextResponse)));
}
