use super::state::{ActiveRun, CorrectionDraft, RunEntry};
use super::{AnalysisPageState, test_snapshot};
use backend::analysis::{
    ArchivedRun, HumanCorrection, Judgment, RunHistory, RunStage, StudentAssessment, StudentOutcome,
};
use uuid::Uuid;

mod review_admission;

#[test]
fn loaded_assessment_editor_uses_only_the_current_saved_decision() {
    for (stage, changed_result) in [
        (RunStage::Ready, false),
        (RunStage::Ready, true),
        (RunStage::Paused, false),
    ] {
        let mut snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Ready);
        snapshot
            .operator_review
            .assessments
            .push(StudentAssessment {
                outcome: StudentOutcome::Pass,
                note: "Completed the exercise independently.".into(),
                basis: snapshot.assessment_basis().unwrap(),
                created_utc_ms: 1000,
            });
        snapshot.checkpoint.stage = stage;
        if changed_result {
            snapshot.checkpoint.assessments[0].reason = "A revised finding".into();
        }

        let entry = RunEntry::new(snapshot);
        let current = stage == RunStage::Ready && !changed_result;
        assert_eq!(
            entry.assessment_draft.outcome,
            if current {
                StudentOutcome::Pass
            } else {
                StudentOutcome::NeedsEvidence
            }
        );
        assert_eq!(
            entry.assessment_draft.note,
            if current {
                "Completed the exercise independently."
            } else {
                ""
            }
        );
        assert_eq!(entry.assessment_draft_is_saved(), current);
        assert_eq!(entry.snapshot.operator_review.assessments.len(), 1);
    }
}

#[test]
fn refreshing_a_run_preserves_the_newer_unsaved_assessment() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let snapshot = test_snapshot(session_id, run_id, RunStage::Ready);
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(snapshot.clone());
    let draft = &mut page
        .sessions
        .get_mut(&session_id)
        .unwrap()
        .runs
        .get_mut(&run_id)
        .unwrap()
        .assessment_draft;
    draft.outcome = StudentOutcome::NeedsPractice;
    draft.note = "My newer unsaved assessment".into();

    page.apply_snapshot(snapshot.clone());
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    page.apply_history_snapshot(snapshot, Some(generation));

    let draft = &page.sessions[&session_id].runs[&run_id].assessment_draft;
    assert_eq!(draft.outcome, StudentOutcome::NeedsPractice);
    assert_eq!(draft.note, "My newer unsaved assessment");
}

#[test]
fn unchanged_assessment_matches_the_saved_outcome_and_exact_note() {
    let snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Ready);
    let mut entry = RunEntry::new(snapshot);
    entry.assessment_draft.note = "Operator review".into();
    assert!(!entry.assessment_draft_is_saved());
    entry
        .snapshot
        .operator_review
        .assessments
        .push(StudentAssessment {
            outcome: entry.assessment_draft.outcome,
            note: entry.assessment_draft.note.clone(),
            basis: entry.snapshot.assessment_basis().unwrap(),
            created_utc_ms: 1000,
        });
    assert!(entry.assessment_draft_is_saved());

    entry.assessment_draft.note.push(' ');
    assert!(!entry.assessment_draft_is_saved());
    entry.assessment_draft.note.pop();
    entry.assessment_draft.outcome = StudentOutcome::Pass;
    assert!(!entry.assessment_draft_is_saved());
    entry.assessment_draft.outcome = StudentOutcome::NeedsEvidence;
    assert!(entry.assessment_draft_is_saved());
}

