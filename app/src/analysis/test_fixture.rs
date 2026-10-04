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
        id: "reader-job".into(),
        criterion_id,
        profile_id: model.profile.id.clone(),
        kind: JobKind::Reader,
        round: 0,
        frame_ids: vec!["candidate-1".into()],
        context_job_ids: Vec::new(),
        state: JobState::Complete,
    }];
    let assessments = if stage == RunStage::Ready {
        vec![CriterionAssessment {
            criterion_id,
            judgment: Judgment::CannotVerify,
            reason: "The closing action is obscured.".into(),
            evidence_ids: vec!["candidate-1".into()],
            unresolved_facts: vec!["Whether the item was fully inside".into()],
            reader_job_ids: vec!["reader-job".into()],
            review_job_ids: Vec::new(),
            selection_job_id: "selection-job".into(),
            final_job_id: Some("final-job".into()),
            additional_evidence_round_used: true,
            coverage_note: "No view reveals the hidden state.".into(),
        }]
    } else {
        Vec::new()
    };
    RunSnapshot {
        inputs: RunInputs {
            schema_version: 1,
            id: run_id,
            created_utc_ms: 1_786_552_805_000,
            prompt_version: "fixture-v1".into(),
            exercise,
            model_set: ResolvedModelSet {
                id: "fixture-set".into(),
                name: "Fixture models".into(),
                readers: vec![model.clone()],
                reviewer: model.clone(),
                formatter: model,
            },
            execution: ExecutionPolicy::default(),
            evidence_policy: EvidencePolicy::default(),
            initial_evidence: evidence.clone(),
            windows: Vec::new(),
        },
        checkpoint: RunCheckpoint {
            stage,
            jobs,
            evidence,
            gathered_criteria: Vec::new(),
            coverage_notes: Vec::new(),
            assessments,
            error: None,
        },
        reader_results: vec![ReaderResult {
            job_id: "reader-job".into(),
            completion: Completion {
                value: ReaderFinding {
                    judgment: Judgment::CannotVerify,
                    reason: "The item is partly obscured.".into(),
                    evidence_ids: vec!["candidate-1".into()],
                    facts: vec![VisualFact {
                        description: "A hand covers the case opening.".into(),
                        evidence_ids: vec!["candidate-1".into()],
                        attempt: None,
                        identity: None,
                    }],
                },
                usage: None,
                elapsed_ms: 1,
            },
        }],
        review_results: Vec::new(),
        selection_results: Vec::new(),
        final_results: Vec::new(),
        corrections: Vec::new(),
    }
}
