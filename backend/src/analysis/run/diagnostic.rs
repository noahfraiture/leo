use std::{path::PathBuf, time::Duration};

use serde_json::{Value, json};
use uuid::Uuid;

use super::{JobKind, RunError, RunResult, RunStore, jobs, selection_aliases, store};
use crate::model::ProviderConnection;

/// Explicitly authorized text-only replay of one saved selector, isolated from durable run state.
pub struct SelectorDiagnostic {
    pub session_directory: PathBuf,
    pub run_id: Uuid,
    pub job_id: String,
    pub provider: ProviderConnection,
    pub contract_version: u8,
}

/// Uses production prompts and validators, returning an artifact without saving into the source run.
/// Callers own approval and cost accounting. This function never reads credentials or environment.
pub async fn diagnose_selector(request: SelectorDiagnostic) -> RunResult<Value> {
    let store = RunStore::new(&request.session_directory);
    let preparation_store = store.clone();
    let run_id = request.run_id;
    let job_id = request.job_id;
    let (snapshot, job, prepared) = tokio::task::spawn_blocking(move || {
        let mut snapshot = preparation_store.load(run_id)?;
        snapshot.inputs.prompt_version = match request.contract_version {
            1 => selection_aliases::LEGACY_PROMPT,
            2 => selection_aliases::ALIASED_PROMPT,
            _ => return Err(RunError::Invalid("Unknown diagnostic contract".into())),
        }
        .into();
        let job = snapshot
            .checkpoint
            .jobs
            .iter()
            .find(|job| job.id == job_id && job.kind == JobKind::SelectEvidence)
            .ok_or_else(|| RunError::Invalid("Select a saved selector job".into()))?
            .clone();
        let prepared = jobs::prepare(&preparation_store, &snapshot, &job)?;
        Ok::<_, RunError>((snapshot, job, prepared))
    })
    .await??;
    let model = &snapshot.inputs.model_set.reviewer;
    let mut artifact = json!({
        "sourceRunId":run_id,"sourceJobId":job.id,"criterionId":job.criterion_id,
        "promptVersion":snapshot.inputs.prompt_version,"model":model,
        "request":{"schema":if request.contract_version == 1 {
            serde_json::to_value(rig_core::schemars::schema_for!(crate::analysis::EvidenceSelection))?
        } else {
            serde_json::to_value(rig_core::schemars::schema_for!(super::selection_wire::SelectionWire))?
        }, "instructions":prepared.request.instructions,
            "document":serde_json::from_str::<Value>(&prepared.request.text)?,
            "maxOutputTokens":model.profile.max_output_tokens},
        "strictSuccess":false
    });
    match jobs::complete(
        &job,
        model,
        &request.provider,
        Duration::from_secs(snapshot.inputs.execution.request_timeout_seconds),
        prepared,
    )
    .await
    {
        Ok(response) => {
            let validation = store::validate_response(&job, &response).and_then(|()| {
                let canonical = match &response.output {
                    store::JobOutput::Selection(value) => value.clone(),
                    store::JobOutput::SelectionV2(value) => value.canonical(&job)?,
                    _ => {
                        return Err(RunError::Invalid(
                            "Unexpected diagnostic response kind".into(),
                        ));
                    }
                };
                store::validate_selection_sources(&snapshot.checkpoint, &job, &canonical.value)?;
                Ok(canonical)
            });
            artifact["response"] = serde_json::to_value(&response)?;
            match validation {
                Ok(canonical) => {
                    artifact["strictSuccess"] = json!(true);
                    artifact["canonicalSelection"] = serde_json::to_value(canonical)?;
                }
                Err(error) => artifact["validationError"] = json!(error.to_string()),
            }
        }
        Err(error) => artifact["requestError"] = json!(error.to_string()),
    }
    Ok(artifact)
}