#[test]
fn changed_results_allow_a_new_assessment_with_the_same_content() {
    let snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Ready);
    let mut entry = RunEntry::new(snapshot);
    entry.assessment_draft.note = "Operator review".into();
    entry
        .snapshot
        .operator_review
        .assessments
        .push(StudentAssessment {
            outcome: entry.assessment_draft.outcome,
            note: entry.assessment_draft.note.clone(),
            basis: entry.snapshot.assessment_basis().unwrap(),
            created_utc_ms: 1000,
        });
    entry.snapshot.checkpoint.assessments[0].reason = "A revised finding".into();

    assert!(!entry.assessment_draft_is_saved());
    assert_eq!(entry.snapshot.operator_review.assessments.len(), 1);
}

#[test]
fn saved_correction_reason_rejects_old_input_and_accepts_new_edits() {
    let mut draft = CorrectionDraft::default();
    let previous_edit_id = draft.reason_edit_id;
    draft.set_reason(previous_edit_id, "Saved operator reason".into());
    draft.saving = true;
    let submitted = draft.clone();
    draft.finish_save(&submitted);
    assert!(draft.reason.is_empty());
    assert!(!draft.saving);

    let current_edit_id = draft.reason_edit_id;
    assert_ne!(current_edit_id, previous_edit_id);
    draft.set_reason(previous_edit_id, "Late input from the saved field".into());
    assert!(draft.reason.is_empty());
    draft.set_reason(current_edit_id, "Next operator reason".into());
    assert_eq!(draft.reason, "Next operator reason");
}

#[test]
fn correction_save_completion_preserves_newer_reason_and_its_field_identity() {
    let mut draft = CorrectionDraft::default();
    let edit_id = draft.reason_edit_id;
    draft.set_reason(edit_id, "Submitted reason".into());
    draft.saving = true;
    let submitted = draft.clone();
    draft.set_reason(edit_id, "Newer operator reason".into());
    draft.finish_save(&submitted);

    assert_eq!(draft.reason, "Newer operator reason");
    assert_eq!(draft.reason_edit_id, edit_id);
    assert!(!draft.saving);
}

#[test]
fn another_recordings_history_cannot_import_corrections_into_its_cached_review() {
    let selected_session = Uuid::from_u128(501);
    let other_session = Uuid::from_u128(502);
    let selected_run = Uuid::from_u128(503);
    let other_run = Uuid::from_u128(504);
    let original = test_snapshot(other_session, other_run, RunStage::Ready);
    let mut foreign = original.clone();
    foreign.corrections.push(HumanCorrection {
        id: Uuid::new_v4(),
        criterion_id: foreign.inputs.exercise.checklist.criteria[0].id,
        judgment: Judgment::Met,
        reason: "Saved only in the misplaced copy".into(),
        evidence_ids: Vec::new(),
        uses_outside_information: true,
        created_utc_ms: 1000,
        supersedes: None,
    });
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(original.clone());

    page.apply_history(
        selected_session,
        Ok(RunHistory {
            runs: vec![
                test_snapshot(selected_session, selected_run, RunStage::Ready),
                foreign,
            ],
            archived: Vec::new(),
        }),
        &Default::default(),
    );

    assert_eq!(page.sessions[&selected_session].runs.len(), 1);
    assert!(
        page.sessions[&selected_session]
            .runs
            .contains_key(&selected_run)
    );
    assert_eq!(
        page.sessions[&other_session].runs[&other_run]
            .snapshot
            .corrections,
        original.corrections
    );
}

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
fn surviving_recording_copy_reloads_history_and_preserves_unsaved_review() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let snapshot = test_snapshot(session_id, run_id, RunStage::Ready);
    let criterion_id = snapshot.inputs.exercise.checklist.criteria[0].id;
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(snapshot.clone());
    let session = page.sessions.get_mut(&session_id).unwrap();
    session.directory = "/original".into();
    session.loaded = true;
    session.selected_run_id = Some(run_id);
    page.edit_correction(session_id, run_id, criterion_id, |draft| {
        draft.reason = "Unsaved review".into()
    });

    let (id, generations) = page
        .begin_history_load(session_id, "/backup".into(), false)
        .unwrap();

    assert_eq!(
        page.sessions[&session_id].directory,
        std::path::PathBuf::from("/backup")
    );
    assert!(!page.sessions[&session_id].runs[&run_id].available);
    page.finish_history_load(
        session_id,
        id,
        Ok(RunHistory {
            runs: vec![snapshot],
            archived: Vec::new(),
        }),
        &generations,
    );
    let session = &page.sessions[&session_id];
    assert!(session.loaded);
    assert!(session.runs[&run_id].available);
    assert_eq!(session.selected_run_id, Some(run_id));
    assert_eq!(
        session.runs[&run_id].corrections[&criterion_id].reason,
        "Unsaved review"
    );
}

