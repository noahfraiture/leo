use super::super::{
    FinalResult, JobKind, JobState, ReviewResult, RunSnapshot, SelectionResult, jobs, selection,
};
use super::support::Fixture;
use crate::{
    analysis::{
        EvidenceSelection, FinalFinding, Judgment, ReviewDisposition, ReviewDispositionKind,
        ReviewFinding,
    },
    model::Completion,
};

fn add_review(snapshot: &mut RunSnapshot, offset: u64, judgment: Judgment) -> (String, String) {
    let mut frame = snapshot.checkpoint.evidence.frames[0].clone();
    frame.id = format!("C1-T{offset}");
    frame.session_offset_ms = offset;
    frame.recording_offset_ms = offset;
    if snapshot.checkpoint.evidence.frame(&frame.id).is_none() {
        snapshot.checkpoint.evidence.frames.push(frame.clone());
    }
    snapshot.checkpoint.evidence.duration_ms = 60_000;
    snapshot.checkpoint.evidence.sources[0].end_utc_ms = 60_000;
    let mut job = jobs::job_record(
        &snapshot.inputs,
        snapshot.inputs.exercise.checklist.criteria[0].id,
        &snapshot.inputs.model_set.reviewer,
        JobKind::Review,
        0,
        vec![frame.id.clone()],
    )
    .unwrap();
    job.state = JobState::Complete;
    snapshot.review_results.push(ReviewResult {
        job_id: job.id.clone(),
        completion: Completion {
            value: ReviewFinding {
                judgment,
                reason: "Visual review report for this window".into(),
                evidence_ids: vec![frame.id.clone()],
                facts: Vec::new(),
                needs_more_evidence: false,
                requested_intervals: Vec::new(),
            },
            usage: None,
            elapsed_ms: 1,
        },
    });
    let id = job.id.clone();
    snapshot.checkpoint.jobs.push(job);
    (id, frame.id)
}

fn add_selection(
    snapshot: &mut RunSnapshot,
    dispositions: Vec<ReviewDisposition>,
    supporting_ids: Vec<String>,
) {
    selection::ensure_jobs(snapshot).unwrap();
    let job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::SelectEvidence)
        .unwrap();
    let value = EvidenceSelection {
        reason: "Compare decisive evidence with contrary observations".into(),
        reviews: dispositions,
        supporting_ids,
        contradicting_ids: Vec::new(),
        context_ids: Vec::new(),
    };
    super::super::super::reconcile::validate_selection(
        &value,
        &job.frame_ids,
        &job.context_job_ids,
    )
    .unwrap();
    snapshot.selection_results.push(SelectionResult {
        job_id: job.id.clone(),
        completion: Completion {
            value,
            usage: None,
            elapsed_ms: 1,
        },
    });
}

#[test]
fn long_recording_background_citations_do_not_erase_a_decisive_late_action() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    snapshot.inputs.model_set.reviewer.profile.image_limit = 2;
    let mut background = Vec::new();
    for offset in (0..49_000).step_by(1000) {
        background.push(add_review(&mut snapshot, offset, Judgment::CannotVerify).0);
    }
    let (decisive_review, decisive_frame) = add_review(&mut snapshot, 59_000, Judgment::Met);
    add_selection(
        &mut snapshot,
        vec![
            ReviewDisposition {
                review_ids: background,
                kind: ReviewDispositionKind::Irrelevant,
                reason: "These windows show no action relevant to the criterion".into(),
                decisive_ids: Vec::new(),
                required_for_decision: false,
            },
            ReviewDisposition {
                review_ids: vec![decisive_review],
                kind: ReviewDispositionKind::Supporting,
                reason: "The actual cup placement occurs here".into(),
                decisive_ids: vec![decisive_frame.clone()],
                required_for_decision: true,
            },
        ],
        vec![decisive_frame.clone()],
    );
    selection::ensure_finals(&mut snapshot).unwrap();
    let final_job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Final)
        .unwrap();
    assert_eq!(final_job.frame_ids, [decisive_frame.clone()]);
    snapshot.final_results.push(FinalResult {
        job_id: final_job.id.clone(),
        completion: Completion {
            value: FinalFinding {
                judgment: Judgment::Met,
                reason: "The final image pass sees the placement".into(),
                evidence_ids: vec![decisive_frame],
                unresolved_facts: Vec::new(),
                requires_unseen_comparison: false,
            },
            usage: None,
            elapsed_ms: 1,
        },
    });
    selection::assess(&mut snapshot).unwrap();
    assert_eq!(snapshot.checkpoint.assessments[0].judgment, Judgment::Met);
    assert_eq!(snapshot.review_results.len(), 50);
}

