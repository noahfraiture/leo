use serde_json::json;
use uuid::Uuid;

use crate::analysis::agent::{AnalysisResponse, ChecklistProgress, Observation};
use crate::analysis::video::AnalysisWarning;

use super::{ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint};

const TOTAL_BATCHES: usize = 2;

fn session_id() -> Uuid {
    Uuid::from_u128(1)
}

fn response(summary: &str) -> AnalysisResponse {
    AnalysisResponse {
        observations: vec![Observation {
            timestamp: "00:00:01".into(),
            description: "The student starts the exercise.".into(),
        }],
        sequence_summary: summary.into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Start the exercise".into(),
            status: "respected".into(),
            note: String::new(),
        }],
    }
}

fn checkpoint() -> AnalysisCheckpoint {
    AnalysisCheckpoint {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        session_id: session_id(),
        checklist: "Start the exercise".into(),
        plan_fingerprint: "0123456789abcdef".into(),
        total_batches: TOTAL_BATCHES,
        warnings: vec![AnalysisWarning::RecordingGap {
            camera_id: 2,
            start_offset_ms: 1_000,
            end_offset_ms: 2_000,
        }],
        responses: vec![response("Batch zero is complete.")],
    }
}

#[test]
fn checkpoint_v2_round_trips_checklist_fingerprint_warnings_and_responses() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("analysis.json");
    let expected = checkpoint();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&expected).expect("checkpoint should serialize"),
    )
    .expect("checkpoint should be written");

    let actual =
        AnalysisCheckpoint::read(&path, session_id()).expect("valid v2 checkpoint should load");

    assert_eq!(actual, expected);
}

#[test]
fn read_rejects_wrong_schema_session_empty_identity_and_excess_responses() {
    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let path = directory.path().join("analysis.json");
    let valid = serde_json::to_value(checkpoint()).expect("checkpoint should serialize");
    let invalid = [
        ("wrong schema", {
            let mut value = valid.clone();
            value["schema_version"] = json!(ANALYSIS_SCHEMA_VERSION - 1);
            value
        }),
        ("wrong session", {
            let mut value = valid.clone();
            value["session_id"] = json!(Uuid::from_u128(2));
            value
        }),
        ("empty checklist", {
            let mut value = valid.clone();
            value["checklist"] = json!("");
            value
        }),
        ("empty fingerprint", {
            let mut value = valid.clone();
            value["plan_fingerprint"] = json!("");
            value
        }),
        ("excess responses", {
            let mut value = valid.clone();
            value["total_batches"] = json!(0);
            value
        }),
    ];

    for (reason, value) in invalid {
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&value).expect("invalid checkpoint should serialize"),
        )
        .expect("invalid checkpoint should be written");

        AnalysisCheckpoint::read(&path, session_id()).expect_err(reason);
    }
}

#[cfg(unix)]
#[test]
fn read_rejects_a_symlinked_checkpoint() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory should be created");
    let target = directory.path().join("target.json");
    let link = directory.path().join("analysis.json");
    std::fs::write(
        &target,
        serde_json::to_vec_pretty(&checkpoint()).expect("checkpoint should serialize"),
    )
    .expect("checkpoint target should be written");
    symlink(&target, &link).expect("checkpoint symlink should be created");

    AnalysisCheckpoint::read(&link, session_id())
        .expect_err("a symlink must not be accepted as a checkpoint file");
}
