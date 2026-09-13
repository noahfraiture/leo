mod evidence_selection;
mod support;

use super::super::reconcile::{Judgment, ReaderFinding, ReviewFinding};
use super::store::{JobOutput, SavedResponse};
use super::*;
use crate::model::Completion;
use support::{Fixture, ModelServer};

fn reader_response(job: &JobRecord, evidence_id: &str) -> SavedResponse {
    SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: job
            .frame_ids
            .iter()
            .map(|id| crate::analysis::SentImage {
                candidate_id: id.clone(),
                sha256: super::store::hash(b"fixture jpeg"),
                size_bytes: 12,
            })
            .collect(),
        reference_hashes: Vec::new(),
        output: JobOutput::Reader(Completion {
            value: ReaderFinding {
                judgment: Judgment::Met,
                reason: "Cup placement is visible".into(),
                evidence_ids: vec![evidence_id.into()],
                facts: Vec::new(),
            },
            usage: None,
            elapsed_ms: 1,
        }),
    }
}

#[test]
fn response_survives_a_crash_before_its_checkpoint_update() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let before = fixture.store.load(fixture.run_id).unwrap();
    let first = &before.checkpoint.jobs[0];
    fixture
        .store
        .save_response(fixture.run_id, first, &reader_response(first, "C1-T0"))
        .unwrap();
    let recovered = fixture.store.load(fixture.run_id).unwrap();
    assert_eq!(recovered.checkpoint.jobs[0].state, JobState::Complete);
    assert_eq!(recovered.checkpoint.jobs[1].state, JobState::Pending);
    assert_eq!(recovered.reader_results.len(), 1);
}

#[test]
fn invented_or_reference_citations_cannot_be_published_as_candidate_evidence() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let snapshot = fixture.store.load(fixture.run_id).unwrap();
    let job = &snapshot.checkpoint.jobs[0];
    for invalid_id in ["R-reference", "C1-T99999"] {
        assert!(
            fixture
                .store
                .save_response(fixture.run_id, job, &reader_response(job, invalid_id))
                .is_err()
        );
    }
    assert!(
        fixture
            .store
            .load(fixture.run_id)
            .unwrap()
            .reader_results
            .is_empty()
    );
}

#[tokio::test]
async fn failed_reader_does_not_erase_other_readers_and_resume_skips_saved_work() {
    let server = ModelServer::new(true);
    let fixture = Fixture::new(&server.base_url);
    let paused = execute_run(fixture.execute_request(), |_| {})
        .await
        .unwrap();
    assert_eq!(paused.checkpoint.stage, RunStage::Paused);
    assert_eq!(paused.reader_results.len(), 1);
    let saved_reader = paused.reader_results[0].clone();
    let mut stages = Vec::new();
    let ready = execute_run(fixture.execute_request(), |snapshot| {
        stages.push(snapshot.checkpoint.stage)
    })
    .await
    .unwrap();
    assert_eq!(ready.checkpoint.stage, RunStage::Ready);
    assert!(ready.reader_results.contains(&saved_reader));
    assert_eq!(ready.checkpoint.assessments.len(), 1);
    assert_eq!(ready.checkpoint.assessments[0].judgment, Judgment::Met);
    let calls = server.requests.lock().unwrap();
    assert_eq!(
        calls
            .iter()
            .filter(|request| request["model"] == "reader-a")
            .count(),
        2
    );
    assert_eq!(
        calls
            .iter()
            .filter(|request| request["model"] == "reader-b")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|request| request["model"] == "reviewer")
            .count(),
        3
    );
    assert!(stages.contains(&RunStage::Reviewing));
    assert!(stages.contains(&RunStage::FinalReview));
}

#[test]
fn one_additional_round_remains_bounded_after_repeated_workflow_calls() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    workflow::ensure_reviews(&mut snapshot).unwrap();
    let job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Review)
        .unwrap();
    snapshot.review_results.push(ReviewResult {
        job_id: job.id.clone(),
        completion: Completion {
            value: ReviewFinding {
                judgment: Judgment::CannotVerify,
                reason: "Need nearby frames".into(),
                evidence_ids: Vec::new(),
                facts: Vec::new(),
                needs_more_evidence: true,
                requested_intervals: vec![crate::analysis::EvidenceRequest {
                    camera_id: 1,
                    start_ms: 500,
                    end_ms: 1000,
                }],
            },
            usage: None,
            elapsed_ms: 1,
        },
    });
    workflow::gather(&mut snapshot).unwrap();
    let after = snapshot.checkpoint.clone();
    assert!(after.evidence.frames.len() > 1);
    assert_eq!(after.gathered_criteria.len(), 1);
    workflow::gather(&mut snapshot).unwrap();
    assert_eq!(snapshot.checkpoint, after);
}

