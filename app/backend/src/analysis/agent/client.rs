use rig_core::client::{CompletionClient, ProviderClientError, optional_env_var, required_env_var};
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use rig_core::providers::openai;
use serde::{Deserialize, Serialize};

use crate::analysis::analyzer::AnalysisIdentity;

use super::error::{Error, Result};

const INSTRUCTIONS: &str = "Analyze the student's video frames against the supplied correct sequence. Describe only visible evidence, preserve timestamps, and update every checklist item.";
const OPENAI_PUBLIC_ENDPOINT_ID: &str = "openai-public";

/// A timestamped action or state observed in the current batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub struct Observation {
    /// Session-relative timestamp supplied with the observed frame set.
    pub timestamp: String,
    /// Visible evidence reported by the model.
    pub description: String,
}

/// The cumulative assessment of one expected checklist item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub struct ChecklistProgress {
    /// Expected checklist item being assessed.
    pub item: String,
    /// Free text such as "respected", "not yet", or "will not be completed".
    pub status: String,
    /// Evidence or rationale supporting the current status.
    pub note: String,
}

/// Structured result for one batch plus the cumulative sequence context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, rig_core::schemars::JsonSchema)]
#[schemars(crate = "rig_core::schemars")]
pub struct AnalysisResponse {
    /// Evidence observed in the current batch.
    pub observations: Vec<Observation>,
    /// Concise rolling context carried into the next batch request.
    pub sequence_summary: String,
    /// Latest status of every item in the correct-sequence checklist.
    pub checklist_progress: Vec<ChecklistProgress>,
}

/// Executes one stateless, structured video-analysis request with a Rig model.
pub struct Agent<M: CompletionModel> {
    model: M,
}

/// OpenAI Responses specialization used by the application analysis pipeline.
pub type OpenAiAgent = Agent<openai::responses_api::ResponsesCompletionModel>;

/// Validated OpenAI settings whose only inspectable state is the non-secret analysis identity.
pub struct OpenAiConfiguration {
    api_key: String,
    base_url: Option<String>,
    identity: AnalysisIdentity,
}

impl OpenAiConfiguration {
    /// Reads and validates the OpenAI analysis configuration from the process environment once.
    pub fn from_env() -> Result<Self> {
        let api_key = required_env_var("OPENAI_API_KEY")?;
        let model = required_env_var("ANALYSIS_MODEL")?;
        let base_url = optional_env_var("OPENAI_BASE_URL")?;
        let endpoint_id = match base_url.as_deref() {
            Some(base_url) if configuration_value_is_present(base_url) => {
                optional_env_var("ANALYSIS_ENDPOINT_ID")?
            }
            _ => None,
        };

        Self::from_values(api_key, model, base_url, endpoint_id)
    }

    fn from_values(
        api_key: String,
        model: String,
        base_url: Option<String>,
        endpoint_id: Option<String>,
    ) -> Result<Self> {
        if !configuration_value_is_present(&api_key) {
            return Err(Error::BlankConfiguration("OPENAI_API_KEY"));
        }
        if !configuration_value_is_present(&model) {
            return Err(Error::BlankConfiguration("ANALYSIS_MODEL"));
        }

        let endpoint_id = match base_url.as_deref() {
            None => OPENAI_PUBLIC_ENDPOINT_ID.to_owned(),
            Some(base_url) if !configuration_value_is_present(base_url) => {
                return Err(Error::BlankConfiguration("OPENAI_BASE_URL"));
            }
            Some(_) => {
                let endpoint_id =
                    endpoint_id.ok_or(Error::MissingConfiguration("ANALYSIS_ENDPOINT_ID"))?;
                if !configuration_value_is_present(&endpoint_id) {
                    return Err(Error::BlankConfiguration("ANALYSIS_ENDPOINT_ID"));
                }
                endpoint_id
            }
        };

        Ok(Self {
            api_key,
            base_url,
            identity: AnalysisIdentity { model, endpoint_id },
        })
    }

    /// Returns the safe identity corresponding exactly to this configuration.
    pub fn identity(&self) -> &AnalysisIdentity {
        &self.identity
    }

    /// Consumes this configuration to build the corresponding Rig OpenAI agent.
    pub fn into_agent(self) -> Result<OpenAiAgent> {
        let mut builder = openai::Client::builder().api_key(self.api_key);
        if let Some(base_url) = self.base_url {
            builder = builder.base_url(base_url);
        }
        let client = builder.build().map_err(ProviderClientError::from)?;

        Ok(Agent::new(client.completion_model(self.identity.model)))
    }
}

fn configuration_value_is_present(value: &str) -> bool {
    !value.trim().is_empty()
}

impl<M: CompletionModel> Agent<M> {
    /// Wraps a configured model, primarily for alternate providers and deterministic tests.
    pub fn new(model: M) -> Self {
        Self { model }
    }

