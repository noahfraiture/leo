use super::*;
use uuid::Uuid;

#[tokio::test]
async fn a_correction_invalidates_only_its_review_and_the_overall_assessment() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let mut snapshot = execute_run(fixture.request(), |_| {}).await.unwrap();
    let ids: Vec<_> = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .map(|item| item.id)
        .collect();
    for id in &ids {
        snapshot = fixture
            .store
            .mark_reviewed(fixture.run_id, *id, &snapshot.criterion_basis(*id).unwrap())
            .unwrap();
    }
    snapshot = fixture
        .store
        .save_student_assessment(
            fixture.run_id,
            StudentOutcome::NeedsEvidence,
            "Original views do not establish the complete action.".into(),
            &snapshot.assessment_basis().unwrap(),
        )
        .unwrap();
    assert!(snapshot.current_student_assessment().is_some());
    fixture
        .store
        .append_human_correction(
            fixture.run_id,
            HumanCorrection {
                id: Uuid::new_v4(),
                criterion_id: ids[0],
                judgment: Judgment::Met,
                reason: "I reviewed the complete episode.".into(),
                evidence_ids: vec!["C1-T0".into()],
                uses_outside_information: false,
                created_utc_ms: 4,
                supersedes: None,
            },
        )
        .unwrap();
    let changed = fixture.snapshot();
    assert!(!changed.is_reviewed(ids[0]));
    assert!(changed.is_reviewed(ids[1]));
    assert!(changed.current_student_assessment().is_none());
    assert_eq!(changed.operator_review.assessments.len(), 1);
    assert_eq!(
        changed.checkpoint.assessments, snapshot.checkpoint.assessments,
        "human edits preserve model outcomes"
    );
    assert!(
        fixture
            .store
            .mark_reviewed(
                fixture.run_id,
                ids[0],
                &snapshot.criterion_basis(ids[0]).unwrap()
            )
            .is_err()
    );
}

#[tokio::test]
async fn student_assessment_requires_every_criterion_reviewed_and_an_explanation() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let snapshot = execute_run(fixture.request(), |_| {}).await.unwrap();
    assert!(
        fixture
            .store
            .save_student_assessment(
                fixture.run_id,
                StudentOutcome::Pass,
                "Reviewed".into(),
                &snapshot.assessment_basis().unwrap()
            )
            .is_err()
    );
    for item in &snapshot.inputs.exercise.checklist.criteria {
        fixture
            .store
            .mark_reviewed(
                fixture.run_id,
                item.id,
                &snapshot.criterion_basis(item.id).unwrap(),
            )
            .unwrap();
    }
    assert!(
        fixture
            .store
            .save_student_assessment(
                fixture.run_id,
                StudentOutcome::Pass,
                " ".into(),
                &snapshot.assessment_basis().unwrap()
            )
            .is_err()
    );
}

#[tokio::test]
async fn a_queued_or_failed_recheck_makes_a_saved_student_assessment_historical() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let mut snapshot = execute_run(fixture.request(), |_| {}).await.unwrap();
    snapshot
        .operator_review
        .assessments
        .push(StudentAssessment {
            outcome: StudentOutcome::NeedsEvidence,
            note: "Awaiting clearer footage".into(),
            basis: snapshot.assessment_basis().unwrap(),
            created_utc_ms: 1,
        });
    assert!(snapshot.current_student_assessment().is_some());
    for stage in [RunStage::Rechecking, RunStage::Paused] {
        snapshot.checkpoint.stage = stage;
        assert!(snapshot.current_student_assessment().is_none());
        assert_eq!(snapshot.operator_review.assessments.len(), 1);
    }
}

#[test]
fn old_and_damaged_runs_do_not_hide_new_history_or_modify_original_files() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let root = fixture.store.session_directory.join("analyses");
    let old = root.join(Uuid::new_v4().to_string());
    let damaged = root.join(Uuid::new_v4().to_string());
    std::fs::create_dir(&old).unwrap();
    std::fs::create_dir(&damaged).unwrap();
    let snapshot = fixture.snapshot();
    let inputs = serde_json::json!({"schemaVersion":1,"exercise":snapshot.inputs.exercise});
    let bytes = serde_json::to_vec(&inputs).unwrap();
    std::fs::write(old.join("inputs.json"), &bytes).unwrap();
    std::fs::write(old.join("checkpoint.json"), b"{\"assessments\":[]}").unwrap();
    std::fs::write(damaged.join("inputs.json"), b"interrupted").unwrap();
    let history = fixture.store.history().unwrap();
    assert_eq!(history.runs.len(), 1);
    assert_eq!(history.archived.len(), 2);
    assert_eq!(std::fs::read(old.join("inputs.json")).unwrap(), bytes);
}
