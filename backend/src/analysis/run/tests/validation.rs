use super::super::{jobs, results};
use super::*;
use crate::analysis::SentImage;
use crate::model::Completion;

#[tokio::test]
async fn provider_connection_rename_keeps_paused_run_inputs_and_allows_resume() {
    let server = ModelServer::new(true);
    let fixture = Fixture::new(&server.base_url);
    assert!(execute_run(fixture.request(), |_| {}).await.is_err());
    let frozen = fixture.snapshot().inputs;
    let mut request = fixture.request();
    request.providers[0].name = "Renamed local connection".into();
    request.providers[0].api_key = "rotated-local-fixture".into();

    let resumed = execute_run(request, |_| {}).await.unwrap();

    assert_eq!(resumed.checkpoint.stage, RunStage::Ready);
    assert_eq!(resumed.inputs, frozen);
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn provider_connection_destination_changes_stay_rejected() {
    for change_transport in [false, true] {
        let server = ModelServer::new(false);
        let fixture = Fixture::new(&server.base_url);
        let mut request = fixture.request();
        if change_transport {
            request.providers[0].transport = crate::model::Transport::Responses;
        } else {
            request.providers[0].base_url.push_str("/changed");
        }

        let error = execute_run(request, |_| {}).await.unwrap_err();

        assert!(error.to_string().contains("saved model connection changed"));
        assert!(server.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn oversized_json_cannot_replace_a_checkpoint_or_publish_a_new_file() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let directory = fixture.store.directory(fixture.run_id).unwrap();
    let path = directory.join("checkpoint.json");
    let original = std::fs::read(&path).unwrap();
    let mut checkpoint = fixture.snapshot().checkpoint;
    checkpoint.error = Some("x".repeat(64 * 1024 * 1024));

    assert!(
        fixture
            .store
            .save_checkpoint(fixture.run_id, &checkpoint)
            .is_err()
    );
    assert_eq!(std::fs::read(path).unwrap(), original);
    let unpublished = directory.join("new-checkpoint.json");
    assert!(super::super::store::write_json(&unpublished, &checkpoint, false).is_err());
    assert!(!unpublished.exists());
    assert!(fixture.store.load(fixture.run_id).is_ok());
}

#[tokio::test]
async fn creation_rejects_a_replaced_recording_before_discovering_its_media() {
    use crate::{
        profiles::MonitoringProfile,
        session::{OperatorAction, SessionCamera, SessionController, mark_recording_complete},
    };
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let selected = fixture.snapshot();
    std::fs::rename(
        fixture.store.session_directory.join("events.jsonl"),
        fixture
            .store
            .session_directory
            .join("original-events.jsonl"),
    )
    .unwrap();
    let mut replacement = SessionController::create(
        fixture.store.session_directory.join("events.jsonl"),
        vec![SessionCamera {
            id: 1,
            name: "Replacement camera".into(),
            enabled: true,
            initial_monitoring_profile_id: 1,
        }],
        vec![MonitoringProfile {
            id: 1,
            name: "Standard".into(),
            sample_every_ms: 1000,
        }],
    )
    .unwrap();
    replacement.apply(OperatorAction::EndSession).unwrap();
    mark_recording_complete(&fixture.store.session_directory).unwrap();
    let exercises = fixture
        .store
        .session_directory
        .parent()
        .unwrap()
        .join("exercises");

    let error = create_run(CreateRun {
        session_directory: fixture.store.session_directory.clone(),
        expected_session_id: selected.inputs.initial_evidence.session_id,
        exercise_store: crate::exercise::ExerciseStore::new(exercises),
        revision: selected.inputs.exercise,
        model: selected.inputs.model,
        execution: selected.inputs.execution,
        evidence_policy: selected.inputs.evidence_policy,
    })
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("selected recording changed"),
        "{error}"
    );
    assert!(!fixture.store.session_directory.join("recordings").exists());
}

#[tokio::test]
async fn resumed_analysis_rejects_a_replaced_session_before_model_requests() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let checkpoint = fixture
        .store
        .directory(fixture.run_id)
        .unwrap()
        .join("checkpoint.json");
    let original = std::fs::read(&checkpoint).unwrap();
    fixture.replace_session_identity();

    let error = execute_run(fixture.request(), |_| {}).await.unwrap_err();

    assert!(error.to_string().contains("another recording"), "{error}");
    assert!(server.requests.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(checkpoint).unwrap(), original);
}

#[tokio::test]
async fn recheck_rejects_a_replaced_session_without_changing_its_checkpoint() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let ready = execute_run(fixture.request(), |_| {}).await.unwrap();
    server.requests.lock().unwrap().clear();
    let checkpoint = fixture
        .store
        .directory(fixture.run_id)
        .unwrap()
        .join("checkpoint.json");
    let original = std::fs::read(&checkpoint).unwrap();
    fixture.replace_session_identity();

    let error = request_recheck(
        fixture.store.session_directory.clone(),
        fixture.run_id,
        ready.inputs.exercise.checklist.criteria[0].id,
        None,
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("another recording"), "{error}");
    assert!(server.requests.lock().unwrap().is_empty());
    assert_eq!(std::fs::read(checkpoint).unwrap(), original);
}

