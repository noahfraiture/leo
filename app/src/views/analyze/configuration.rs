use backend::exercise::{ChecklistOrder, ExerciseRevision};
use dioxus::prelude::*;

/// The operator policy used by this run, independent of later edits to the reusable exercise.
#[component]
pub fn FrozenExercise(exercise: ExerciseRevision) -> Element {
    let policy = match exercise.preparation.order {
        ChecklistOrder::Unspecified => {
            "Not specified; flag ordering ambiguity when it affects a decision"
        }
        ChecklistOrder::ListedOrder => "Every step must follow the listed order",
        ChecklistOrder::ExplicitOnly => "Only order explicitly required by a step",
    };
    rsx! {
        details { class: "rounded-box border border-base-300 p-3 text-sm",
            summary { class: "font-semibold", "Exercise saved with this run" }
            p { class: "mt-3", "Context: {exercise.preparation.scene_context}" }
            p { class: "mt-2", "Order: {policy}" }
            h4 { class: "mt-3 font-semibold", "Operator clarification answers" }
            for question in exercise.preparation.questions.iter().filter(|question| exercise.checklist.criteria.iter().any(|item| item.id == question.criterion_id)) {
                div { class: "mt-2",
                    p { "{question.question}" }
                    if question.answer.trim().is_empty() { p { class: "text-warning", "Pending; no operator standard supplied" } }
                    else { p { "{question.answer}" } }
                }
            }
            if let Some(source) = &exercise.preparation.source {
                details { class: "mt-3",
                    summary { "Original wording before preparation" }
                    ol { class: "mt-2 list-decimal pl-5", for criterion in &source.criteria { li { "{criterion.text}" } } }
                }
            }
        }
    }
}
