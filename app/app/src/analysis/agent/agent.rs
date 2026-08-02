use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig_core::OneOrMany;
use rig_core::client::{CompletionClient, ProviderClient, required_env_var};
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use rig_core::message::{ImageMediaType, UserContent};
use rig_core::providers::openai;
use serde::{Deserialize, Serialize};

use super::error::{Error, Result};

const INSTRUCTIONS: &str = "Analyze the student's video frames against the supplied correct sequence. Describe only visible evidence, preserve timestamps, and update every checklist item.";

/// One JPEG and the human-readable source label shown to the model.
pub(crate) struct PromptFrame {
    pub source: String,
    pub jpeg: Vec<u8>,
}

/// Materialized frames from every available source at one session timestamp.
pub(crate) struct PromptFrameSet {
    /// Session-relative timestamp formatted for the model and its response.
    pub timestamp: String,
    pub frames: Vec<PromptFrame>,
}

/// Ordered frame sets sent together in one model request.
pub(crate) struct AnalysisBatch {
    pub frame_sets: Vec<PromptFrameSet>,
}

/// Everything needed for one stateless model call.
pub(crate) struct AnalysisRequest<'a> {
    pub batch: &'a AnalysisBatch,
    pub checklist: &'a str,
    /// Complete response from the preceding batch, or `None` for the first batch.
    pub previous: Option<&'a AnalysisResponse>,
}

/// A timestamped action or state observed in the current batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(crate) struct Observation {
    pub timestamp: String,
    pub description: String,
}

/// The cumulative assessment of one expected checklist item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(crate) struct ChecklistProgress {
    pub item: String,
    /// Free text such as "respected", "not yet", or "will not be completed".
    pub status: String,
    pub note: String,
}

/// Structured result for one batch plus the cumulative sequence context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub(crate) struct AnalysisResponse {
    pub observations: Vec<Observation>,
    /// Concise rolling context carried into the next batch request.
    pub sequence_summary: String,
    /// Latest status of every item in the correct-sequence checklist.
    pub checklist_progress: Vec<ChecklistProgress>,
}

/// Executes one stateless, structured video-analysis request with a Rig model.
pub(crate) struct Agent<M: CompletionModel> {
    model: M,
}

/// OpenAI Responses specialization used by the application analysis pipeline.
pub(crate) type OpenAiAgent = Agent<openai::responses_api::ResponsesCompletionModel>;

impl OpenAiAgent {
    /// Builds the model from `OPENAI_API_KEY`, `ANALYSIS_MODEL`, and optional `OPENAI_BASE_URL`.
    pub(crate) fn from_env() -> Result<Self> {
        let client = openai::Client::from_env()?;
        let model = required_env_var("ANALYSIS_MODEL")?;

        Ok(Self::new(client.completion_model(model)))
    }
}

impl<M: CompletionModel> Agent<M> {
    /// Wraps a configured model, primarily for alternate providers and deterministic tests.
    pub(crate) fn new(model: M) -> Self {
        Self { model }
    }

