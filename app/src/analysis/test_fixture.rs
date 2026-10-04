use backend::{
    analysis::*,
    exercise::{Checklist, Criterion, ExerciseRevision},
    model::*,
};
use uuid::Uuid;

/// Small deterministic saved run shared by state and rendering tests; no model or media I/O.
pub fn snapshot(session_id: Uuid, run_id: Uuid, stage: RunStage) -> RunSnapshot {
    let criterion_id = Uuid::from_u128(71);
    let provider = ProviderConnection {
        id: "fixture-provider".into(),
        name: "Fixture service".into(),
        base_url: "http://127.0.0.1:1234/v1".into(),
        transport: Transport::ChatCompletions,
        api_key: "fixture".into(),
    };
    let model = ModelProfile {
        id: "reader-one".into(),
        name: "Reader One".into(),
        provider_id: provider.id.clone(),
        model: "fixture-model".into(),
        reasoning_effort: None,
        memory_cost_mib: 0,
        image_limit: 8,
        max_output_tokens: Some(6000),
    };
    let model = model.resolve(&[provider]).unwrap();
    let evidence = EvidenceManifest {
        session_id,
        session_start_utc_ms: 1_786_552_800_000,
        duration_ms: 4000,
        sources: Vec::new(),
        frames: vec![CandidateFrame {
            id: "candidate-1".into(),
            camera_id: 17,
            source_index: 0,
            session_offset_ms: 1000,
            recording_offset_ms: 1000,
        }],
        frame_sets: vec![vec!["candidate-1".into()]],
        warnings: Vec::new(),
        eligible_periods: Vec::new(),
    };
    let exercise = ExerciseRevision {
        schema_version: 1,
        id: Uuid::from_u128(72),
        exercise_id: Uuid::from_u128(73),
        edit_version: 1,
        name: "Case exercise".into(),
        checklist: Checklist {
            format_version: "operator-text-v1".into(),
            criteria: vec![Criterion {
                id: criterion_id,
                text: "Place the item inside the case.".into(),
            }],
        },
        references: Vec::new(),
        preparation: Default::default(),
    };
    let jobs = vec![JobRecord {
        id: "scan-job".into(),
        criterion_id: None,
        kind: JobKind::Scan,
        frame_ids: vec!["candidate-1".into()],
        state: JobState::Complete,
        failures: Vec::new(),
    }];
    let finding = Finding {
        criterion_id: criterion_id.to_string(),
        judgment: Judgment::CannotVerify,
        reason: "The closing action is obscured.".into(),
        evidence_ids: vec!["candidate-1".into()],
        configuration_issue: None,
        conflicting_evidence: false,
    };
    let assessments = if stage == RunStage::Ready {
        vec![CriterionAssessment {
            criterion_id,
            judgment: finding.judgment,
            reason: finding.reason.clone(),
            evidence_ids: finding.evidence_ids.clone(),
            configuration_issue: None,
            initial: finding,
            recheck: None,
            recheck_note: None,
        }]
    } else {
        Vec::new()
    };
    RunSnapshot {
        inputs: RunInputs {
            schema_version: 2,
            id: run_id,
            created_utc_ms: 1_786_552_805_000,
            prompt_version: "fixture-v2".into(),
            exercise,
            model,
            execution: ExecutionPolicy::default(),
            evidence_policy: EvidencePolicy::default(),
            initial_evidence: evidence.clone(),
            windows: Vec::new(),
        },
        checkpoint: RunCheckpoint {
            stage,
            jobs,
            evidence,
            assessments,
            error: None,
        },
        responses: vec![SavedResponse {
            job_id: "scan-job".into(),
            frame_ids: vec!["candidate-1".into()],
            sent_images: Vec::new(),
            reference_hashes: Vec::new(),
            output: JobOutput::Scan(Completion {
                value: ScanFinding {
                    observations: vec![ScanObservation {
                        criterion_id: criterion_id.to_string(),
                        description: "A hand covers the case opening.".into(),
                        evidence_ids: vec!["candidate-1".into()],
                    }],
                },
                usage: None,
                elapsed_ms: 1,
            }),
        }],
        corrections: Vec::new(),
        operator_review: OperatorReview::default(),
    }
}