#[test]
fn selector_cannot_hide_explicit_contrary_evidence_to_fit_the_image_limit() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    snapshot.inputs.model_set.reviewer.profile.image_limit = 1;
    let (positive_review, positive) = add_review(&mut snapshot, 0, Judgment::Met);
    let (negative_review, negative) = add_review(&mut snapshot, 59_000, Judgment::NotMet);
    add_selection(
        &mut snapshot,
        vec![
            ReviewDisposition {
                review_ids: vec![positive_review],
                kind: ReviewDispositionKind::Supporting,
                reason: "Support".into(),
                decisive_ids: vec![positive.clone()],
                required_for_decision: true,
            },
            ReviewDisposition {
                review_ids: vec![negative_review],
                kind: ReviewDispositionKind::Irrelevant,
                reason: "The selector mistakenly dismissed this contrary observation".into(),
                decisive_ids: Vec::new(),
                required_for_decision: false,
            },
        ],
        vec![positive],
    );
    selection::ensure_finals(&mut snapshot).unwrap();
    selection::assess(&mut snapshot).unwrap();
    assert_eq!(
        snapshot.checkpoint.assessments[0].judgment,
        Judgment::CannotVerify
    );
    assert!(
        snapshot.checkpoint.assessments[0]
            .evidence_ids
            .contains(&negative)
    );
    assert!(snapshot.checkpoint.assessments[0].final_job_id.is_none());
    assert!(snapshot.final_results.is_empty());
    assert_eq!(
        snapshot.review_results[1].completion.value.judgment,
        Judgment::NotMet
    );
}

#[test]
fn contradictory_facts_in_an_uncertain_review_are_included_by_its_disposition() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    snapshot.inputs.model_set.reviewer.profile.image_limit = 2;
    let (positive_review, positive) = add_review(&mut snapshot, 0, Judgment::Met);
    let (uncertain_review, contrary) = add_review(&mut snapshot, 59_000, Judgment::CannotVerify);
    add_selection(&mut snapshot, vec![
        ReviewDisposition { review_ids: vec![positive_review], kind: ReviewDispositionKind::Supporting, reason: "Visible support".into(), decisive_ids: vec![positive.clone()], required_for_decision: true },
        ReviewDisposition { review_ids: vec![uncertain_review], kind: ReviewDispositionKind::Contradictory, reason: "This ambiguous later view could contradict completion and needs visual comparison".into(), decisive_ids: vec![contrary.clone()], required_for_decision: true },
    ], vec![positive.clone()]);
    selection::ensure_finals(&mut snapshot).unwrap();
    let final_job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Final)
        .unwrap();
    assert_eq!(final_job.frame_ids, [positive, contrary]);
}

#[test]
fn selection_must_account_for_every_visual_review() {
    let value = EvidenceSelection {
        reason: "An incomplete selection".into(),
        reviews: Vec::new(),
        supporting_ids: Vec::new(),
        contradicting_ids: Vec::new(),
        context_ids: Vec::new(),
    };
    assert!(
        super::super::super::reconcile::validate_selection(&value, &[], &["omitted-review".into()])
            .is_err()
    );
}

#[test]
fn optional_contradictions_still_require_visual_evidence() {
    let value = EvidenceSelection {
        reason: "A contrary observation must not disappear".into(),
        reviews: vec![ReviewDisposition {
            review_ids: vec!["uncertain-review".into()],
            kind: ReviewDispositionKind::Contradictory,
            reason: "This uncertain view may contradict completion".into(),
            decisive_ids: Vec::new(),
            required_for_decision: false,
        }],
        supporting_ids: Vec::new(),
        contradicting_ids: Vec::new(),
        context_ids: Vec::new(),
    };
    assert!(
        super::super::super::reconcile::validate_selection(
            &value,
            &["C1-T0".into()],
            &["uncertain-review".into()],
        )
        .unwrap_err()
        .contains("decisive candidate IDs")
    );
}

#[test]
fn real_images_from_another_window_cannot_stand_in_for_a_contradiction() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    let (positive_review, positive) = add_review(&mut snapshot, 0, Judgment::Met);
    let (uncertain_review, _) = add_review(&mut snapshot, 59_000, Judgment::CannotVerify);
    add_selection(
        &mut snapshot,
        vec![
            ReviewDisposition {
                review_ids: vec![positive_review],
                kind: ReviewDispositionKind::Supporting,
                reason: "Visible support".into(),
                decisive_ids: vec![positive.clone()],
                required_for_decision: true,
            },
            ReviewDisposition {
                review_ids: vec![uncertain_review],
                kind: ReviewDispositionKind::Contradictory,
                reason: "This later view could contradict completion".into(),
                decisive_ids: vec![positive.clone()],
                required_for_decision: false,
            },
        ],
        vec![positive],
    );
    fixture
        .store
        .save_checkpoint(fixture.run_id, &snapshot.checkpoint)
        .unwrap();
    let job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::SelectEvidence)
        .unwrap();
    let response = super::super::store::SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output: super::super::store::JobOutput::Selection(
            snapshot.selection_results[0].completion.clone(),
        ),
    };
    let error = fixture
        .store
        .save_response(fixture.run_id, job, &response)
        .unwrap_err();
    assert!(error.to_string().contains("named visual review windows"));
    assert!(
        fixture
            .store
            .response(fixture.run_id, job)
            .unwrap()
            .is_none()
    );
}

