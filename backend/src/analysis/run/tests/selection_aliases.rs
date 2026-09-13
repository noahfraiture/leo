use super::super::selection_wire::{ReviewDispositionWire, SelectionWire};
use super::super::{jobs, selection_aliases::*, store, workflow, *};
use super::support::Fixture;
use crate::{
    analysis::{Judgment, ReviewDispositionKind, ReviewFinding},
    model::Completion,
};

fn setup() -> (Fixture, RunSnapshot, JobRecord) {
    let fixture = Fixture::new("http://127.0.0.1:9/v1");
    let mut snapshot = fixture.store.load(fixture.run_id).unwrap();
    snapshot.inputs.prompt_version = ALIASED_PROMPT.into();
    for job in &snapshot.checkpoint.jobs {
        let response = super::reader_response(job, &job.frame_ids[0]);
        let store::JobOutput::Reader(completion) = response.output else {
            unreachable!()
        };
        snapshot.reader_results.push(ReaderResult {
            job_id: job.id.clone(),
            completion,
        });
    }
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
                    judgment: Judgment::Met,
                    reason: "Cup placement is visible".into(),
                    evidence_ids: job.frame_ids.clone(),
                    facts: Vec::new(),
                    needs_more_evidence: false,
                    requested_intervals: Vec::new(),
                },
                usage: None,
                elapsed_ms: 1,
            },
        });
    }
    super::super::selection::ensure_jobs(&mut snapshot).unwrap();
    let job = snapshot
        .checkpoint
        .jobs
        .iter()
        .find(|job| job.kind == JobKind::SelectEvidence)
        .unwrap()
        .clone();
    (fixture, snapshot, job)
}

fn aliased(job: &JobRecord) -> AliasedSelection {
    AliasedSelection {
        aliases: SelectionAliases::for_job(job),
        completion: Completion {
            value: SelectionWire {
                reason: "Compare the visible placement".into(),
                reviews: vec![ReviewDispositionWire {
                    visual_review_ids: vec!["V001".into()],
                    kind: ReviewDispositionKind::Supporting,
                    reason: "The action is visible".into(),
                    decisive_candidate_frame_ids: vec!["F001".into()],
                    required_for_decision: true,
                }],
                supporting_candidate_frame_ids: vec!["F001".into()],
                contradicting_candidate_frame_ids: Vec::new(),
                context_candidate_frame_ids: Vec::new(),
            },
            usage: None,
            elapsed_ms: 1,
        },
    }
}

#[test]
fn v2_prompt_uses_aliases_and_metadata_while_v1_keeps_its_canonical_request() {
    let (fixture, mut snapshot, job) = setup();
    let prepared = jobs::prepare(&fixture.store, &snapshot, &job).unwrap();
    assert!(prepared.request.images.is_empty());
    assert!(!prepared.request.text.contains(&job.frame_ids[0]));
    assert!(!prepared.request.text.contains(&job.context_job_ids[0]));
    let document: serde_json::Value = serde_json::from_str(&prepared.request.text).unwrap();
    assert_eq!(
        document["candidateFrames"][0],
        serde_json::json!({"id":"F001","camera":1,"timeMs":0})
    );
    assert_eq!(document["detail"]["visualReviews"][0]["jobId"], "V001");
    assert!(prepared.request.text.contains("Cup placement is visible"));
    snapshot.inputs.prompt_version = LEGACY_PROMPT.into();
    let legacy = jobs::prepare(&fixture.store, &snapshot, &job).unwrap();
    assert!(legacy.selection_aliases.is_none());
    assert!(legacy.request.text.contains(&job.context_job_ids[0]));
    assert!(legacy.request.text.contains(&job.frame_ids[0]));
}

#[test]
fn repeated_set_members_normalize_without_changing_the_raw_selection() {
    let (_, _, job) = setup();
    let mut raw = aliased(&job);
    raw.completion
        .value
        .supporting_candidate_frame_ids
        .push("F001".into());
    raw.completion.value.reviews[0]
        .visual_review_ids
        .push("V001".into());
    raw.completion.value.reviews[0]
        .decisive_candidate_frame_ids
        .push("F001".into());
    let canonical = raw.canonical(&job).unwrap();
    assert_eq!(canonical.value.supporting_ids, job.frame_ids);
    assert_eq!(canonical.value.reviews[0].review_ids, job.context_job_ids);
    assert_eq!(
        raw.completion.value.supporting_candidate_frame_ids,
        ["F001", "F001"]
    );
}

#[test]
fn unknown_reference_or_canonical_ids_are_not_accepted_as_aliases() {
    let (_, _, job) = setup();
    for id in ["F999", "R001", "V001", "C1-T0", "F1"] {
        let mut raw = aliased(&job);
        raw.completion.value.supporting_candidate_frame_ids = vec![id.into()];
        assert!(raw.canonical(&job).is_err(), "accepted {id}");
    }
}

