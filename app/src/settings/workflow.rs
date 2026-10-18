//! Persisted model connections and run policy for the operator-owned checklist workflow.
use backend::model::{
    ExecutionPolicy, ModelProfile, ModelSet, ModelSnapshot, ProviderConnection, ResolvedModelSet,
    ScheduledJob, plan_execution,
};
use backend::profiles::{ImageDetailPolicy, ImageSizePolicy};
use serde::{Deserialize, Serialize};

/// Sampling limits shared by all readers, so differences in their answers are comparable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceSettings {
    pub max_prompt_span_ms: u64,
    pub overlap_frame_sets: usize,
    pub image_size: ImageSizePolicy,
    pub image_detail: ImageDetailPolicy,
}
impl Default for EvidenceSettings {
    fn default() -> Self {
        Self {
            max_prompt_span_ms: 7000,
            overlap_frame_sets: 2,
            image_size: ImageSizePolicy::MaximumLongEdge(1280),
            image_detail: ImageDetailPolicy::ProviderDefault,
        }
    }
}

/// Named connections and models. Secrets stay in private settings, not run snapshots.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowSettings {
    pub providers: Vec<ProviderConnection>,
    pub models: Vec<ModelProfile>,
    pub model_sets: Vec<ModelSet>,
    pub execution: ExecutionPolicy,
    pub evidence: EvidenceSettings,
}
impl WorkflowSettings {
    /// Resolve one selected profile and check its request policy without inspecting unrelated profiles.
    pub fn resolve_model(&self, id: &str) -> backend::model::Result<ModelSnapshot> {
        let mut matches = self.models.iter().filter(|model| model.id == id);
        let model = matches.next().ok_or(backend::model::Error::UnknownModel)?;
        if matches.next().is_some() {
            return Err(backend::model::Error::UnknownModel);
        }
        let model = model.resolve(&self.providers)?;
        plan_execution(
            &[ScheduledJob {
                id: "selected-model".into(),
                profile_id: model.profile.id.clone(),
            }],
            std::slice::from_ref(&model),
            &self.execution,
        )?;
        Ok(model)
    }

    /// Resolves only the selected set. An unrelated invalid set cannot disable this run.
    pub fn resolve(&self, id: &str) -> Result<ResolvedModelSet, String> {
        self.execution.validate().map_err(|e| e.to_string())?;
        if self.execution.max_concurrent_jobs > 16 || self.execution.max_jobs_per_model > 16 {
            return Err(
                "Concurrent requests and requests per model must be between 1 and 16".into(),
            );
        }
        if self.evidence.max_prompt_span_ms == 0 {
            return Err("Evidence time span must be positive".into());
        }
        let set = self
            .model_sets
            .iter()
            .find(|set| set.id == id)
            .ok_or("Select a model set")?;
        let resolved = set
            .resolve(&self.models, &self.providers)
            .map_err(|e| e.to_string())?;
        if resolved
            .readers
            .iter()
            .chain([&resolved.reviewer, &resolved.formatter])
            .any(|model| model.profile.memory_cost_mib > 49152)
        {
            return Err("Each model memory allowance must be between 0 and 49152 MiB".into());
        }
        Ok(resolved)
    }
}

#[cfg(test)]
#[path = "tests/workflow.rs"]
mod tests;