#[test]
fn surviving_copy_does_not_inherit_reviews_saved_only_in_the_original_folder() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let backup = test_snapshot(session_id, run_id, RunStage::Ready);
    let mut original = backup.clone();
    original.corrections.push(HumanCorrection {
        id: Uuid::new_v4(),
        criterion_id: original.inputs.exercise.checklist.criteria[0].id,
        judgment: Judgment::Met,
        reason: "Saved after the backup was copied".into(),
        evidence_ids: Vec::new(),
        uses_outside_information: true,
        created_utc_ms: 1000,
        supersedes: None,
    });
    original
        .operator_review
        .marks
        .push(backend::analysis::ReviewMark {
            criterion_id: original.inputs.exercise.checklist.criteria[0].id,
            basis: original
                .criterion_basis(original.inputs.exercise.checklist.criteria[0].id)
                .unwrap(),
            created_utc_ms: 1000,
        });
    original
        .operator_review
        .assessments
        .push(backend::analysis::StudentAssessment {
            outcome: backend::analysis::StudentOutcome::Pass,
            note: "Assessment saved after the backup was copied".into(),
            basis: original.assessment_basis().unwrap(),
            created_utc_ms: 1000,
        });
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(original);
    let session = page.sessions.get_mut(&session_id).unwrap();
    session.directory = "/original".into();
    session.loaded = true;
    let (id, generations) = page
        .begin_history_load(session_id, "/backup".into(), false)
        .unwrap();

    page.finish_history_load(
        session_id,
        id,
        Ok(RunHistory {
            runs: vec![backup.clone()],
            archived: Vec::new(),
        }),
        &generations,
    );

    assert_eq!(
        page.sessions[&session_id].runs[&run_id]
            .snapshot
            .corrections,
        backup.corrections
    );
    assert_eq!(
        page.sessions[&session_id].runs[&run_id]
            .snapshot
            .operator_review,
        backup.operator_review
    );
}

#[test]
fn an_old_folder_read_cannot_publish_after_history_moves_to_another_folder() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let mut page = AnalysisPageState::default();
    let (old_id, old_generations) = page
        .begin_history_load(session_id, "/original".into(), false)
        .unwrap();
    let (new_id, new_generations) = page
        .begin_history_load(session_id, "/backup".into(), false)
        .unwrap();

    page.finish_history_load(
        session_id,
        old_id,
        Ok(RunHistory {
            runs: vec![test_snapshot(session_id, run_id, RunStage::Ready)],
            archived: Vec::new(),
        }),
        &old_generations,
    );

    assert!(page.sessions[&session_id].runs.is_empty());
    assert!(page.sessions[&session_id].loading);
    page.finish_history_load(
        session_id,
        new_id,
        Ok(RunHistory::default()),
        &new_generations,
    );
    assert!(!page.sessions[&session_id].loading);
    assert!(page.sessions[&session_id].loaded);
}

