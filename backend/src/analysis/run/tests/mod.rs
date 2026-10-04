mod references;
mod review;
mod support;
mod validation;
use super::*;
use support::{Fixture, ModelServer, request_body};

#[tokio::test]
async fn complete_scan_uses_one_request_per_window_and_one_consolidation() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let result = execute_run(fixture.request(), |_| {}).await.unwrap();
    assert_eq!(result.checkpoint.stage, RunStage::Ready);
    assert_eq!(result.checkpoint.assessments.len(), 2);
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    let bodies = requests.iter().map(request_body).collect::<Vec<_>>();
    for (body, window) in bodies[..2].iter().zip(&result.inputs.windows) {
        assert_eq!(
            body["detail"]["checklist"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            body["candidateFrames"]
                .as_array()
                .unwrap()
                .iter()
                .map(|frame| frame["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            window.frame_ids
        );
    }
    assert_eq!(
        bodies[2]["detail"]["observations"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn provider_failure_pauses_without_retry_and_explicit_resume_keeps_failure_history() {
    let server = ModelServer::new(true);
    let fixture = Fixture::new(&server.base_url);
    assert!(execute_run(fixture.request(), |_| {}).await.is_err());
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let paused = fixture.snapshot();
    assert_eq!(paused.checkpoint.stage, RunStage::Paused);
    assert!(paused.checkpoint.assessments.is_empty());
    let completed = execute_run(fixture.request(), |_| {}).await.unwrap();
    assert_eq!(completed.checkpoint.jobs[0].failures.len(), 1);
    assert_eq!(server.requests.lock().unwrap().len(), 4);
    execute_run(fixture.request(), |_| {}).await.unwrap();
    assert_eq!(
        server.requests.lock().unwrap().len(),
        4,
        "completed run must not call a provider again"
    );
}

#[tokio::test]
async fn response_survives_a_missing_checkpoint_update_without_duplicate_inference() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let initial = fixture.snapshot();
    execute_run(fixture.request(), |_| {}).await.unwrap();
    // Simulate interruption after all scan responses were written but before jobs were marked complete.
    fixture
        .store
        .save_checkpoint(fixture.run_id, &initial.checkpoint)
        .unwrap();
    let result = execute_run(fixture.request(), |_| {}).await;
    // Consolidation has a stable ID too, so recovery must reuse its existing response.
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(server.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn closer_look_is_independent_and_automatic_and_manual_share_one_allowance() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let initial = execute_run(fixture.request(), |_| {}).await.unwrap();
    let id = initial.inputs.exercise.checklist.criteria[0].id;
    let queued = request_recheck(
        fixture.store.session_directory.clone(),
        fixture.run_id,
        id,
        None,
    )
    .await
    .unwrap();
    assert!(
        request_recheck(
            fixture.store.session_directory.clone(),
            fixture.run_id,
            id,
            None
        )
        .await
        .is_err()
    );
    fixture.cache_frames();
    let job = queued.checkpoint.jobs.last().unwrap();
    let prepared = super::jobs::prepare(&fixture.store, &queued, job).unwrap();
    let text: serde_json::Value = serde_json::from_str(&prepared.request.text).unwrap();
    assert!(text["detail"].get("criterion").is_some());
    assert_eq!(
        text["detail"]["checklist"]["criteria"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "the independent judge needs neighboring requirements to interpret order"
    );
    assert!(
        !prepared
            .request
            .text
            .contains(&initial.checkpoint.assessments[0].reason)
    );
    assert!(text["detail"].get("observations").is_none());
    assert!(prepared.request.images.len() > initial.checkpoint.assessments[0].evidence_ids.len());
    let result = execute_run(fixture.request(), |_| {}).await.unwrap();
    assert!(result.checkpoint.assessments[0].recheck.is_some());
    assert_eq!(
        result.checkpoint.assessments[0].initial,
        initial.checkpoint.assessments[0].initial
    );
}

#[test]
fn removed_criteria_cannot_supply_active_operator_policy() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    let removed = snapshot.inputs.exercise.checklist.criteria.remove(1);
    snapshot
        .inputs
        .exercise
        .preparation
        .questions
        .push(crate::exercise::Clarification {
            criterion_id: removed.id,
            source_text: removed.text,
            question: "Where must this discarded step happen?".into(),
            answer: "On the shelf".into(),
        });
    let prepared =
        super::jobs::prepare(&fixture.store, &snapshot, &snapshot.checkpoint.jobs[0]).unwrap();
    let text: serde_json::Value = serde_json::from_str(&prepared.request.text).unwrap();
    assert_eq!(text["operatorClarifications"], serde_json::json!([]));
}

#[tokio::test]
async fn rechecks_for_two_criteria_share_samples_without_competing_for_them() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let mut snapshot = execute_run(fixture.request(), |_| {}).await.unwrap();
    let ids = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    super::recheck::queue(&mut snapshot, ids[0], None).unwrap();
    let first = snapshot.checkpoint.jobs.last().unwrap().frame_ids.clone();
    super::recheck::queue(&mut snapshot, ids[1], None).unwrap();
    assert_eq!(snapshot.checkpoint.jobs.last().unwrap().frame_ids, first);
    let ids = snapshot
        .checkpoint
        .evidence
        .frames
        .iter()
        .map(|frame| &frame.id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(ids.len(), snapshot.checkpoint.evidence.frames.len());
}
