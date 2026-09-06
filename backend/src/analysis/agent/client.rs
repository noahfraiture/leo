use rig_core::client::{CompletionClient, ProviderClientError};
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use rig_core::providers::openai;
use serde::{Deserialize, Serialize};

use crate::profiles::AnalysisProfile;

use super::error::{Error, Result};

const INSTRUCTIONS: &str = "Analyze the student's video frames against the supplied correct sequence. Describe only visible evidence, preserve timestamps, and update every checklist item.";

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
    #[cfg(feature = "test-support")]
    metering: Option<super::metering::Metering>,
    #[cfg(feature = "test-support")]
    additional_parameters: Option<serde_json::Value>,
}

/// OpenAI Responses specialization used by the application analysis pipeline.
pub type OpenAiAgent = Agent<openai::responses_api::ResponsesCompletionModel>;

/// Local evaluation transport for servers that enforce schemas only through Chat Completions.
#[cfg(feature = "test-support")]
pub type LocalChatAgent = Agent<openai::completion::CompletionModel>;

#[cfg(feature = "test-support")]
impl LocalChatAgent {
    /// Uses a loopback server while retaining the same production prompt and JSON parser.
    pub fn from_local_config(
        config: OpenAiConfig,
        profile: &AnalysisProfile,
        reasoning_effort: Option<String>,
    ) -> Result<Self> {
        let base = config.base_url.as_deref().ok_or(Error::LocalEndpoint)?;
        let url = url::Url::parse(base).map_err(|_| Error::LocalEndpoint)?;
        if !matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost")) {
            return Err(Error::LocalEndpoint);
        }
        let metering = super::metering::Metering::from_environment(profile, &config)?;
        let client = openai::Client::builder()
            .api_key(config.api_key)
            .base_url(base)
            .build()
            .map_err(ProviderClientError::from)?
            .completions_api();
        Ok(Self {
            metering,
            additional_parameters: reasoning_effort
                .map(|effort| serde_json::json!({"reasoning_effort": effort})),
            ..Self::new(client.completion_model(profile.model.clone()))
        })
    }
}

/// Explicit OpenAI configuration captured before an analysis starts.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiConfig {
    pub api_key: String,
    pub base_url: Option<String>,
}

impl OpenAiAgent {
    /// Builds one model from an explicit application-owned configuration.
    pub fn from_config(config: OpenAiConfig, profile: &AnalysisProfile) -> Result<Self> {
        if config.api_key.trim().is_empty() {
            return Err(Error::BlankConfiguration("OpenAI API key"));
        }
        if profile.model.trim().is_empty() {
            return Err(Error::BlankConfiguration("OpenAI model"));
        }

        #[cfg(feature = "test-support")]
        let metering = super::metering::Metering::from_environment(profile, &config)?;
        #[cfg(feature = "test-support")]
        let additional_parameters = (metering.is_some() && config.base_url.is_none())
            .then(|| serde_json::json!({"service_tier": "default"}));
        let mut builder = openai::Client::builder().api_key(config.api_key);
        if let Some(base_url) = config.base_url {
            builder = builder.base_url(base_url);
        }
        let client = builder.build().map_err(ProviderClientError::from)?;
        let agent = Self::new(client.completion_model(profile.model.clone()));
        #[cfg(feature = "test-support")]
        let agent = Self {
            metering,
            additional_parameters,
            ..agent
        };
        Ok(agent)
    }
}

impl<M: CompletionModel> Agent<M> {
    /// Wraps a configured model, primarily for alternate providers and deterministic tests.
    pub fn new(model: M) -> Self {
        Self {
            model,
            #[cfg(feature = "test-support")]
            metering: None,
            #[cfg(feature = "test-support")]
            additional_parameters: None,
        }
    }

    /// Sends one prebuilt prompt and parses its structured analysis response.
    pub async fn analyze(
        &self,
        prompt: Message,
        max_output_tokens: Option<u64>,
    ) -> Result<AnalysisResponse> {
        let mut request = self
            .model
            .completion_request(prompt)
            .preamble(INSTRUCTIONS.to_owned())
            .output_schema(rig_core::schemars::schema_for!(AnalysisResponse));
        if let Some(limit) = max_output_tokens {
            request = request.max_tokens(limit);
        }
        #[cfg(feature = "test-support")]
        if let Some(parameters) = &self.additional_parameters {
            request = request.additional_params(parameters.clone());
        }
        #[cfg(feature = "test-support")]
        let measurement = self
            .metering
            .as_ref()
            .map(|meter| meter.start())
            .transpose()?;
        let response = request.send().await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                #[cfg(feature = "test-support")]
                if let Some(measurement) = measurement {
                    measurement.finish::<M::Response>(None, None)?;
                }
                return Err(error.into());
            }
        };
        let text = response
            .choice
            .iter()
            .filter_map(|content| match content {
                AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let parsed = if text.is_empty() {
            Err(Error::MissingTextResponse)
        } else {
            serde_json::from_str(&text).map_err(Error::from)
        };
        #[cfg(feature = "test-support")]
        if let Some(measurement) = measurement {
            // Invalid JSON still incurs provider usage. Preserve both outcomes in the journal.
            measurement.finish(Some(&response), Some(parsed.is_ok()))?;
        }
        parsed
    }
}

#[cfg(test)]
#[path = "tests/client.rs"]
mod tests;
