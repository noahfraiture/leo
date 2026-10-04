use super::labels::judgment_label;
use crate::analysis::{self, AnalysisContext};
use backend::analysis::{RunStage, StudentOutcome};
use dioxus::prelude::*;
use uuid::Uuid;

/// Complete checklist and an explicitly human overall decision, bound to reviewed findings.
#[component]
pub fn StudentReview(session_id: Uuid, run_id: Uuid) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut state = context.state;
    let entry = state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .cloned();
    let Some(entry) = entry else {
        return rsx! {};
    };
    let snapshot = entry.snapshot;
    let reviewed = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .filter(|item| snapshot.is_reviewed(item.id))
        .count();
    let count = snapshot.inputs.exercise.checklist.criteria.len();
    let draft = entry.assessment_draft;
    let disabled = snapshot.checkpoint.stage != RunStage::Ready
        || reviewed != count
        || draft.saving
        || draft.note.trim().is_empty();
    let current = snapshot.current_student_assessment();
    rsx! {
        section { class: "flex flex-col gap-3 border-t border-base-300 pt-4", aria_label: "Student assessment",
            h2 { class: "text-lg font-semibold", "Student assessment" }
            p { class: "text-sm", "You make the overall decision. {reviewed} of {count} criteria reviewed." }
            if let Some(saved) = current {
                div { class: "alert alert-success",
                    p { "Saved operator assessment: {outcome_label(saved.outcome)}. {saved.note}" }
                }
            } else if !snapshot.operator_review.assessments.is_empty() {
                p { class: "alert alert-warning", "Results changed since the last assessment. Review the changed criteria and save a new decision." }
            }
            div { class: "overflow-x-auto",
                table { class: "table table-sm",
                    thead { tr { th { "Complete checklist" } th { "Current result" } th { "Review" } } }
                    tbody {
                        for criterion in &snapshot.inputs.exercise.checklist.criteria {
                            tr {
                                td { "{criterion.text}" }
                                td {
                                    if let Some(correction) = snapshot.corrections.iter().rev().find(|item| item.criterion_id == criterion.id) {
                                        "Operator: {judgment_label(correction.judgment)}"
                                    } else if let Some(assessment) = snapshot.checkpoint.assessments.iter().find(|item| item.criterion_id == criterion.id) {
                                        "Model: {judgment_label(assessment.judgment)}"
                                    } else { "Not assessed" }
                                }
                                td { if snapshot.is_reviewed(criterion.id) { "Reviewed" } else { "To review" } }
                            }
                        }
                    }
                }
            }
            label { class: "form-control",
                span { class: "label-text mb-1", "Your overall decision" }
                select { id: "student-outcome", class: "select select-bordered", disabled: draft.saving,
                    onchange: move |event| {
                        if let Some(entry) = state.write().sessions.get_mut(&session_id).and_then(|session| session.runs.get_mut(&run_id)) {
                            entry.assessment_draft.outcome = match event.value().as_str() { "pass" => StudentOutcome::Pass, "practice" => StudentOutcome::NeedsPractice, _ => StudentOutcome::NeedsEvidence };
                        }
                    },
                    option { value: "evidence", selected: draft.outcome == StudentOutcome::NeedsEvidence, "Need more evidence" }
                    option { value: "practice", selected: draft.outcome == StudentOutcome::NeedsPractice, "Needs further practice" }
                    option { value: "pass", selected: draft.outcome == StudentOutcome::Pass, "Pass" }
                }
            }
            label { class: "form-control",
                span { class: "label-text mb-1", "Assessment note" }
                textarea { id: "student-note", class: "textarea textarea-bordered", rows: 3, value: draft.note, disabled: draft.saving,
                    oninput: move |event| {
                        if let Some(entry) = state.write().sessions.get_mut(&session_id).and_then(|session| session.runs.get_mut(&run_id)) { entry.assessment_draft.note = event.value(); }
                    },
                }
            }
            if let Some(error) = draft.error { p { class: "alert alert-error", role: "alert", "{error}" } }
            button { id: "student-save", class: "btn btn-primary self-start", disabled,
                onclick: move |_| analysis::save_assessment(context, session_id, run_id),
                "Save student assessment"
            }
            if current.is_some() { p { class: "text-sm", "For the next student, select another recording. The confirmed exercise is reusable; review marks, corrections and the overall assessment belong only to this run." } }
            if !snapshot.operator_review.assessments.is_empty() {
                details { class: "text-sm",
                    summary { "Operator assessment history" }
                    for saved in &snapshot.operator_review.assessments { p { class: "mt-2", "{outcome_label(saved.outcome)}: {saved.note}" } }
                }
            }
        }
    }
}
fn outcome_label(outcome: StudentOutcome) -> &'static str {
    match outcome {
        StudentOutcome::Pass => "Pass",
        StudentOutcome::NeedsPractice => "Needs further practice",
        StudentOutcome::NeedsEvidence => "Need more evidence",
    }
}