fn completion<T>(value: T) -> Completion<T> {
    Completion {
        value,
        usage: None,
        elapsed_ms: 1,
    }
}

#[tokio::test]
async fn cached_images_cannot_send_inconsistent_timeline_labels_to_a_provider() {
    let server = ModelServer::new(false);
    let fixture = Fixture::new(&server.base_url);
    let mut snapshot = fixture.snapshot();
    snapshot.checkpoint.evidence.frames[0].session_offset_ms = 1000;
    fixture
        .store
        .save_checkpoint(fixture.run_id, &snapshot.checkpoint)
        .unwrap();
    // The bytes and cache identity are valid; only their claimed session timestamp is wrong.
    fixture.cache_frames();

    assert!(execute_run(fixture.request(), |_| {}).await.is_err());
    assert!(server.requests.lock().unwrap().is_empty());
    let paused = fixture.snapshot();
    assert_eq!(paused.checkpoint.stage, RunStage::Paused);
    assert!(paused.checkpoint.assessments.is_empty());
}
fn finding(snapshot: &RunSnapshot, judgment: Judgment) -> Finding {
    Finding {
        criterion_id: snapshot.inputs.exercise.checklist.criteria[0]
            .id
            .to_string(),
        judgment,
        reason: "Visible action".into(),
        evidence_ids: vec!["C1-T0".into()],
        configuration_issue: None,
        conflicting_evidence: false,
    }
}
fn scan_response(snapshot: &RunSnapshot) -> SavedResponse {
    let job = &snapshot.checkpoint.jobs[0];
    SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: job
            .frame_ids
            .iter()
            .map(|id| SentImage {
                candidate_id: id.clone(),
                sha256: "a".repeat(64),
                size_bytes: 1,
            })
            .collect(),
        reference_hashes: Vec::new(),
        output: JobOutput::Scan(completion(ScanFinding {
            observations: vec![ScanObservation {
                criterion_id: snapshot.inputs.exercise.checklist.criteria[0]
                    .id
                    .to_string(),
                description: "A cup is visible.".into(),
                evidence_ids: vec!["C1-T0".into()],
            }],
        })),
    }
}

#[test]
fn scan_rejects_unknown_criteria_and_reference_images_as_candidate_citations() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let snapshot = fixture.snapshot();
    for (criterion, evidence) in [
        (uuid::Uuid::new_v4().to_string(), "C1-T0"),
        (
            snapshot.inputs.exercise.checklist.criteria[0]
                .id
                .to_string(),
            "R-reference",
        ),
    ] {
        let mut response = scan_response(&snapshot);
        let JobOutput::Scan(output) = &mut response.output else {
            unreachable!()
        };
        output.value.observations[0].criterion_id = criterion;
        output.value.observations[0].evidence_ids = vec![evidence.into()];
        assert!(
            results::validate_response(&snapshot, &snapshot.checkpoint.jobs[0], &response).is_err()
        );
    }
}

#[test]
fn consolidation_cannot_borrow_another_criterions_citations_or_omit_a_criterion() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    snapshot.responses.push(scan_response(&snapshot));
    let job = jobs::record(
        &snapshot.inputs,
        JobKind::Consolidate,
        None,
        vec!["C1-T0".into()],
    )
    .unwrap();
    let first = finding(&snapshot, Judgment::Met);
    let response = |findings| SavedResponse {
        job_id: job.id.clone(),
        frame_ids: job.frame_ids.clone(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output: JobOutput::Consolidate(completion(Consolidation { findings })),
    };
    assert!(results::validate_response(&snapshot, &job, &response(vec![first.clone()])).is_err());
    let mut second = first.clone();
    second.criterion_id = snapshot.inputs.exercise.checklist.criteria[1]
        .id
        .to_string();
    assert!(
        results::validate_response(
            &snapshot,
            &job,
            &response(vec![first.clone(), second.clone()])
        )
        .is_err()
    );
    second.judgment = Judgment::CannotVerify;
    second.evidence_ids.clear();
    assert!(results::validate_response(&snapshot, &job, &response(vec![first, second])).is_ok());
}