#[test]
fn review_membership_conflicts_and_omissions_are_not_normalized_away() {
    let (_, _, job) = setup();
    let mut raw = aliased(&job);
    raw.completion
        .value
        .reviews
        .push(raw.completion.value.reviews[0].clone());
    assert!(raw.canonical(&job).is_err());
    raw.completion.value.reviews.clear();
    assert!(raw.canonical(&job).is_err());
}

#[test]
fn saved_alias_maps_cannot_redirect_evidence_to_another_internal_id() {
    let (_, _, job) = setup();
    let mut raw = aliased(&job);
    raw.aliases.candidate_ids[0] = "C1-T99999".into();
    assert!(
        raw.canonical(&job)
            .unwrap_err()
            .to_string()
            .contains("immutable job")
    );
}

#[test]
fn translated_aliases_still_require_their_named_review_window() {
    let (fixture, mut snapshot, mut job) = setup();
    job.frame_ids.push("C1-T99999".into());
    let mut raw = aliased(&job);
    raw.completion.value.reviews[0].decisive_candidate_frame_ids = vec!["F002".into()];
    let response = store::SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output: store::JobOutput::SelectionV2(raw),
    };
    snapshot
        .checkpoint
        .jobs
        .iter_mut()
        .find(|source| source.id == job.id)
        .unwrap()
        .frame_ids = job.frame_ids.clone();
    store::write_json(
        &fixture
            .store
            .directory(fixture.run_id)
            .unwrap()
            .join("inputs.json"),
        &snapshot.inputs,
        true,
    )
    .unwrap();
    fixture
        .store
        .save_checkpoint(fixture.run_id, &snapshot.checkpoint)
        .unwrap();
    let error = fixture
        .store
        .save_response(fixture.run_id, &job, &response)
        .unwrap_err();
    assert!(error.to_string().contains("named visual review windows"));
    // The same validation protects reloading an externally altered persisted response.
    store::write_json(
        &fixture
            .store
            .directory(fixture.run_id)
            .unwrap()
            .join("responses")
            .join(format!("{}.json", job.id)),
        &response,
        false,
    )
    .unwrap();
    assert!(
        fixture
            .store
            .response(fixture.run_id, &job)
            .unwrap_err()
            .to_string()
            .contains("named visual review windows")
    );
}

#[test]
fn persisted_v2_keeps_the_raw_alias_reply_and_rejects_a_v1_output_under_v2_inputs() {
    let (fixture, snapshot, job) = setup();
    store::write_json(
        &fixture
            .store
            .directory(fixture.run_id)
            .unwrap()
            .join("inputs.json"),
        &snapshot.inputs,
        true,
    )
    .unwrap();
    fixture
        .store
        .save_checkpoint(fixture.run_id, &snapshot.checkpoint)
        .unwrap();
    let raw = aliased(&job);
    let mut response = store::SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output: store::JobOutput::Selection(raw.canonical(&job).unwrap()),
    };
    assert!(
        fixture
            .store
            .save_response(fixture.run_id, &job, &response)
            .unwrap_err()
            .to_string()
            .contains("frozen prompt version")
    );
    response.output = store::JobOutput::SelectionV2(raw.clone());
    fixture
        .store
        .save_response(fixture.run_id, &job, &response)
        .unwrap();
    assert_eq!(
        fixture
            .store
            .response(fixture.run_id, &job)
            .unwrap()
            .unwrap(),
        response
    );
    let loaded = fixture.store.load(fixture.run_id).unwrap();
    assert_eq!(
        loaded.selection_results[0].completion.value.supporting_ids,
        job.frame_ids
    );
    assert_eq!(
        raw.completion.value.supporting_candidate_frame_ids,
        ["F001"]
    );
}

#[test]
fn wire_schema_distinguishes_review_ids_from_candidate_frame_ids() {
    let schema = serde_json::to_value(rig_core::schemars::schema_for!(SelectionWire)).unwrap();
    for field in [
        "supportingCandidateFrameIds",
        "contradictingCandidateFrameIds",
        "contextCandidateFrameIds",
    ] {
        assert_eq!(
            schema["properties"][field]["items"]["pattern"],
            "^F[0-9]{3,}$"
        );
    }
    let disposition = &schema["$defs"]["ReviewDispositionWire"]["properties"];
    assert_eq!(
        disposition["visualReviewIds"]["items"]["pattern"],
        "^V[0-9]{3,}$"
    );
    assert_eq!(
        disposition["decisiveCandidateFrameIds"]["items"]["pattern"],
        "^F[0-9]{3,}$"
    );
}
