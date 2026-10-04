use super::{AnalysisPageState, test_snapshot};
use backend::analysis::{HumanCorrection, Judgment, RunStage};
use uuid::Uuid;

#[test]
fn delayed_history_refresh_cannot_replace_newer_live_progress() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let older = test_snapshot(session_id, run_id, RunStage::Scanning);
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(older.clone());
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    page.apply_snapshot(test_snapshot(session_id, run_id, RunStage::Ready));
    page.apply_history_snapshot(older, Some(generation));
    assert_eq!(
        page.sessions[&session_id].runs[&run_id]
            .snapshot
            .checkpoint
            .stage,
        RunStage::Ready
    );
}

#[test]
fn progress_for_another_run_preserves_selection_and_correction_draft() {
    let session_id = Uuid::new_v4();
    let selected = Uuid::new_v4();
    let other = Uuid::new_v4();
    let first = test_snapshot(session_id, selected, RunStage::Scanning);
    let criterion = first.inputs.exercise.checklist.criteria[0].id;
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(first);
    page.sessions.get_mut(&session_id).unwrap().selected_run_id = Some(selected);
    page.edit_correction(session_id, selected, criterion, |draft| {
        draft.reason = "My unsaved note".into()
    });
    page.apply_snapshot(test_snapshot(session_id, other, RunStage::Ready));
    assert_eq!(page.sessions[&session_id].selected_run_id, Some(selected));
    assert_eq!(
        page.sessions[&session_id].runs[&selected].corrections[&criterion].reason,
        "My unsaved note"
    );
}

#[test]
fn execution_snapshot_cannot_remove_a_concurrently_saved_human_correction() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let earlier = test_snapshot(session_id, run_id, RunStage::Scanning);
    let mut corrected = earlier.clone();
    let correction = HumanCorrection {
        id: Uuid::new_v4(),
        criterion_id: earlier.inputs.exercise.checklist.criteria[0].id,
        judgment: Judgment::Met,
        reason: "Verified in footage.".into(),
        evidence_ids: vec!["candidate-1".into()],
        uses_outside_information: false,
        created_utc_ms: 1,
        supersedes: None,
    };
    corrected.corrections.push(correction.clone());
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(corrected);
    page.apply_snapshot(earlier);
    assert_eq!(
        page.sessions[&session_id].runs[&run_id]
            .snapshot
            .corrections,
        vec![correction]
    );
}

#[test]
fn correction_save_updates_human_history_without_replacing_finished_processing() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let run = test_snapshot(session_id, run_id, RunStage::Ready);
    let criterion_id = run.inputs.exercise.checklist.criteria[0].id;
    let model_assessments = run.checkpoint.assessments.clone();
    let correction = HumanCorrection {
        id: Uuid::new_v4(),
        criterion_id,
        judgment: Judgment::Met,
        reason: "Observed the completion in the footage.".into(),
        evidence_ids: vec!["candidate-1".into()],
        uses_outside_information: false,
        created_utc_ms: 1,
        supersedes: None,
    };
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(run);
    page.apply_corrections(session_id, run_id, vec![correction.clone()]);
    let updated = &page.sessions[&session_id].runs[&run_id].snapshot;
    assert_eq!(updated.checkpoint.stage, RunStage::Ready);
    assert_eq!(updated.checkpoint.assessments, model_assessments);
    assert_eq!(updated.corrections, vec![correction]);
}
