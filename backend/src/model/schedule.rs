use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Error, ModelSnapshot, ProviderConnection, Result};

/// Operator-configured limits for the current 48 GiB laptop.
/// Memory costs include the operator's allowance for model weights and request working memory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub memory_budget_mib: u32,
    pub max_concurrent_jobs: usize,
    /// Concurrent requests against the same physical model, including aliased profiles.
    pub max_jobs_per_model: usize,
    pub request_timeout_seconds: u64,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            memory_budget_mib: 32 * 1024,
            max_concurrent_jobs: 4,
            max_jobs_per_model: 1,
            request_timeout_seconds: 600,
        }
    }
}

impl ExecutionPolicy {
    /// Rejects unusable bounds before any provider request is sent.
    pub fn validate(&self) -> Result<()> {
        if self.memory_budget_mib > 48 * 1024
            || self.max_concurrent_jobs == 0
            || self.max_jobs_per_model == 0
            || self.request_timeout_seconds == 0
        {
            return Err(Error::InvalidExecutionPolicy);
        }
        Ok(())
    }
}

/// Work ready to run independently. Dependencies must be resolved before calling the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledJob {
    pub id: String,
    pub profile_id: String,
}

/// Physical-model identity used to avoid double counting aliases on the same server.
/// Different server URLs are counted separately; the planner cannot discover URL aliases.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResidencyKey {
    pub base_url: String,
    pub model: String,
}

/// A model's conservative allowance, taking the maximum across profiles sharing its identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResidentModel {
    pub key: ResidencyKey,
    pub memory_cost_mib: u32,
}

/// Jobs that may execute together under the selected concurrency and estimated memory limits.
/// Finishing a wave does not unload a model from an external inference server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionWave {
    pub job_ids: Vec<String>,
    pub resident_models: Vec<ResidentModel>,
    pub memory_cost_mib: u32,
    /// Locally costed models leaving the previous wave. The executor must account for residency
    /// through a supported server policy; this field is not an instruction to invent an unload API.
    pub retire_before: Vec<ResidencyKey>,
}

/// Packs ready jobs into deterministic waves, counting one allocation per physical model.
/// Zero-cost hosted models still consume concurrency slots. These are scheduling estimates,
/// not a measurement or enforcement of process RAM; server eviction must be handled separately.
pub fn plan_execution(
    jobs: &[ScheduledJob],
    models: &[ModelSnapshot],
    policy: &ExecutionPolicy,
) -> Result<Vec<ExecutionWave>> {
    policy.validate()?;
    let mut job_ids = HashSet::new();
    let mut selected = BTreeMap::new();
    let mut costs = BTreeMap::<ResidencyKey, u32>::new();
    for job in jobs {
        if job.id.trim().is_empty() || !job_ids.insert(&job.id) {
            return Err(Error::InvalidJobIdentity);
        }
        let mut matches = models
            .iter()
            .filter(|model| model.profile.id == job.profile_id);
        let model = matches.next().ok_or(Error::UnknownModel)?;
        if matches.next().is_some() {
            return Err(Error::UnknownModel);
        }
        model.profile.validate()?;
        if model.profile.provider_id != model.provider.id {
            return Err(Error::ProviderMismatch);
        }
        let provider = ProviderConnection {
            id: model.provider.id.clone(),
            name: model.provider.name.clone(),
            base_url: model.provider.base_url.clone(),
            transport: model.provider.transport,
            api_key: String::new(),
        }
        .snapshot()?;
        let key = ResidencyKey {
            base_url: provider.base_url,
            model: model.profile.model.clone(),
        };
        let cost = costs.entry(key.clone()).or_default();
        *cost = (*cost).max(model.profile.memory_cost_mib);
        if *cost > policy.memory_budget_mib {
            return Err(Error::ModelExceedsMemoryBudget);
        }
        selected.insert(&job.id, key);
    }

    let mut pending: Vec<_> = jobs.iter().collect();
    let mut waves = Vec::<ExecutionWave>::new();
    while !pending.is_empty() {
        let mut wave = ExecutionWave {
            job_ids: Vec::new(),
            resident_models: Vec::new(),
            memory_cost_mib: 0,
            retire_before: Vec::new(),
        };
        let mut requests_per_model = BTreeMap::<&ResidencyKey, usize>::new();
        let mut deferred = Vec::new();
        for job in pending {
            let key = &selected[&job.id];
            let count = requests_per_model.get(key).copied().unwrap_or(0);
            let additional_cost = if count == 0 { costs[key] } else { 0 };
            if wave.job_ids.len() >= policy.max_concurrent_jobs
                || count >= policy.max_jobs_per_model
                || additional_cost > policy.memory_budget_mib - wave.memory_cost_mib
            {
                deferred.push(job);
                continue;
            }
            wave.job_ids.push(job.id.clone());
            wave.memory_cost_mib += additional_cost;
            requests_per_model.insert(key, count + 1);
            if count == 0 {
                wave.resident_models.push(ResidentModel {
                    key: key.clone(),
                    memory_cost_mib: costs[key],
                });
            }
        }
        if let Some(previous) = waves.last() {
            wave.retire_before = previous
                .resident_models
                .iter()
                .filter(|model| {
                    model.memory_cost_mib > 0 && !requests_per_model.contains_key(&model.key)
                })
                .map(|model| model.key.clone())
                .collect();
        }
        waves.push(wave);
        pending = deferred;
    }
    Ok(waves)
}

#[cfg(test)]
#[path = "tests/schedule.rs"]
mod tests;
