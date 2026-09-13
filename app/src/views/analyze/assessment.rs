use super::labels::{judgment_class, judgment_label};
use crate::analysis::AnalysisContext;
use backend::{
    analysis::{CriterionAssessment, HumanCorrection, JobKind, RunSnapshot},
    exercise::Criterion,
};
use dioxus::prelude::*;
use uuid::Uuid;

/// One row for every operator criterion, with original and corrected judgments kept distinct.
#[component]
pub fn AssessmentList(session_id: Uuid, run_id: Uuid, snapshot: RunSnapshot) -> Element {
    let context = use_context::<AnalysisContext>();
    let selected = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .and_then(|entry| entry.selected_criterion);
    rsx! {
        section { aria_label: "Checklist assessments",
            h3 { class: "font-semibold", "Checklist results" }
            p { class: "mt-1 text-sm opacity-70",
                "Select a step to inspect source footage, independent findings, and the reviewer reasoning."
            }
            div { class: "mt-3 flex flex-col gap-2",
                for (index, criterion) in snapshot.inputs.exercise.checklist.criteria.iter().enumerate() {
                    AssessmentRow {
                        key: "{criterion.id}",
                        session_id,
                        run_id,
                        criterion: criterion.clone(),
                        index,
                        selected: selected == Some(criterion.id),
                        assessment: snapshot.checkpoint.assessments.iter().find(|item| item.criterion_id == criterion.id).cloned(),
                        correction: snapshot.corrections.iter().rev().find(|item| item.criterion_id == criterion.id).cloned(),
                        disagreement: reader_disagreement(&snapshot, criterion.id),
                    }
                }
            }
        }
    }
}

#[component]
fn AssessmentRow(
    session_id: Uuid,
    run_id: Uuid,
    criterion: Criterion,
    index: usize,
    selected: bool,
    assessment: Option<CriterionAssessment>,
    correction: Option<HumanCorrection>,
    disagreement: bool,
) -> Element {
    let AnalysisContext { mut state } = use_context::<AnalysisContext>();
    let id = criterion.id;
    rsx! {
        button {
            class: if selected { "rounded-box border border-primary bg-base-200 p-3 text-left" } else { "rounded-box border border-base-300 p-3 text-left" },
            aria_pressed: selected,
            "data-assessment-criterion": "{id}",
            onclick: move |_| {
                if let Some(entry) = state
                    .write()
                    .sessions
                    .get_mut(&session_id)
                    .and_then(|session| session.runs.get_mut(&run_id))
                {
                    if entry.selected_criterion != Some(id) {
                        entry.playback = Default::default();
                    }
                    entry.selected_criterion = Some(id);
                }
            },
            div { class: "flex flex-wrap items-start justify-between gap-2",
                p { class: "min-w-0 flex-1 text-sm font-medium", "{index + 1}. {criterion.text}" }
                if let Some(correction) = &correction {
                    span { class: judgment_class(correction.judgment),
                        "Operator: {judgment_label(correction.judgment)}"
                    }
                }
                if let Some(assessment) = &assessment {
                    span { class: judgment_class(assessment.judgment),
                        "Model: {judgment_label(assessment.judgment)}"
                    }
                } else {
                    span { class: "badge badge-outline", "Not assessed yet" }
                }
            }
            if let Some(assessment) = assessment {
                p { class: "mt-2 text-sm", "{assessment.reason}" }
            }
            if disagreement {
                p { class: "mt-2 text-sm text-warning", "Reader disagreement on matching evidence" }
            }
        }
    }
}

/// Compare independent readers only when they received the same criterion and candidate frames.
pub fn reader_disagreement(snapshot: &RunSnapshot, criterion_id: Uuid) -> bool {
    let findings = snapshot
        .reader_results
        .iter()
        .filter_map(|result| {
            snapshot
                .checkpoint
                .jobs
                .iter()
                .find(|job| {
                    job.id == result.job_id
                        && job.kind == JobKind::Reader
                        && job.criterion_id == criterion_id
                })
                .map(|job| (job, result.completion.value.judgment))
        })
        .collect::<Vec<_>>();
    findings
        .iter()
        .enumerate()
        .any(|(index, (left, left_judgment))| {
            findings[index + 1..].iter().any(|(right, right_judgment)| {
                left.profile_id != right.profile_id
                    && left.frame_ids == right.frame_ids
                    && left_judgment != right_judgment
            })
        })
}
