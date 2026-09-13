use crate::{
    analysis::{AnalysisTab, test_snapshot},
    test_support::{
        RenderHarness, START_UTC_MS, assert_button_disabled, opening_tag_with_marker, ready_preview,
    },
};
use backend::{
    analysis::{AnalysisWarning, HumanCorrection, JobState, Judgment, RunSnapshot, RunStage},
    session::mark_recording_complete,
};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

fn write_completed_session(
    root: &Path,
    name: &str,
    session_id: Uuid,
    start_utc_ms: i64,
) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("recordings"))
        .expect("render session directories should be created");
    let events = [
        json!({
            "schema_version": 2,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started", "monitoring_profiles": crate::test_monitoring_profiles(),
                "cameras": [{
                    "camera_id": 17,
                    "name": "Salon 1",
                    "enabled": true,
                    "initial_monitoring_profile_id": 1
                }]
            }
        }),
        json!({
            "schema_version": 2,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + 4_000,
            "session_offset_ms": 4_000,
            "action": { "type": "session_ended" }
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events)
        .expect("render session events should be written");
    mark_recording_complete(&directory).expect("render session should be marked complete");
    directory
}

fn prepared(session_id: Uuid, snapshot: Option<RunSnapshot>) -> RenderHarness {
    let mut harness = RenderHarness::new();
    let directory = write_completed_session(
        &harness.operator().session_root,
        "selected",
        session_id,
        START_UTC_MS,
    );
    harness.operator_mut().refresh_sessions().unwrap();
    harness.operator_mut().selected_session_id = Some(session_id);
    harness.analysis.tab = AnalysisTab::Evaluate;
    let session = harness.analysis.sessions.entry(session_id).or_default();
    session.directory = directory;
    session.loaded = true;
    if let Some(snapshot) = snapshot {
        session.selected_run_id = Some(snapshot.inputs.id);
        harness.analysis.apply_snapshot(snapshot);
    }
    harness
}

#[test]
fn empty_catalog_keeps_exercise_preparation_available() {
    let html = RenderHarness::new().render_at(ready_preview(), "/analyze");
    assert!(html.contains("Completed sessions"));
    assert_button_disabled(&html, "Refresh sessions", false);
    assert!(html.contains("No completed sessions found."));
    assert!(html.contains("New configuration"));
    assert!(html.contains("1. Exercise setup"));
    assert!(!html.contains("analysis-start"));
}

#[test]
fn catalog_selects_newest_recording_and_explains_required_configuration() {
    let mut harness = RenderHarness::new();
    let root = harness.operator().session_root.clone();
    let older_id = Uuid::from_u128(101);
    let newer_id = Uuid::from_u128(102);
    write_completed_session(&root, "older", older_id, START_UTC_MS);
    write_completed_session(&root, "newer", newer_id, START_UTC_MS + 10_000);
    harness.operator_mut().refresh_sessions().unwrap();
    harness.analysis.tab = AnalysisTab::Evaluate;
    let html = harness.render_at(ready_preview(), "/analyze");
    assert!(
        html.find(&format!("Session {newer_id}")).unwrap()
            < html.find(&format!("Session {older_id}")).unwrap()
    );
    assert!(html.contains("Duration: 0:04. Cameras: 1."));
    assert!(html.contains("Save the current exercise configuration"));
    assert!(
        opening_tag_with_marker(&html, "button", r#"id="analysis-start""#).contains("disabled")
    );
}

#[test]
fn history_failure_is_reported_without_becoming_a_candidate_judgment() {
    let session_id = Uuid::new_v4();
    let mut harness = prepared(session_id, None);
    harness
        .analysis
        .sessions
        .get_mut(&session_id)
        .unwrap()
        .error = Some("Analysis data could not be read.".into());
    let html = harness.render_at(ready_preview(), "/analyze");
    assert!(html.contains("Analysis data could not be read."));
    assert!(html.contains("Needs attention"));
    assert!(!html.contains("Model: Not met"));
}

#[test]
fn paused_processing_retains_findings_and_offers_resume() {
    let session_id = Uuid::new_v4();
    let mut run = test_snapshot(session_id, Uuid::new_v4(), RunStage::Paused);
    run.checkpoint.error = Some("Model request failed".into());
    let mut failed = run.checkpoint.jobs[0].clone();
    failed.id = "failed-job".into();
    failed.state = JobState::Failed {
        message: "Temporary provider failure".into(),
    };
    run.checkpoint.jobs.push(failed);
    let html = prepared(session_id, Some(run)).render_at(ready_preview(), "/analyze");
    assert!(html.contains("Model request failed"));
    assert!(html.contains("The item is partly obscured."));
    assert!(html.contains("Completed model requests: 1 of 2"));
    assert!(html.contains("Not assessed yet"));
    assert_button_disabled(&html, "Resume saved run", false);
}

#[test]
fn ready_run_keeps_uncertainty_and_recording_gaps_visible() {
    let session_id = Uuid::new_v4();
    let mut run = test_snapshot(session_id, Uuid::new_v4(), RunStage::Ready);
    run.checkpoint
        .evidence
        .warnings
        .push(AnalysisWarning::RecordingGap {
            camera_id: 17,
            start_offset_ms: 500,
            end_offset_ms: 1000,
        });
    let html = prepared(session_id, Some(run)).render_at(ready_preview(), "/analyze");
    assert!(html.contains("Ready for review"));
    assert!(html.contains("Model: Cannot verify"));
    assert!(html.contains("The closing action is obscured."));
    assert!(html.contains("Recording gap on camera 17"));
    assert!(html.contains("Reference examples used for this run"));
    assert!(html.contains("Reason for your correction"));
    assert!(!html.contains("analysis-resume"));
}

#[test]
fn recording_blocks_resuming_a_saved_run() {
    let session_id = Uuid::new_v4();
    let run = test_snapshot(session_id, Uuid::new_v4(), RunStage::Paused);
    let mut harness = prepared(session_id, Some(run));
    harness.activate();
    let html = harness.render_at(ready_preview(), "/analyze");
    assert!(html.contains("Analysis is unavailable while recording is active."));
    assert_button_disabled(&html, "Resume saved run", true);
}

#[test]
fn saved_correction_keeps_its_own_candidate_links_after_reload() {
    let session_id = Uuid::new_v4();
    let mut run = test_snapshot(session_id, Uuid::new_v4(), RunStage::Ready);
    run.corrections.push(HumanCorrection {
        id: Uuid::new_v4(),
        criterion_id: run.inputs.exercise.checklist.criteria[0].id,
        judgment: Judgment::Met,
        reason: "The operator confirmed the step in this footage.".into(),
        evidence_ids: vec!["candidate-1".into()],
        uses_outside_information: false,
        created_utc_ms: START_UTC_MS as u64,
        supersedes: None,
    });
    let html = prepared(session_id, Some(run)).render_at(ready_preview(), "/analyze");
    let saved = html.split("Saved correction history").nth(1).unwrap();
    assert!(saved.contains("data-evidence-id=\"candidate-1\""));
    assert!(saved.contains("The operator confirmed the step in this footage."));
    assert!(html.contains("Model: Cannot verify"));
    assert!(html.contains("Operator: Met"));
}