#[test]
fn overruled_negative_reader_evidence_remains_in_the_actual_final_comparison() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    snapshot.inputs.model_set.reviewer.profile.image_limit = 2;
    let (positive_review, positive) = add_review(&mut snapshot, 59_000, Judgment::Met);
    let (background_review, contrary) = add_review(&mut snapshot, 0, Judgment::CannotVerify);
    let reader = &snapshot.checkpoint.jobs[0];
    snapshot.reader_results.push(super::super::ReaderResult {
        job_id: reader.id.clone(),
        completion: Completion {
            value: crate::analysis::ReaderFinding {
                judgment: Judgment::NotMet,
                reason: "The reader reports a prohibited act".into(),
                evidence_ids: vec![contrary.clone()],
                facts: Vec::new(),
            },
            usage: None,
            elapsed_ms: 1,
        },
    });
    add_selection(
        &mut snapshot,
        vec![
            ReviewDisposition {
                review_ids: vec![positive_review],
                kind: ReviewDispositionKind::Supporting,
                reason: "Visible support".into(),
                decisive_ids: vec![positive.clone()],
                required_for_decision: true,
            },
            ReviewDisposition {
                review_ids: vec![background_review],
                kind: ReviewDispositionKind::Irrelevant,
                reason: "Reviewer dismissed the negative reader observation".into(),
                decisive_ids: Vec::new(),
                required_for_decision: false,
            },
        ],
        vec![positive.clone()],
    );
    selection::ensure_finals(&mut snapshot).unwrap();
    let final_job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Final)
        .unwrap();
    assert_eq!(final_job.frame_ids, [contrary, positive]);
}

#[test]
fn required_unresolved_comparison_cannot_receive_credit_without_a_final_image_pass() {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    let (review, image) = add_review(&mut snapshot, 0, Judgment::CannotVerify);
    add_selection(
        &mut snapshot,
        vec![ReviewDisposition {
            review_ids: vec![review],
            kind: ReviewDispositionKind::Unresolved,
            reason: "The object's internal state is hidden".into(),
            decisive_ids: vec![image.clone()],
            required_for_decision: true,
        }],
        vec![image],
    );
    selection::ensure_finals(&mut snapshot).unwrap();
    selection::assess(&mut snapshot).unwrap();
    assert!(
        snapshot
            .checkpoint
            .jobs
            .iter()
            .all(|job| job.kind != JobKind::Final)
    );
    assert_eq!(
        snapshot.checkpoint.assessments[0].judgment,
        Judgment::CannotVerify
    );
    assert!(snapshot.checkpoint.assessments[0].final_job_id.is_none());
}

#[tokio::test]
async fn selector_sends_only_text_and_finalizer_sends_the_selected_candidate_bytes() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::Value;

    let server = super::support::ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let ready = super::super::execute_run(fixture.execute_request(), |_| {})
        .await
        .unwrap();
    let calls = server.requests.lock().unwrap();
    let mut visual_final_count = 0;
    let mut text_selection_count = 0;
    for request in calls.iter() {
        let content = &request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "user")
            .unwrap()["content"];
        let text = content
            .as_str()
            .or_else(|| content[0]["text"].as_str())
            .unwrap();
        let body: Value = serde_json::from_str(text).unwrap();
        match body["jobKind"].as_str().unwrap() {
            "selectEvidence" => {
                assert!(content.is_string());
                text_selection_count += 1;
            }
            "final" => {
                let images: Vec<_> = content
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|part| part["type"] == "image_url")
                    .collect();
                assert_eq!(images.len(), 1);
                assert_eq!(body["candidateImageIds"], serde_json::json!(["C1-T0"]));
                assert_eq!(
                    images[0]["image_url"]["url"],
                    format!(
                        "data:image/jpeg;base64,{}",
                        STANDARD.encode(b"fixture jpeg")
                    )
                );
                visual_final_count += 1;
            }
            _ => {}
        }
    }
    assert_eq!((text_selection_count, visual_final_count), (1, 1));
    let final_job = ready
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::Final)
        .unwrap();
    let saved = fixture
        .store
        .response(fixture.run_id, final_job)
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.sent_images[0].sha256,
        super::super::store::hash(b"fixture jpeg")
    );
}
