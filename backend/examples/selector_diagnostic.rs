//! Explicitly gated, text-only comparison of saved selector contracts. Never mutates source runs.
use std::{fs, path::PathBuf};

use backend::{
    analysis::{JobKind, RunStore, SelectorDiagnostic, diagnose_selector},
    model::ProviderConnection,
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    session_directory: PathBuf,
    run_id: Uuid,
    settings_file: PathBuf,
    output_directory: PathBuf,
    contract_versions: Vec<u8>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref() != Ok("1") {
        return Err("Explicit paid selector-diagnostic approval is required".into());
    }
    let path = std::env::args()
        .nth(1)
        .ok_or("Pass one diagnostic manifest path")?;
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    if !manifest.output_directory.is_absolute() || manifest.output_directory.exists() {
        return Err("Use a new absolute output directory".into());
    }
    if !matches!(manifest.contract_versions.as_slice(), [1, 2] | [2]) {
        return Err("Select the approved paired or v2-only contracts".into());
    }
    let store = RunStore::new(&manifest.session_directory);
    let snapshot = store.load(manifest.run_id)?;
    let model = &snapshot.inputs.model_set.reviewer;
    if model.profile.model != "gpt-5.6-luna" {
        return Err("This approved diagnostic is restricted to Luna".into());
    }
    let settings: serde_json::Value = serde_json::from_slice(&fs::read(&manifest.settings_file)?)
        .map_err(|_| "Cannot parse private Settings")?;
    let providers: Vec<ProviderConnection> =
        serde_json::from_value(settings["workflow"]["providers"].clone())
            .map_err(|_| "Cannot read private provider configuration")?;
    let provider = providers
        .into_iter()
        .find(|provider| provider.id == model.provider.id)
        .ok_or("The frozen provider is missing from Settings")?;
    if provider.snapshot()? != model.provider {
        return Err("Current provider does not match the frozen source run".into());
    }
    fs::create_dir_all(&manifest.output_directory)?;
    let jobs: Vec<_> = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.kind == JobKind::SelectEvidence)
        .collect();
    if jobs.len() != 3 {
        return Err("The approved diagnostic expects exactly three selector cases".into());
    }
    let mut tasks = tokio::task::JoinSet::new();
    for repeat in 1..=2 {
        for &contract_version in &manifest.contract_versions {
            for (index, job) in jobs.iter().enumerate() {
                let request = SelectorDiagnostic {
                    session_directory: manifest.session_directory.clone(),
                    run_id: manifest.run_id,
                    job_id: job.id.clone(),
                    provider: provider.clone(),
                    contract_version,
                };
                let output = manifest.output_directory.join(format!(
                    "case-{}-v{contract_version}-repeat-{repeat}.json",
                    index + 1
                ));
                tasks.spawn(async move {
                    let result = diagnose_selector(request).await;
                    (output, result)
                });
            }
        }
    }
    while let Some(result) = tasks.join_next().await {
        let (path, artifact) = result?;
        let artifact = artifact?;
        fs::write(&path, serde_json::to_vec_pretty(&artifact)?)?;
        println!(
            "{} strictSuccess={}",
            path.file_name().unwrap().to_string_lossy(),
            artifact["strictSuccess"]
        );
    }
    Ok(())
}