#[test]
fn history_keeps_its_folder_while_operator_review_is_saving() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(test_snapshot(session_id, run_id, RunStage::Ready));
    let session = page.sessions.get_mut(&session_id).unwrap();
    session.directory = "/original".into();
    session.loaded = true;
    session
        .runs
        .get_mut(&run_id)
        .unwrap()
        .assessment_draft
        .saving = true;

    assert!(
        page.begin_history_load(session_id, "/backup".into(), false)
            .is_none()
    );
    assert_eq!(
        page.sessions[&session_id].directory,
        std::path::PathBuf::from("/original")
    );

    page.sessions
        .get_mut(&session_id)
        .unwrap()
        .runs
        .get_mut(&run_id)
        .unwrap()
        .assessment_draft
        .saving = false;
    assert!(
        page.begin_history_load(session_id, "/backup".into(), false)
            .is_some()
    );
}

#[test]
fn unreadable_history_hides_the_cached_run_but_preserves_unsaved_review() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let original = test_snapshot(session_id, run_id, RunStage::Ready);
    let criterion_id = original.inputs.exercise.checklist.criteria[0].id;
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(original.clone());
    page.sessions.get_mut(&session_id).unwrap().selected_run_id = Some(run_id);
    page.edit_correction(session_id, run_id, criterion_id, |draft| {
        draft.reason = "An unsaved review note".into();
    });
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    let generations = [(run_id, generation)].into();
    let archived = ArchivedRun {
        id: run_id,
        message: "The saved run cannot be read".into(),
        ..Default::default()
    };

    page.apply_history(
        session_id,
        Ok(RunHistory {
            runs: Vec::new(),
            archived: vec![archived.clone()],
        }),
        &generations,
    );

    let session = &page.sessions[&session_id];
    assert!(!session.runs[&run_id].available);
    assert_eq!(session.selected_run_id, None);
    assert_eq!(session.archived, vec![archived]);
    assert_eq!(
        session.runs[&run_id].corrections[&criterion_id].reason,
        "An unsaved review note"
    );

    page.apply_history(
        session_id,
        Ok(RunHistory {
            runs: vec![original],
            archived: Vec::new(),
        }),
        &generations,
    );
    assert!(page.sessions[&session_id].runs[&run_id].available);
    assert_eq!(page.sessions[&session_id].selected_run_id, Some(run_id));
    assert_eq!(
        page.sessions[&session_id].runs[&run_id].corrections[&criterion_id].reason,
        "An unsaved review note"
    );
}

#[test]
fn a_history_read_started_before_live_progress_cannot_hide_the_updated_run() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(test_snapshot(session_id, run_id, RunStage::Scanning));
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    page.apply_snapshot(test_snapshot(session_id, run_id, RunStage::Ready));

    page.apply_history(
        session_id,
        Ok(RunHistory::default()),
        &[(run_id, generation)].into(),
    );

    assert!(page.sessions[&session_id].runs[&run_id].available);
    assert_eq!(page.sessions[&session_id].selected_run_id, Some(run_id));
}

#[test]
fn a_history_refresh_cannot_hide_the_active_run_between_progress_callbacks() {
    let session_id = Uuid::new_v4();
    let run_id = Uuid::new_v4();
    let mut page = AnalysisPageState::default();
    page.apply_snapshot(test_snapshot(session_id, run_id, RunStage::Scanning));
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    page.active = Some(ActiveRun {
        token: Uuid::new_v4(),
        session_id,
        run_id: Some(run_id),
    });

    page.apply_history(
        session_id,
        Ok(RunHistory::default()),
        &[(run_id, generation)].into(),
    );

    assert!(page.sessions[&session_id].runs[&run_id].available);
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
    page.apply_snapshot(run.clone());
    let generation = page.sessions[&session_id].runs[&run_id].snapshot_generation;
    page.apply_corrections(session_id, run_id, vec![correction.clone()]);
    page.apply_history_snapshot(run, Some(generation));
    let updated = &page.sessions[&session_id].runs[&run_id].snapshot;
    assert_eq!(updated.checkpoint.stage, RunStage::Ready);
    assert_eq!(updated.checkpoint.assessments, model_assessments);
    assert_eq!(updated.corrections, vec![correction]);
}
