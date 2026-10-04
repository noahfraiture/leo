use super::labels::{judgment_class, judgment_label};
use crate::analysis::AnalysisContext;
use backend::{
    analysis::{CriterionAssessment, HumanCorrection, Judgment, RunSnapshot},
    exercise::Criterion,
};
use dioxus::prelude::*;
use uuid::Uuid;

/// One row for every operator criterion, with original and corrected judgments kept distinct.
#[component]
pub fn AssessmentList(session_id: Uuid, run_id: Uuid, snapshot: RunSnapshot) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut filter = use_signal(|| ResultFilter::All);
    let selected = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .and_then(|entry| entry.selected_criterion);
    let (mut met, mut not_met, mut cannot_verify, mut not_assessed) = (0, 0, 0, 0);
    for criterion in &snapshot.inputs.exercise.checklist.criteria {
        let judgment = snapshot
            .checkpoint
            .assessments
            .iter()
            .find(|assessment| assessment.criterion_id == criterion.id)
            .map(|assessment| assessment.judgment);
        match judgment {
            Some(Judgment::Met) => met += 1,
            Some(Judgment::NotMet) => not_met += 1,
            Some(Judgment::CannotVerify) => cannot_verify += 1,
            None => not_assessed += 1,
        }
    }
    rsx! {
        section { aria_label: "Checklist assessments",
            h3 { class: "font-semibold", "Checklist results" }
            div {
                class: "mt-2 flex flex-wrap items-center gap-2 text-sm",
                aria_label: "Model result counts",
                span { "Model results:" }
                span { class: judgment_class(Judgment::Met), "Met: {met}" }
                span { class: judgment_class(Judgment::NotMet), "Not met: {not_met}" }
                span { class: judgment_class(Judgment::CannotVerify), "Cannot verify: {cannot_verify}" }
                span { class: "badge badge-outline", "Not assessed: {not_assessed}" }
            }
            p { class: "mt-1 text-sm opacity-70",
                "Select a step to inspect source footage, the initial result, and any closer look."
            }
            div { class: "tabs tabs-boxed mt-3", aria_label: "Filter checklist results",
                for (value, label) in [(ResultFilter::All, "All"), (ResultFilter::NeedsAttention, "Needs attention"), (ResultFilter::Resolved, "Resolved uncertainty")] {
                    button { class: if filter() == value { "tab tab-active" } else { "tab" }, onclick: move |_| filter.set(value), "{label}" }
                }
            }
            div { class: "mt-3 flex flex-col gap-2",
                for (index, criterion) in snapshot.inputs.exercise.checklist.criteria.iter().enumerate().filter(|(_, item)| filter().includes(&snapshot, item.id)) {
                    AssessmentRow {
                        key: "{criterion.id}",
                        session_id,
                        run_id,
                        criterion: criterion.clone(),
                        index,
                        selected: selected == Some(criterion.id),
                        assessment: snapshot
                            .checkpoint
                            .assessments
                            .iter()
                            .find(|item| item.criterion_id == criterion.id)
                            .cloned(),
                        correction: snapshot
                            .corrections
                            .iter()
                            .rev()
                            .find(|item| item.criterion_id == criterion.id)
                            .cloned(),
                        reviewed: snapshot.is_reviewed(criterion.id),
                        resolved: ResultFilter::Resolved.includes(&snapshot, criterion.id),
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
    reviewed: bool,
    resolved: bool,
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
            if resolved { span { class: "badge badge-info mt-2", "Uncertainty resolved; initial finding retained" } }
            if reviewed { span { class: "badge badge-outline mt-2", "Reviewed by you" } }
        }
    }
}

/// A result can resolve uncertainty while still needing attention because a requirement was not met.
#[derive(Clone, Copy, PartialEq)]
pub enum ResultFilter {
    All,
    NeedsAttention,
    Resolved,
}
impl ResultFilter {
    pub fn includes(self, snapshot: &RunSnapshot, criterion_id: Uuid) -> bool {
        let assessment = snapshot
            .checkpoint
            .assessments
            .iter()
            .find(|item| item.criterion_id == criterion_id);
        let effective = snapshot
            .corrections
            .iter()
            .rev()
            .find(|item| item.criterion_id == criterion_id)
            .map(|item| item.judgment)
            .or_else(|| assessment.map(|item| item.judgment));
        match self {
            Self::All => true,
            Self::NeedsAttention => effective != Some(Judgment::Met),
            Self::Resolved => {
                assessment.is_some_and(|item| item.initial.judgment == Judgment::CannotVerify)
                    && effective.is_some_and(|judgment| judgment != Judgment::CannotVerify)
            }
        }
    }
}
