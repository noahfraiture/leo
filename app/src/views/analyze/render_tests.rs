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
    RenderHarness, START_UTC_MS, assert_button_disabled, opening_tag_before,
    opening_tag_with_marker, ready_preview,
};

fn write_completed_session(
    root: &Path,
    name: &str,
    session_id: Uuid,
    start_utc_ms: i64,
    end_offset_ms: u64,
) -> PathBuf {
    let directory = root.join(name);
    fs::create_dir_all(directory.join("recordings"))
        .expect("render session directories should be created");
    let events = [
        json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started",
                "cameras": [
                    {
                        "camera_id": 17,
                        "name": "Salon 1",
                        "enabled": true,
                        "sample_every_ms": 1_000
                    },
                    {
                        "camera_id": 42,
                        "name": "Salon 2",
                        "enabled": false,
                        "sample_every_ms": 2_000
                    }
                ]
            }
        }),
        json!({
            "schema_version": 1,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + i64::try_from(end_offset_ms).unwrap(),
            "session_offset_ms": end_offset_ms,
            "action": { "type": "session_ended" }
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("render event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events)
        .expect("render session events should be written");
    mark_recording_complete(&directory).expect("render session should be marked complete");
    directory
}

fn response(
    timestamp: &str,
    description: &str,
    summary: &str,
    status: &str,
    note: &str,
) -> AnalysisResponse {
    AnalysisResponse {
        observations: vec![Observation {
            timestamp: timestamp.into(),
            description: description.into(),
        }],
        sequence_summary: summary.into(),
        checklist_progress: vec![ChecklistProgress {
            item: "Complete the exercise".into(),
            status: status.into(),
            note: note.into(),
        }],
    }
}

fn checkpoint(
    session_id: Uuid,
    total_batches: usize,
    warnings: Vec<AnalysisWarning>,
    responses: Vec<AnalysisResponse>,
) -> AnalysisCheckpoint {
    AnalysisCheckpoint {
        schema_version: 2,
        session_id,
        checklist: "Persisted correct-sequence checklist".into(),
        plan_fingerprint: "0123456789abcdef".into(),
        total_batches,
        warnings,
        responses,
    }
}

fn write_checkpoint(directory: &Path, checkpoint: &AnalysisCheckpoint) {
    fs::write(
        directory.join("analysis.json"),
        serde_json::to_vec_pretty(checkpoint).expect("render checkpoint should serialize"),
    )
    .expect("render checkpoint should be written");
}

fn prepare_session(
    harness: &mut RenderHarness,
    session_id: Uuid,
    checkpoint: Option<&AnalysisCheckpoint>,
) -> PathBuf {
    let directory = write_completed_session(
        &harness.workflow().session_root,
        &format!("session-{session_id}"),
        session_id,
        START_UTC_MS,
        4_000,
    );
    if let Some(checkpoint) = checkpoint {
        write_checkpoint(&directory, checkpoint);
    }
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");
    harness.workflow_mut().selected_session_id = Some(session_id);
    directory
}

fn assert_analysis_action(html: &str, label: &str, disabled: bool) {
    let button = opening_tag_with_marker(html, "button", r#"id="analysis-action""#);
    assert_eq!(
        button.contains("disabled"),
        disabled,
        "unexpected analysis action state: {button}"
    );
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
        "missing {status:?} row for {session_id}: {button}"
    );
}

fn assert_progress(html: &str, completed: usize, total: usize) {
    let marker = format!(r#"aria-label="Analysis progress: {completed} of {total} batches""#);
    let progress = opening_tag_with_marker(html, "progress", &marker);
    assert!(
        progress.contains(&format!(r#"value="{completed}""#)),
        "{progress}"
    );
    assert!(
        progress.contains(&format!(r#"max="{total}""#)),
        "{progress}"
    );
}

#[test]
fn empty_catalog_renders_refresh_and_selection_guidance() {
    let html = RenderHarness::new().render_at(ready_preview(), "/analyze");

    assert!(html.contains("Completed sessions"), "{html}");
    assert_button_disabled(&html, "Refresh sessions", false);
    assert!(html.contains("No completed sessions found."), "{html}");
    assert!(
        html.contains("Select a completed session to analyze."),
        "{html}"
    );
    assert!(!html.contains("analysis-action"), "{html}");
    assert_eq!(html.matches("<video").count(), 0, "{html}");
}

#[test]
fn catalog_lists_newest_first_and_selected_session_renders_a_recap() {
    let mut harness = RenderHarness::new();
    let root = harness.workflow().session_root.clone();
    let older_id = Uuid::from_u128(101);
    let newer_id = Uuid::from_u128(102);
    write_completed_session(&root, "older", older_id, START_UTC_MS, 2_000);
    let newer_directory =
        write_completed_session(&root, "newer", newer_id, START_UTC_MS + 10_000, 4_000);
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("render sessions should refresh");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, older_id, "Not started");
    assert_row_status(&html, newer_id, "Not started");
    let newer_row = html
        .find(&format!(
            "Session {newer_id}, UTC milliseconds: {}, status: Not started",
            START_UTC_MS + 10_000
        ))
        .expect("newer session row should render");
    let older_row = html
        .find(&format!(
            "Session {older_id}, UTC milliseconds: {START_UTC_MS}, status: Not started"
        ))
        .expect("older session row should render");
    assert!(
        newer_row < older_row,
        "sessions must render newest first: {html}"
    );
    assert!(html.contains(&newer_id.to_string()), "{html}");
    assert!(html.contains("Duration: 4000 ms"), "{html}");
    assert!(html.contains("Camera count: 2"), "{html}");
    assert!(
        html.contains(&newer_directory.display().to_string()),
        "{html}"
    );
    for (name, path) in [
        ("events.jsonl", newer_directory.join("events.jsonl")),
        ("recordings/", newer_directory.join("recordings")),
        (
            "recording-complete",
            newer_directory.join("recording-complete"),
        ),
        ("analysis.json", newer_directory.join("analysis.json")),
    ] {
        assert!(html.contains(name), "missing {name:?} in {html}");
        assert!(html.contains(&path.display().to_string()), "{html}");
    }
    let textarea = opening_tag_with_marker(&html, "textarea", r#"id="analysis-checklist""#);
    assert!(!textarea.contains("readonly"), "{textarea}");
    assert_analysis_action(&html, "Analyze", false);
    assert!(!html.contains("session_started"), "{html}");
}

#[test]
fn invalid_checkpoint_is_sanitized_and_cannot_be_replaced() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(103);
    let directory = prepare_session(&mut harness, session_id, None);
    fs::write(directory.join("analysis.json"), b"not JSON")
        .expect("invalid render checkpoint should be written");
    harness
        .workflow_mut()
        .refresh_sessions()
        .expect("invalid checkpoint should remain a row result");

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Invalid checkpoint");
    assert!(html.contains("Invalid analysis checkpoint"), "{html}");
    assert!(
        html.contains("analysis checkpoint is not valid JSON"),
        "{html}"
    );
    assert_analysis_action(&html, "Analyze", true);
    assert!(!html.contains("not JSON"), "{html}");
}

#[test]
fn missing_model_configuration_disables_new_analysis() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(104);
    prepare_session(&mut harness, session_id, None);
    harness.workflow_mut().model_config_error =
        Some("Analysis requires an OpenAI API key and model in Settings.".into());

    let html = harness.render_at(ready_preview(), "/analyze");

    assert!(
        html.contains("Analysis requires an OpenAI API key and model in Settings."),
        "{html}"
    );
    assert_analysis_action(&html, "Analyze", true);
}

#[test]
fn zero_response_checkpoint_renders_locked_resumable_progress() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(105);
    let saved = checkpoint(session_id, 2, Vec::new(), Vec::new());
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "In progress");
    let textarea = opening_tag_with_marker(&html, "textarea", r#"id="analysis-checklist""#);
    assert!(textarea.contains("readonly"), "{textarea}");
    assert!(
        html.contains("Persisted correct-sequence checklist"),
        "{html}"
    );
    assert_progress(&html, 0, 2);
    assert!(html.contains("No completed batches yet."), "{html}");
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn partial_checkpoint_renders_observations_in_order_and_latest_cumulative_state() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(106);
    let saved = checkpoint(
        session_id,
        3,
        Vec::new(),
        vec![
            response(
                "00:00:01.000",
                "First visible movement",
                "Old sequence summary",
                "old status",
                "Old checklist note",
            ),
            response(
                "00:00:02.000",
                "Second visible movement",
                "Latest sequence summary",
                "respected",
                "Latest checklist note",
            ),
        ],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_progress(&html, 2, 3);
    let first = html.find("First visible movement").unwrap();
    let second = html.find("Second visible movement").unwrap();
    assert!(
        first < second,
        "observations must retain response order: {html}"
    );
    for expected in [
        "00:00:01.000",
        "00:00:02.000",
        "Latest sequence summary",
        "respected",
        "Latest checklist note",
    ] {
        assert!(html.contains(expected), "missing {expected:?} in {html}");
    }
    for stale in ["Old sequence summary", "old status", "Old checklist note"] {
        assert!(!html.contains(stale), "found stale {stale:?} in {html}");
    }
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn complete_checkpoint_renders_final_results_and_disables_resume() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(107);
    let saved = checkpoint(
        session_id,
        2,
        Vec::new(),
        vec![
            response(
                "00:00:01.000",
                "Exercise started",
                "Exercise in progress",
                "not yet",
                "Waiting for completion",
            ),
            response(
                "00:00:03.000",
                "Exercise completed",
                "Exercise completed correctly",
                "respected",
                "All steps were visible",
            ),
        ],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Complete");
    assert_progress(&html, 2, 2);
    assert!(html.contains("Exercise started"), "{html}");
    assert!(html.contains("Exercise completed correctly"), "{html}");
    assert!(html.contains("All steps were visible"), "{html}");
    assert!(!html.contains("Exercise in progress"), "{html}");
    assert!(!html.contains("Waiting for completion"), "{html}");
    assert_analysis_action(&html, "Resume", true);
}

#[test]
fn recording_gaps_render_before_complete_model_results() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(108);
    let saved = checkpoint(
        session_id,
        1,
        vec![
            AnalysisWarning::RecordingGap {
                camera_id: 17,
                start_offset_ms: 500,
                end_offset_ms: 1_000,
            },
            AnalysisWarning::RecordingGap {
                camera_id: 42,
                start_offset_ms: 2_000,
                end_offset_ms: 2_500,
            },
        ],
        vec![response(
            "00:00:03.000",
            "Visible result after gaps",
            "Latest warning summary",
            "uncertain",
            "Recording gaps limit confidence",
        )],
    );
    prepare_session(&mut harness, session_id, Some(&saved));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Complete with warning");
    let first_gap = "Recording gap: camera 17, 500 ms to 1000 ms";
    let second_gap = "Recording gap: camera 42, 2000 ms to 2500 ms";
    let result_index = html.find("Visible result after gaps").unwrap();
    assert!(html.find(first_gap).unwrap() < result_index, "{html}");
    assert!(html.find(second_gap).unwrap() < result_index, "{html}");
    assert_analysis_action(&html, "Resume", true);
}

#[test]
fn running_analysis_disables_only_the_analysis_action() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(109);
    prepare_session(&mut harness, session_id, None);
    harness.workflow_mut().running_analysis_id = Some(session_id);
    harness.workflow_mut().analysis_error = Some((session_id, "stale failure".into()));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Running");
    assert_analysis_action(&html, "Analyze", true);
    assert!(html.contains("Analysis is running."), "{html}");
    assert!(!html.contains("stale failure"), "{html}");
    let monitor_link = opening_tag_before(&html, "a", "Monitor");
    let analyze_link = opening_tag_before(&html, "a", "Analyze");
    assert!(!monitor_link.contains("disabled"), "{monitor_link}");
    assert!(!analyze_link.contains("disabled"), "{analyze_link}");
    assert!(analyze_link.contains(r#"aria-current="page""#), "{html}");
}

#[test]
fn failed_analysis_keeps_saved_progress_and_allows_resume() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(110);
    let saved = checkpoint(
        session_id,
        2,
        Vec::new(),
        vec![response(
            "00:00:01.000",
            "Saved observation",
            "Saved partial summary",
            "not yet",
            "One batch remains",
        )],
    );
    prepare_session(&mut harness, session_id, Some(&saved));
    harness.workflow_mut().analysis_error =
        Some((session_id, "The model request failed safely.".into()));

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_row_status(&html, session_id, "Failed");
    assert!(html.contains("The model request failed safely."), "{html}");
    assert_progress(&html, 1, 2);
    assert!(html.contains("Saved observation"), "{html}");
    assert_analysis_action(&html, "Resume", false);
}

#[test]
fn active_recording_disables_analysis_for_a_completed_session() {
    let mut harness = RenderHarness::new();
    let session_id = Uuid::from_u128(111);
    prepare_session(&mut harness, session_id, None);
    harness.activate();

    let html = harness.render_at(ready_preview(), "/analyze");

    assert_analysis_action(&html, "Analyze", true);
    assert!(
        html.contains("Analysis is unavailable while recording is active."),
        "{html}"
    );
}
