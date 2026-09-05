use std::{
    fs,
    path::{Path, PathBuf},
};

use backend::{
    analysis::{
        AnalysisCheckpoint, AnalysisResponse, AnalysisWarning, ChecklistProgress, Observation,
    },
    session::mark_recording_complete,
};
use serde_json::json;
use uuid::Uuid;

use crate::test_support::{
    RenderHarness, START_UTC_MS, assert_button_disabled, opening_tag_with_marker, ready_preview,
};

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

fn checkpoint(
    session_id: Uuid,
    total_batches: usize,
    warnings: Vec<AnalysisWarning>,
    responses: Vec<AnalysisResponse>,
) -> AnalysisCheckpoint {
    AnalysisCheckpoint {
        schema_version: 3,
        session_id,
        checklist: "Persisted correct-sequence checklist".into(),
        plan_fingerprint: "0123456789abcdef".into(),
        total_batches,
        analysis_profile: crate::test_analysis_profile(5, 0),
        resolved_batches: (0..total_batches).map(|i| i..i + 1).collect(),
        warnings,
        responses,
    }
}

fn response(description: &str, summary: &str) -> AnalysisResponse {
    AnalysisResponse {
        observations: vec![Observation {
            timestamp: "00:00:01.000".into(),
            description: description.into(),
        }],
        sequence_summary: summary.into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Complete the exercise".into(),
            status: "respected".into(),
            note: "All steps were visible".into(),
        }],
    }
}

fn prepare_session(
    harness: &mut RenderHarness,
    session_id: Uuid,
    saved: Option<&AnalysisCheckpoint>,
) -> PathBuf {
    let directory = write_completed_session(
        &harness.operator().session_root,
        &format!("session-{session_id}"),
        session_id,
        START_UTC_MS,
    );
    if let Some(saved) = saved {
        fs::write(
            directory.join("analysis.json"),
            serde_json::to_vec_pretty(saved).expect("checkpoint should serialize"),
        )
        .expect("checkpoint should be written");
    }
    harness
        .operator_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");
    harness.operator_mut().selected_session_id = Some(session_id);
    directory
}

fn assert_analysis_action(html: &str, label: &str, disabled: bool) {
    let button = opening_tag_with_marker(html, "button", r#"id="analysis-action""#);
    assert_eq!(button.contains("disabled"), disabled, "{button}");
    assert!(html.contains(&format!(">{label}</button>")), "{html}");
}

fn assert_row_status(html: &str, session_id: Uuid, status: &str) {
    let button = opening_tag_with_marker(
        html,
        "button",
        &format!("Session {session_id}, UTC milliseconds:"),
    );
    assert!(
        button.contains(&format!(", status: {status}\"")),
        "{button}"
    );
}

#[test]
fn empty_catalog_offers_refresh_and_selection_guidance() {
    let html = RenderHarness::new().render_at(ready_preview(), "/analyze");

    assert!(html.contains("Completed sessions"), "{html}");
    assert_button_disabled(&html, "Refresh sessions", false);
    assert!(html.contains("No completed sessions found."), "{html}");
    assert!(html.contains("Select a completed session"), "{html}");
    assert!(!html.contains("analysis-action"), "{html}");
}

#[test]
fn catalog_selects_the_newest_session_and_renders_its_recap() {
    let mut harness = RenderHarness::new();
    let root = harness.operator().session_root.clone();
    let older_id = Uuid::from_u128(101);
    let newer_id = Uuid::from_u128(102);
    write_completed_session(&root, "older", older_id, START_UTC_MS);
    let newer_directory = write_completed_session(&root, "newer", newer_id, START_UTC_MS + 10_000);
    harness
        .operator_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");

    let html = harness.render_at(ready_preview(), "/analyze");

    let newer_row = html.find(&format!("Session {newer_id}")).unwrap();
    let older_row = html.find(&format!("Session {older_id}")).unwrap();
    assert!(newer_row < older_row, "{html}");
    assert!(html.contains("Duration: 4000 ms"), "{html}");
    assert!(html.contains("Camera count: 1"), "{html}");
    assert!(
        html.contains(&newer_directory.display().to_string()),
        "{html}"
    );
    assert_analysis_action(&html, "Analyze", false);
}

#[test]
fn invalid_checkpoint_is_reported_without_exposing_its_contents() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(103);
    let directory = prepare_session(&mut harness, session_id, None);
    fs::write(directory.join("analysis.json"), b"not JSON")
        .expect("invalid checkpoint should be written");
    harness
        .operator_mut()
        .refresh_sessions()
        .expect("invalid checkpoint should remain a row result");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Invalid checkpoint");
    assert!(html.contains("Invalid analysis checkpoint"), "{html}");
    assert_analysis_action(&html, "Analyze", true);
    assert!(!html.contains("not JSON"), "{html}");
}

#[test]
fn failed_partial_analysis_keeps_progress_and_allows_resume() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(104);
    let saved = checkpoint(
        session_id,
        2,
        Vec::new(),
        vec![response("Saved observation", "Saved partial summary")],
    );
    prepare_session(&mut harness, session_id, Some(&saved));
    harness.operator_mut().analysis_error = Some((session_id, "Model request failed".into()));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Failed");
    assert!(html.contains("Model request failed"), "{html}");
    assert!(html.contains("Saved observation"), "{html}");
    assert!(html.contains("Analysis progress: 1 of 2 batches"), "{html}");
    assert!(
        html.contains("Persisted correct-sequence checklist"),
        "{html}"
    );
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn complete_analysis_renders_results_and_recording_warnings() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(105);
    let saved = checkpoint(
        session_id,
        1,
        vec![AnalysisWarning::RecordingGap {
            camera_id: 17,
            start_offset_ms: 500,
            end_offset_ms: 1_000,
        }],
        vec![response("Exercise completed", "Completed correctly")],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Complete with warning");
    assert!(
        html.contains("Recording gap: camera 17, 500 ms to 1000 ms"),
        "{html}"
    );
    assert!(html.contains("Exercise completed"), "{html}");
    assert!(html.contains("Completed correctly"), "{html}");
    assert!(html.contains("All steps were visible"), "{html}");
    assert_analysis_action(&html, "Resume", true);
}

#[derive(Clone, Copy, Debug)]
enum Blocker {
    MissingModel,
    Running,
    Recording,
}

#[test]
fn relevant_analysis_blockers_disable_the_action() {
    for (blocker, message) in [
        (
            Blocker::MissingModel,
            "Analysis requires an OpenAI API key and model in Settings.",
        ),
        (Blocker::Running, "Analysis is running."),
        (
            Blocker::Recording,
            "Analysis is unavailable while recording is active.",
        ),
    ] {
        let mut harness = RenderHarness::new();
        let session_id = Uuid::new_v4();
        prepare_session(&mut harness, session_id, None);
        match blocker {
            Blocker::MissingModel => {
                harness.operator_mut().model_config_error = Some(message.into());
            }
            Blocker::Running => harness.operator_mut().running_analysis_id = Some(session_id),
            Blocker::Recording => {
                harness.activate();
            }
        }

        let html = harness.render_at(ready_preview(), "/analyze");

        assert!(html.contains(message), "{blocker:?}: {html}");
        assert_analysis_action(&html, "Analyze", true);
    }
}