    /// Sends one prebuilt prompt and parses its structured analysis response.
    pub async fn analyze(&self, prompt: Message) -> Result<AnalysisResponse> {
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
    use uuid::Uuid;

    use crate::analysis::{
        analyzer::{ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint},
        video::AnalysisWarning,
    };

    use super::{
        Agent, AnalysisResponse, ChecklistProgress, Error, Message, Observation,
        OpenAiConfiguration, configuration_value_is_present,
    };

    const AUTHORIZATION_MATERIAL_SENTINEL: &str = "authorization-material-sentinel-task-35";
    const URL_USERINFO_SENTINEL: &str = "url-userinfo-sentinel-task-35";
    const URL_QUERY_SENTINEL: &str = "url-query-sentinel-task-35";
    const URL_FRAGMENT_SENTINEL: &str = "url-fragment-sentinel-task-35";
    const BASE_URL_SENTINEL: &str = "https://url-userinfo-sentinel-task-35:credential@raw-base-url-sentinel.invalid/v1?token=url-query-sentinel-task-35#url-fragment-sentinel-task-35";

    #[test]
    fn configuration_builds_exact_non_secret_identity() {
        const MODEL: &str = "  model-byte-sentinel\t";
        const ENDPOINT_ID: &str = "  endpoint-byte-sentinel\t";

        let public = OpenAiConfiguration::from_values(
            AUTHORIZATION_MATERIAL_SENTINEL.to_owned(),
            MODEL.to_owned(),
            None,
            Some("ignored-without-custom-base".to_owned()),
        )
        .expect("public OpenAI configuration should be valid");
        assert_eq!(public.identity().model, MODEL);
        assert_eq!(public.identity().endpoint_id, "openai-public");

        let custom = OpenAiConfiguration::from_values(
            AUTHORIZATION_MATERIAL_SENTINEL.to_owned(),
            MODEL.to_owned(),
            Some(BASE_URL_SENTINEL.to_owned()),
            Some(ENDPOINT_ID.to_owned()),
        )
        .expect("custom OpenAI configuration should be valid");
        assert_eq!(custom.identity().model, MODEL);
        assert_eq!(custom.identity().endpoint_id, ENDPOINT_ID);

        let checkpoint = AnalysisCheckpoint {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            session_id: Uuid::nil(),
            analysis_identity: custom.identity().clone(),
            checklist: "test-checklist".into(),
            plan_fingerprint: "test-plan".into(),
            total_batches: 1,
            warnings: vec![AnalysisWarning::RecordingGap {
                camera_id: 2,
                start_offset_ms: 1_000,
                end_offset_ms: 2_000,
            }],
            responses: vec![AnalysisResponse {
                observations: vec![Observation {
                    timestamp: "00:00:01.000".into(),
                    description: "persisted observation".into(),
                }],
                sequence_summary: "persisted sequence result".into(),
                checklist_progress: vec![ChecklistProgress {
                    item: "test-checklist".into(),
                    status: "respected".into(),
                    note: "persisted result".into(),
                }],
            }],
        };
        let checkpoint_bytes =
            serde_json::to_vec(&checkpoint).expect("checkpoint should serialize");
        let checkpoint_json =
            std::str::from_utf8(&checkpoint_bytes).expect("checkpoint should be UTF-8");
        assert!(
            checkpoint_json
                .contains(&serde_json::to_string(MODEL).expect("model should serialize as JSON"))
        );
        assert!(checkpoint_json.contains(
            &serde_json::to_string(ENDPOINT_ID).expect("endpoint ID should serialize as JSON")
        ));
        for sensitive in [
            AUTHORIZATION_MATERIAL_SENTINEL,
            BASE_URL_SENTINEL,
            URL_USERINFO_SENTINEL,
            URL_QUERY_SENTINEL,
            URL_FRAGMENT_SENTINEL,
        ] {
            assert!(
                !checkpoint_json.contains(sensitive),
                "checkpoint contained sensitive configuration bytes: {sensitive}"
            );
        }
        let saved: serde_json::Value =
            serde_json::from_slice(&checkpoint_bytes).expect("checkpoint should be valid JSON");
        assert_eq!(saved["checklist"], "test-checklist");
        assert_eq!(
            saved["responses"][0]["sequence_summary"],
            "persisted sequence result"
        );
        assert_eq!(
            saved["responses"][0]["checklist_progress"][0]["note"],
            "persisted result"
        );

        let agent = custom
            .into_agent()
            .expect("configuration should build the OpenAI agent");
        assert_eq!(agent.model.model, MODEL);
    }

    #[test]
    fn custom_base_requires_nonblank_endpoint_id() {
        let invalid = [
            (
                "blank API key",
                " \t",
                "model",
                None,
                None,
                "analysis configuration OPENAI_API_KEY must not be blank",
            ),
            (
                "blank model",
                AUTHORIZATION_MATERIAL_SENTINEL,
                " \t",
                None,
                None,
                "analysis configuration ANALYSIS_MODEL must not be blank",
            ),
            (
                "blank custom base",
                AUTHORIZATION_MATERIAL_SENTINEL,
                "model",
                Some(" \t"),
                Some("endpoint"),
                "analysis configuration OPENAI_BASE_URL must not be blank",
            ),
            (
                "missing endpoint ID",
                AUTHORIZATION_MATERIAL_SENTINEL,
                "model",
                Some(BASE_URL_SENTINEL),
                None,
                "analysis configuration ANALYSIS_ENDPOINT_ID is required",
            ),
            (
                "blank endpoint ID",
                AUTHORIZATION_MATERIAL_SENTINEL,
                "model",
                Some(BASE_URL_SENTINEL),
                Some(" \t"),
                "analysis configuration ANALYSIS_ENDPOINT_ID must not be blank",
            ),
        ];

        for (reason, api_key, model, base_url, endpoint_id, expected) in invalid {
            let result = OpenAiConfiguration::from_values(
                api_key.to_owned(),
                model.to_owned(),
                base_url.map(str::to_owned),
                endpoint_id.map(str::to_owned),
            );
            let Err(error) = result else {
                panic!("{reason} should be rejected");
            };
            let message = error.to_string();
            assert_eq!(message, expected, "{reason}");
            assert!(
                !message.contains(AUTHORIZATION_MATERIAL_SENTINEL),
                "{reason}"
            );
            assert!(!message.contains(BASE_URL_SENTINEL), "{reason}");
        }
    }

    #[test]
    fn provider_configuration_rejects_blank_values() {
        assert!(!configuration_value_is_present(""));
        assert!(!configuration_value_is_present("  \t"));
        assert!(configuration_value_is_present("configured"));
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
}
