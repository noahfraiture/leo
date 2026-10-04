//! Read-only history for retired run formats. The old execution engine is deliberately absent.
use super::{RunStore, store::read_json};
use crate::exercise::Checklist;
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

/// An old or unavailable run stays discoverable without blocking valid newer history.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArchivedRun {
    pub id: Uuid,
    pub directory: PathBuf,
    pub name: String,
    pub criteria: Vec<(String, String, String)>,
    pub message: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyInputs {
    schema_version: u8,
    exercise: LegacyExercise,
}
#[derive(Deserialize)]
struct LegacyExercise {
    name: String,
    checklist: Checklist,
}
#[derive(Deserialize)]
struct LegacyCheckpoint {
    assessments: Vec<LegacyAssessment>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyAssessment {
    criterion_id: Uuid,
    judgment: String,
    reason: String,
}

pub fn read(store: &RunStore, id: Uuid, error: String) -> ArchivedRun {
    let mut entry = ArchivedRun {
        id,
        directory: store
            .session_directory
            .join("analyses")
            .join(id.to_string()),
        name: "Unavailable run".into(),
        message: error,
        ..Default::default()
    };
    let Ok(directory) = store.directory(id) else {
        return entry;
    };
    let Ok(inputs) = read_json::<LegacyInputs>(&directory.join("inputs.json")) else {
        return entry;
    };
    entry.name = inputs.exercise.name;
    if inputs.schema_version != 1 {
        return entry;
    }
    let Ok(checkpoint) = read_json::<LegacyCheckpoint>(&directory.join("checkpoint.json")) else {
        return entry;
    };
    entry.criteria = inputs
        .exercise
        .checklist
        .criteria
        .into_iter()
        .map(|criterion| {
            let finding = checkpoint
                .assessments
                .iter()
                .find(|assessment| assessment.criterion_id == criterion.id);
            (
                criterion.text,
                finding
                    .map(|item| item.judgment.clone())
                    .unwrap_or_else(|| "Not assessed".into()),
                finding.map(|item| item.reason.clone()).unwrap_or_default(),
            )
        })
        .collect();
    entry.message = "Previous strategy, read-only. Original evidence, responses and correction history are preserved in the run folder. Start a new analysis to use the current flow.".into();
    entry
}
