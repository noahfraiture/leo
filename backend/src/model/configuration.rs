use std::{collections::HashSet, fmt};

use serde::{Deserialize, Serialize};

use super::{Error, Result};

/// The explicitly selected OpenAI-compatible wire protocol. No fallback is attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Transport {
    Responses,
    ChatCompletions,
}

/// A named connection in private application settings. Only snapshots belong in saved runs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnection {
    pub id: String,
    pub name: String,
    /// Base URL such as `http://localhost:1234/v1`, without an endpoint suffix.
    pub base_url: String,
    pub transport: Transport,
    /// Stored only with private settings. An empty value supports unauthenticated local servers.
    pub api_key: String,
}

impl fmt::Debug for ProviderConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Even an invalid endpoint may contain an accidentally pasted credential.
        f.debug_struct("ProviderConnection")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}

/// Validated, normalized provider identity captured without its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSnapshot {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub transport: Transport,
}

impl ProviderConnection {
    /// Validates the endpoint and removes its trailing slash for stable run identities.
    pub fn snapshot(&self) -> Result<ProviderSnapshot> {
        for (value, field) in [(&self.id, "provider ID"), (&self.name, "provider name")] {
            if value.trim().is_empty() {
                return Err(Error::BlankField(field));
            }
        }
        let endpoint = url::Url::parse(self.base_url.trim()).map_err(|_| Error::InvalidEndpoint)?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(Error::InvalidEndpoint);
        }
        Ok(ProviderSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            base_url: endpoint.as_str().trim_end_matches('/').to_owned(),
            transport: self.transport,
        })
    }
}

/// A selectable model and its operator-estimated memory allowance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelProfile {
    pub id: String,
    pub name: String,
    pub provider_id: String,
    /// Exact identifier sent to the configured provider, including a quantization suffix if needed.
    pub model: String,
    /// MiB allowed for weights and working memory. Hosted models use zero.
    pub memory_cost_mib: u32,
    /// Maximum candidate and reference images combined in a single request.
    pub image_limit: usize,
    pub max_output_tokens: Option<u64>,
    /// Explicit provider-supported effort, such as "low". None leaves the provider default.
    pub reasoning_effort: Option<String>,
}

impl ModelProfile {
    /// Checks identity fields independently of unrelated configured models.
    pub fn validate(&self) -> Result<()> {
        for (value, field) in [
            (&self.id, "model profile ID"),
            (&self.name, "model profile name"),
            (&self.provider_id, "provider ID"),
            (&self.model, "model identifier"),
        ] {
            if value.trim().is_empty() {
                return Err(Error::BlankField(field));
            }
        }
        if self.image_limit == 0 || self.max_output_tokens == Some(0) {
            return Err(Error::InvalidModelLimits);
        }
        if self
            .reasoning_effort
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(Error::BlankField("reasoning effort"));
        }
        Ok(())
    }

    /// Resolves one profile against exactly one matching provider, without copying its key.
    pub fn resolve(&self, providers: &[ProviderConnection]) -> Result<ModelSnapshot> {
        self.validate()?;
        let mut matches = providers
            .iter()
            .filter(|provider| provider.id == self.provider_id);
        let provider = matches.next().ok_or(Error::UnknownProvider)?;
        if matches.next().is_some() {
            return Err(Error::UnknownProvider);
        }
        Ok(ModelSnapshot {
            profile: self.clone(),
            provider: provider.snapshot()?,
        })
    }
}

/// Exact model and secret-free connection retained with immutable analysis inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSnapshot {
    pub profile: ModelProfile,
    pub provider: ProviderSnapshot,
}

/// Independent visual readers and the models used for review and checklist suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSet {
    pub id: String,
    pub name: String,
    pub reader_ids: Vec<String>,
    pub reviewer_id: String,
    pub formatter_id: String,
}

/// Fully resolved role selections. A role may reuse another role's model profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedModelSet {
    pub id: String,
    pub name: String,
    pub readers: Vec<ModelSnapshot>,
    pub reviewer: ModelSnapshot,
    pub formatter: ModelSnapshot,
}

impl ModelSet {
    /// Validates only this set and its selected models, leaving unrelated sets usable.
    pub fn resolve(
        &self,
        profiles: &[ModelProfile],
        providers: &[ProviderConnection],
    ) -> Result<ResolvedModelSet> {
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err(Error::BlankField("model set ID or name"));
        }
        if self.reader_ids.is_empty()
            || self.reader_ids.iter().collect::<HashSet<_>>().len() != self.reader_ids.len()
        {
            return Err(Error::InvalidReaders);
        }
        let resolve = |id: &String| -> Result<ModelSnapshot> {
            let mut matches = profiles.iter().filter(|profile| profile.id == *id);
            let profile = matches.next().ok_or(Error::UnknownModel)?;
            if matches.next().is_some() {
                return Err(Error::UnknownModel);
            }
            profile.resolve(providers)
        };
        Ok(ResolvedModelSet {
            id: self.id.clone(),
            name: self.name.clone(),
            readers: self.reader_ids.iter().map(resolve).collect::<Result<_>>()?,
            reviewer: resolve(&self.reviewer_id)?,
            formatter: resolve(&self.formatter_id)?,
        })
    }
}

#[cfg(test)]
#[path = "tests/configuration.rs"]
mod tests;