#[test]
fn criteria_share_cached_frames_without_losing_their_independent_revisit_jobs() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    let first_id = snapshot.inputs.exercise.checklist.criteria[0].id;
    let second_id = uuid::Uuid::new_v4();
    snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .push(crate::exercise::Criterion {
            id: second_id,
            text: "The cup is upright".into(),
        });
    let mut second_windows = snapshot.inputs.windows[0].clone();
    second_windows.criterion_id = second_id;
    snapshot.inputs.windows.push(second_windows);
    workflow::ensure_reviews(&mut snapshot).unwrap();
    for job in snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.kind == JobKind::Review)
    {
        snapshot.review_results.push(ReviewResult {
            job_id: job.id.clone(),
            completion: Completion {
                value: ReviewFinding {
                    judgment: Judgment::CannotVerify,
                    reason: "Inspect the same nearby interval for this criterion".into(),
                    evidence_ids: Vec::new(),
                    facts: Vec::new(),
                    needs_more_evidence: true,
                    requested_intervals: vec![crate::analysis::EvidenceRequest {
                        camera_id: 1,
                        start_ms: 500,
                        end_ms: 1000,
                    }],
                },
                usage: None,
                elapsed_ms: 1,
            },
        });
    }
    workflow::gather(&mut snapshot).unwrap();
    let frames_for = |criterion_id, kind| {
        snapshot
            .checkpoint
            .jobs
            .iter()
            .filter(|job| job.criterion_id == criterion_id && job.kind == kind && job.round == 1)
            .flat_map(|job| job.frame_ids.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let expected = frames_for(first_id, JobKind::Reader);
    assert!(!expected.is_empty());
    assert_eq!(frames_for(second_id, JobKind::Reader), expected);
    assert_eq!(frames_for(second_id, JobKind::Review), expected);
    assert_eq!(
        snapshot.checkpoint.evidence.frames.len(),
        snapshot.inputs.initial_evidence.frames.len() + expected.len()
    );
}

#[test]
fn correction_history_preserves_model_results_and_requires_the_latest_revision() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let snapshot = fixture.store.load(fixture.run_id).unwrap();
    let job = &snapshot.checkpoint.jobs[0];
    fixture
        .store
        .save_response(fixture.run_id, job, &reader_response(job, "C1-T0"))
        .unwrap();
    let correction = HumanCorrection {
        id: uuid::Uuid::new_v4(),
        criterion_id: snapshot.inputs.exercise.checklist.criteria[0].id,
        judgment: Judgment::CannotVerify,
        reason: "The object is occluded".into(),
        evidence_ids: vec!["C1-T0".into()],
        uses_outside_information: false,
        created_utc_ms: 1,
        supersedes: None,
    };
    fixture
        .store
        .append_human_correction(fixture.run_id, correction.clone())
        .unwrap();
    let stale = HumanCorrection {
        id: uuid::Uuid::new_v4(),
        ..correction.clone()
    };
    assert!(
        fixture
            .store
            .append_human_correction(fixture.run_id, stale)
            .is_err()
    );
    let after = fixture.store.load(fixture.run_id).unwrap();
    assert_eq!(after.corrections, [correction]);
    assert_eq!(
        after.reader_results[0].completion.value.judgment,
        Judgment::Met
    );
}

#[tokio::test]
async fn changed_recording_pauses_with_a_durable_failure_before_any_request() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    std::fs::write(
        fixture.store.session_directory.join("source.mkv"),
        b"changed",
    )
    .unwrap();
    let mut seen = Vec::new();
    assert!(
        execute_run(fixture.execute_request(), |snapshot| seen
            .push(snapshot.checkpoint))
        .await
        .is_err()
    );
    assert_eq!(seen.last().unwrap().stage, RunStage::Paused);
    assert!(
        fixture
            .store
            .load(fixture.run_id)
            .unwrap()
            .checkpoint
            .error
            .is_some()
    );
}
