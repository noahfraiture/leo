use super::super::{jobs, results};
use super::*;
use crate::analysis::SentImage;
use crate::model::Completion;

fn completion<T>(value: T) -> Completion<T> {
    Completion {
        value,
        usage: None,
        elapsed_ms: 1,
    }
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
