use std::time::{Duration, Instant};

use rig_core::{
    client::CompletionClient,
    completion::{AssistantContent, CompletionModel},
    providers::openai,
    schemars::JsonSchema,
};
use serde::de::DeserializeOwned;

use super::{
    Completion, Error, ModelProfile, ProviderConnection, Result, StructuredJob, TokenUsage,
    Transport,
};

enum ClientTransport {
    Responses(openai::responses_api::ResponsesCompletionModel),
    Chat(openai::completion::CompletionModel),
}

/// Executes stateless structured requests through exactly the configured provider and transport.
/// Evaluation callers own their authorization and usage journal; this client reads no environment.
pub struct ModelClient {
    transport: ClientTransport,
    timeout: Duration,
    image_limit: usize,
    max_output_tokens: Option<u64>,
    additional_parameters: Option<serde_json::Value>,
}

impl ModelClient {
    /// Resolves a provider explicitly. Credentials are never part of snapshots or client diagnostics.
    pub fn new(
        provider: &ProviderConnection,
        profile: &ModelProfile,
        request_timeout: Duration,
    ) -> Result<Self> {
        profile.validate()?;
        let snapshot = provider.snapshot()?;
        if profile.provider_id != provider.id {
            return Err(Error::ProviderMismatch);
        }
        if request_timeout.is_zero() {
            return Err(Error::InvalidRequest);
        }
        let client = openai::Client::builder()
            .api_key(provider.api_key.clone())
            .base_url(snapshot.base_url)
            .build()
            .map_err(|_| Error::ClientConfiguration)?;
        let transport = match snapshot.transport {
            Transport::Responses => {
                ClientTransport::Responses(client.completion_model(profile.model.clone()))
            }
            Transport::ChatCompletions => ClientTransport::Chat(
                client
                    .completions_api()
                    .completion_model(profile.model.clone()),
            ),
        };
        let additional_parameters = profile
            .reasoning_effort
            .as_ref()
            .map(|effort| match snapshot.transport {
                Transport::Responses => serde_json::json!({"reasoning": {"effort": effort}}),
                Transport::ChatCompletions => serde_json::json!({"reasoning_effort": effort}),
            });
        Ok(Self {
            transport,
            timeout: request_timeout,
            image_limit: profile.image_limit,
            max_output_tokens: profile.max_output_tokens,
            additional_parameters,
        })
    }

    /// Enforces a typed output schema and parses the matching result, without retries or fallback.
    pub async fn complete<T: DeserializeOwned + JsonSchema>(
        &self,
        job: &StructuredJob,
    ) -> Result<Completion<T>> {
        if job.images.len() > self.image_limit
            || matches!((job.max_output_tokens, self.max_output_tokens), (Some(requested), Some(limit)) if requested > limit)
        {
            return Err(Error::RequestExceedsModelLimits);
        }
        let token_limit = job.max_output_tokens.or(self.max_output_tokens);
        match &self.transport {
            ClientTransport::Responses(model) => {
                complete(
                    model,
                    job,
                    self.timeout,
                    token_limit,
                    self.additional_parameters.clone(),
                )
                .await
            }
            ClientTransport::Chat(model) => {
                complete(
                    model,
                    job,
                    self.timeout,
                    token_limit,
                    self.additional_parameters.clone(),
                )
                .await
            }
        }
    }
}

async fn complete<M: CompletionModel, T: DeserializeOwned + JsonSchema>(
    model: &M,
    job: &StructuredJob,
    timeout: Duration,
    token_limit: Option<u64>,
    additional_parameters: Option<serde_json::Value>,
) -> Result<Completion<T>> {
    let mut request = model
        .completion_request(job.message()?)
        .preamble(job.instructions.clone())
        .output_schema(rig_core::schemars::schema_for!(T));
    if let Some(limit) = token_limit {
        request = request.max_tokens(limit);
    }
    if let Some(parameters) = additional_parameters {
        request = request.additional_params(parameters);
    }
    let started = Instant::now();
    let response = tokio::time::timeout(timeout, request.send())
        .await
        .map_err(|_| Error::Timeout)?
        // Raw provider errors may contain credentials, prompts, or returned image bytes.
        .map_err(Error::from)?;
    let text = response
        .choice
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Err(Error::MissingText);
    }
    let value = serde_json::from_str(&text).map_err(|error| Error::InvalidResponse {
        line: error.line(),
        column: error.column(),
    })?;
    let usage = response.usage;
    Ok(Completion {
        value,
        usage: (usage.total_tokens != 0 || usage.input_tokens != 0 || usage.output_tokens != 0)
            .then_some(TokenUsage {
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                total_tokens: usage.total_tokens,
                cached_input_tokens: usage.cached_input_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            }),
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
#[path = "tests/client.rs"]
mod tests;
