//! Human assessment saves do not replace model progress or other recording selections.
use super::AnalysisContext;
use backend::analysis::{OperatorReview, RunStore};
use dioxus::prelude::*;
use uuid::Uuid;

pub fn mark_reviewed(context: AnalysisContext, session_id: Uuid, run_id: Uuid, criterion_id: Uuid) {
    let basis = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .and_then(|entry| entry.snapshot.criterion_basis(criterion_id).ok());
    let Some(basis) = basis else {
        return;
    };
    save(context, session_id, run_id, move |store| {
        store
            .mark_reviewed(run_id, criterion_id, &basis)
            .map(|snapshot| snapshot.operator_review)
    });
}

pub fn save_assessment(context: AnalysisContext, session_id: Uuid, run_id: Uuid) {
    let submitted = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .and_then(|entry| {
            entry
                .snapshot
                .assessment_basis()
                .ok()
                .map(|basis| (basis, entry.assessment_draft.clone()))
        });
    let Some((basis, draft)) = submitted else {
        return;
    };
    save(context, session_id, run_id, move |store| {
        store
            .save_student_assessment(run_id, draft.outcome, draft.note, &basis)
            .map(|snapshot| snapshot.operator_review)
    });
}

fn save(
    mut context: AnalysisContext,
    session_id: Uuid,
    run_id: Uuid,
    operation: impl FnOnce(RunStore) -> backend::analysis::RunResult<OperatorReview> + Send + 'static,
) {
    let directory = {
        let mut page = context.state.write();
        let Some(session) = page.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(entry) = session.runs.get_mut(&run_id) else {
            return;
        };
        if entry.assessment_draft.saving {
            return;
        }
        entry.assessment_draft.saving = true;
        entry.assessment_draft.error = None;
        session.directory.clone()
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = tokio::task::spawn_blocking(move || operation(RunStore::new(directory)))
            .await
            .map_err(|_| "The operator review could not be saved.".to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
        let mut page = context.state.write();
        if let Some(entry) = page
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.runs.get_mut(&run_id))
        {
            entry.assessment_draft.saving = false;
            match result {
                Ok(review) => {
                    entry.snapshot.operator_review = review;
                    entry.snapshot_generation = entry.snapshot_generation.wrapping_add(1);
                }
                Err(error) => entry.assessment_draft.error = Some(error),
            }
        }
    });
}