#[test]
fn policy_uncertainty_and_conflicting_evidence_cannot_be_published_as_a_pass() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let snapshot = fixture.snapshot();
    let id = snapshot.inputs.exercise.checklist.criteria[0].id;
    let job = jobs::record(
        &snapshot.inputs,
        JobKind::Recheck,
        Some(id),
        vec!["C1-T0".into()],
    )
    .unwrap();
    for policy in [true, false] {
        let mut value = finding(&snapshot, Judgment::Met);
        if policy {
            value.configuration_issue = Some("Is a retry allowed?".into());
        } else {
            value.conflicting_evidence = true;
        }
        let response = SavedResponse {
            job_id: job.id.clone(),
            frame_ids: job.frame_ids.clone(),
            sent_images: vec![SentImage {
                candidate_id: "C1-T0".into(),
                sha256: "a".repeat(64),
                size_bytes: 1,
            }],
            reference_hashes: Vec::new(),
            output: JobOutput::Recheck(completion(value)),
        };
        assert!(results::validate_response(&snapshot, &job, &response).is_err());
    }
}

#[test]
fn contradictory_closer_look_preserves_both_findings_and_requires_review() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    let first = finding(&snapshot, Judgment::NotMet);
    let second = finding(&snapshot, Judgment::Met);
    let response = |output| SavedResponse {
        job_id: String::new(),
        frame_ids: Vec::new(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output,
    };
    snapshot.responses = vec![
        response(JobOutput::Consolidate(completion(Consolidation {
            findings: vec![first.clone()],
        }))),
        response(JobOutput::Recheck(completion(second.clone()))),
    ];
    results::assess(&mut snapshot);
    assert_eq!(
        snapshot.checkpoint.assessments[0].judgment,
        Judgment::CannotVerify
    );
    assert_eq!(snapshot.checkpoint.assessments[0].initial, first);
    assert_eq!(
        snapshot.checkpoint.assessments[0].recheck.as_ref(),
        Some(&second)
    );
}

#[test]
fn a_policy_issue_found_by_the_closer_look_remains_visible_in_the_combined_result() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    let initial = finding(&snapshot, Judgment::Met);
    let mut recheck = finding(&snapshot, Judgment::CannotVerify);
    recheck.configuration_issue = Some("The required placement order is unspecified".into());
    let response = |output| SavedResponse {
        job_id: String::new(),
        frame_ids: Vec::new(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output,
    };
    snapshot.responses = vec![
        response(JobOutput::Consolidate(completion(Consolidation {
            findings: vec![initial],
        }))),
        response(JobOutput::Recheck(completion(recheck.clone()))),
    ];
    results::assess(&mut snapshot);
    assert_eq!(
        snapshot.checkpoint.assessments[0].judgment,
        Judgment::CannotVerify
    );
    assert_eq!(
        snapshot.checkpoint.assessments[0].configuration_issue,
        recheck.configuration_issue
    );
}

#[test]
fn distant_evidence_is_not_dropped_to_force_a_bounded_closer_look() {
    let fixture = Fixture::new("http://127.0.0.1:1/v1");
    let mut snapshot = fixture.snapshot();
    let mut initial = finding(&snapshot, Judgment::CannotVerify);
    initial.evidence_ids.push("C1-T2000".into());
    snapshot.responses.push(SavedResponse {
        job_id: String::new(),
        frame_ids: Vec::new(),
        sent_images: Vec::new(),
        reference_hashes: Vec::new(),
        output: JobOutput::Consolidate(completion(Consolidation {
            findings: vec![initial],
        })),
    });
    results::assess(&mut snapshot);
    snapshot.checkpoint.evidence.frames[2].session_offset_ms = 90_000;
    snapshot.checkpoint.evidence.duration_ms = 100_000;
    let before = snapshot.checkpoint.clone();
    let id = snapshot.inputs.exercise.checklist.criteria[0].id;
    assert!(super::super::recheck::queue(&mut snapshot, id, None).is_err());
    assert_eq!(
        snapshot.checkpoint, before,
        "failed reservation cannot mutate evidence or spend a request"
    );
}
