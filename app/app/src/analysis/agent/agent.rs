use rig_core::client::{CompletionClient, ProviderClient, required_env_var};
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use rig_core::providers::openai;
use serde::{Deserialize, Serialize};

use super::error::{Error, Result};

const INSTRUCTIONS: &str = "Analyze the student's video frames against the supplied correct sequence. Describe only visible evidence, preserve timestamps, and update every checklist item.";

/// A timestamped action or state observed in the current batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(in crate::analysis) struct Observation {
    /// Session-relative timestamp supplied with the observed frame set.
    pub(in crate::analysis) timestamp: String,
    /// Visible evidence reported by the model.
    pub(in crate::analysis) description: String,
}

/// The cumulative assessment of one expected checklist item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(in crate::analysis) struct ChecklistProgress {
    /// Expected checklist item being assessed.
    pub(in crate::analysis) item: String,
    /// Free text such as "respected", "not yet", or "will not be completed".
    pub(in crate::analysis) status: String,
    /// Evidence or rationale supporting the current status.
    pub(in crate::analysis) note: String,
}

/// Structured result for one batch plus the cumulative sequence context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(in crate::analysis) struct AnalysisResponse {
    /// Evidence observed in the current batch.
    pub(in crate::analysis) observations: Vec<Observation>,
    /// Concise rolling context carried into the next batch request.
    pub(in crate::analysis) sequence_summary: String,
    /// Latest status of every item in the correct-sequence checklist.
    pub(in crate::analysis) checklist_progress: Vec<ChecklistProgress>,
}

/// Executes one stateless, structured video-analysis request with a Rig model.
pub(in crate::analysis) struct Agent<M: CompletionModel> {
    model: M,
}

/// OpenAI Responses specialization used by the application analysis pipeline.
pub(in crate::analysis) type OpenAiAgent = Agent<openai::responses_api::ResponsesCompletionModel>;

impl OpenAiAgent {
    /// Builds the model from `OPENAI_API_KEY`, `ANALYSIS_MODEL`, and optional `OPENAI_BASE_URL`.
    pub(in crate::analysis) fn from_env() -> Result<Self> {
        let client = openai::Client::from_env()?;
        let model = required_env_var("ANALYSIS_MODEL")?;

        Ok(Self::new(client.completion_model(model)))
    }
}

impl<M: CompletionModel> Agent<M> {
    /// Wraps a configured model, primarily for alternate providers and deterministic tests.
    pub(in crate::analysis) fn new(model: M) -> Self {
        Self { model }
    }

    /// Sends one prebuilt prompt and parses its structured analysis response.
    pub(in crate::analysis) async fn analyze(&self, prompt: Message) -> Result<AnalysisResponse> {
        let response = self
            .model
            .completion_request(prompt)
            .preamble(INSTRUCTIONS.to_owned())
            .output_schema(rig_core::schemars::schema_for!(AnalysisResponse))
            .send()
            .await?;
        let text = response
            .choice
            .iter()
            .filter_map(|content| match content {
                AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if text.is_empty() {
            return Err(Error::MissingTextResponse);
        }

        Ok(serde_json::from_str(&text)?)
    }
}

#[cfg(test)]
mod tests {
    use rig_core::test_utils::{MockCompletionModel, MockTurn};

    use super::{Agent, AnalysisResponse, ChecklistProgress, Error, Message, Observation};

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
}