    /// Analyzes one materialized batch; callers explicitly provide rolling context.
    pub(crate) async fn analyze(&self, request: AnalysisRequest<'_>) -> Result<AnalysisResponse> {
        let prompt = prompt_message(&request)?;
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

fn prompt_message(request: &AnalysisRequest<'_>) -> Result<Message> {
    let previous = request
        .previous
        .map(serde_json::to_string)
        .transpose()?
        .unwrap_or_else(|| "This is the first batch; there is no previous response.".into());
    let mut content = OneOrMany::one(UserContent::text(format!(
        "Correct sequence checklist:\n{}\n\nPrevious batch response:\n{}",
        request.checklist, previous
    )));

    for frame_set in &request.batch.frame_sets {
        content.push(UserContent::text(format!(
            "Frame set timestamp: {}",
            frame_set.timestamp
        )));

        for frame in &frame_set.frames {
            content.push(UserContent::text(format!(
                "Frame source: {} at {}",
                frame.source, frame_set.timestamp
            )));
            content.push(UserContent::image_base64(
                STANDARD.encode(&frame.jpeg),
                Some(ImageMediaType::JPEG),
                None,
            ));
        }
    }

    Ok(Message::User { content })
}

#[cfg(test)]
mod tests {
    use rig_core::completion::Message;
    use rig_core::message::{DocumentSourceKind, UserContent};
    use rig_core::test_utils::{MockCompletionModel, MockTurn};

    use super::{
        Agent, AnalysisBatch, AnalysisRequest, AnalysisResponse, ChecklistProgress, Error,
        Observation, PromptFrame, PromptFrameSet,
    };

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
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };

        let actual = agent
            .analyze(AnalysisRequest {
                batch: &batch,
                checklist: "Reach for the handle",
                previous: None,
            })
            .await
            .expect("analysis should succeed");

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn request_preserves_previous_response_and_frame_order() {
        let response = AnalysisResponse {
            observations: Vec::new(),
            sequence_summary: "The first step has started.".into(),
            checklist_progress: vec![ChecklistProgress {
                item: "Open the valve".into(),
                status: "in progress".into(),
                note: "The hand is on the valve.".into(),
            }],
        };
        let model = MockCompletionModel::text(
            serde_json::to_string(&response).expect("response should serialize"),
        );
        let recorded_model = model.clone();
        let agent = Agent::new(model);
        let batch = AnalysisBatch {
            frame_sets: vec![
                PromptFrameSet {
                    timestamp: "00:00:01".into(),
                    frames: vec![
                        PromptFrame {
                            source: "camera-b".into(),
                            jpeg: vec![1, 2],
                        },
                        PromptFrame {
                            source: "camera-a".into(),
                            jpeg: vec![3],
                        },
                    ],
                },
                PromptFrameSet {
                    timestamp: "00:00:02".into(),
                    frames: vec![PromptFrame {
                        source: "camera-c".into(),
                        jpeg: vec![4],
                    }],
                },
            ],
        };

        agent
            .analyze(AnalysisRequest {
                batch: &batch,
                checklist: "Open the valve",
                previous: Some(&response),
            })
            .await
            .expect("analysis should succeed");

        let requests = recorded_model.requests();
        let Message::User { content } = requests[0]
            .chat_history
            .iter()
            .last()
            .expect("request should contain a user message")
        else {
            panic!("last request message should be from the user");
        };
        let content = content.iter().collect::<Vec<_>>();

        assert!(matches!(
            content[0],
            UserContent::Text(text)
                if text.text.contains("Open the valve")
                    && text.text.contains("The first step has started.")
        ));
        assert!(matches!(
            content[1],
            UserContent::Text(text) if text.text.contains("00:00:01")
        ));
        assert!(matches!(
            content[2],
            UserContent::Text(text) if text.text.contains("camera-b")
        ));
        assert!(matches!(
            content[3],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("AQI=".into())
        ));
        assert!(matches!(
            content[4],
            UserContent::Text(text) if text.text.contains("camera-a")
        ));
        assert!(matches!(
            content[5],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("Aw==".into())
        ));
        assert!(matches!(
            content[6],
            UserContent::Text(text) if text.text.contains("00:00:02")
        ));
        assert!(matches!(
            content[7],
            UserContent::Text(text) if text.text.contains("camera-c")
        ));
        assert!(matches!(
            content[8],
            UserContent::Image(image)
                if image.data == DocumentSourceKind::Base64("BA==".into())
        ));
    }

    #[tokio::test]
    async fn analyze_rejects_a_response_without_text() {
        let model = MockCompletionModel::new([MockTurn::tool_call(
            "call-1",
            "unexpected_tool",
            serde_json::json!({}),
        )]);
        let agent = Agent::new(model);
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };

        let result = agent
            .analyze(AnalysisRequest {
                batch: &batch,
                checklist: "Start the exercise",
                previous: None,
            })
            .await;

        assert!(matches!(result, Err(Error::MissingTextResponse)));
    }
}
