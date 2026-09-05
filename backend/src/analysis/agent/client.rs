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
}

/// OpenAI Responses specialization used by the application analysis pipeline.
pub type OpenAiAgent = Agent<openai::responses_api::ResponsesCompletionModel>;

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

        let mut builder = openai::Client::builder().api_key(config.api_key);
        if let Some(base_url) = config.base_url {
            builder = builder.base_url(base_url);
        }
        let client = builder.build().map_err(ProviderClientError::from)?;
        Ok(Self::new(client.completion_model(profile.model.clone())))
    }
}

impl<M: CompletionModel> Agent<M> {
    /// Wraps a configured model, primarily for alternate providers and deterministic tests.
    pub fn new(model: M) -> Self {
        Self { model }
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
        let response = request.send().await?;
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
#[path = "tests/client.rs"]
mod tests;
