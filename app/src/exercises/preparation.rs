//! Small operator-owned preparation steps. No candidate recording is used to define policy.
use backend::exercise::ChecklistOrder;
use dioxus::prelude::*;
use uuid::Uuid;

use super::{checklist::ChecklistEditor, state::ExerciseContext};

#[component]
pub fn ContextEditor(exercise_id: Uuid) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let preparation = state.read().configurations[&exercise_id]
        .draft
        .preparation
        .clone();
    rsx! {
        label { class: "form-control",
            span { class: "label-text mb-1", "Scene context" }
            textarea { id: "exercise-context", class: "textarea textarea-bordered", rows: 3,
                placeholder: "Equipment, location and names the evaluator should understand.",
                // Input events persist the draft without replaying older text over native typing.
                initial_value: preparation.scene_context,
                oninput: move |event| state.write().edit(exercise_id, |draft| draft.preparation.scene_context = event.value()),
            }
        }
        label { class: "form-control",
            span { class: "label-text mb-1", "Does checklist order matter?" }
            select { class: "select select-bordered", id: "exercise-order",
                onchange: move |event| state.write().edit(exercise_id, |draft| {
                    draft.preparation.order = match event.value().as_str() {
                        "listed" => ChecklistOrder::ListedOrder,
                        "explicit" => ChecklistOrder::ExplicitOnly,
                        _ => ChecklistOrder::Unspecified,
                    };
                }),
                option { value: "unspecified", selected: preparation.order == ChecklistOrder::Unspecified, "Not specified; flag it if it affects the result" }
                option { value: "listed", selected: preparation.order == ChecklistOrder::ListedOrder, "Every step must follow the listed order" }
                option { value: "explicit", selected: preparation.order == ChecklistOrder::ExplicitOnly, "Only order explicitly required by a step" }
            }
        }
    }
}

#[component]
pub fn OriginalChecklist(exercise_id: Uuid) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let draft = state.read().configurations[&exercise_id].draft.clone();
    let source = draft.preparation.source.clone();
    let new_steps = source
        .as_ref()
        .map(|source| {
            draft
                .checklist
                .criteria
                .iter()
                .filter(|item| {
                    !source
                        .criteria
                        .iter()
                        .any(|original| original.id == item.id)
                })
                .count()
        })
        .unwrap_or(0);
    rsx! {
        h3 { class: "font-semibold", "Original checklist" }
        if new_steps > 0 {
            p { class: "text-sm text-warning", "{new_steps} steps were added to the final checklist after preparation. Include them here if you want wording assistance for them." }
            button { class: "btn btn-outline btn-sm self-start", onclick: move |_| {
                if let Some(editor) = state.write().configurations.get_mut(&exercise_id) { editor.include_new_source_steps(); }
            }, "Include added steps in preparation" }
        }
        if let Some(source) = source {
            p { class: "text-sm opacity-70", "Source preserved before assistance. Editing it leaves your final checklist unchanged; review the final wording before confirming." }
            for (index, criterion) in source.criteria.iter().enumerate() {
                textarea { key: "{criterion.id}", class: "textarea textarea-bordered w-full", rows: 2,
                    aria_label: "Original step {index + 1}", initial_value: criterion.text.clone(),
                    oninput: move |event| state.write().edit(exercise_id, |draft| {
                        if let Some(source) = &mut draft.preparation.source { source.criteria[index].text = event.value(); }
                    }),
                }
            }
        } else {
            ChecklistEditor { exercise_id }
        }
    }
}

#[component]
pub fn ClarificationAnswers(exercise_id: Uuid) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let Some((draft, question_ids)) = state
        .read()
        .configurations
        .get(&exercise_id)
        .map(|editor| (editor.draft.clone(), editor.question_ids.clone()))
    else {
        return rsx! {};
    };
    rsx! {
        section { class: "flex flex-col gap-3", aria_label: "Operator clarification answers",
            h3 { class: "font-semibold", "Questions for you" }
            p { class: "text-sm opacity-70", "An unanswered question remains pending. Leo must flag affected decisions, not invent a standard." }
            if draft.preparation.questions.is_empty() {
                p { class: "text-sm", "No questions saved. You can review and confirm the checklist without AI assistance." }
            }
            for (question_id, question) in question_ids.into_iter().zip(draft.preparation.questions.clone()) {
                label { key: "{question_id}", class: "form-control rounded-box border border-base-300 p-3",
                    span { class: "text-xs opacity-70",
                        "Original wording: {question.source_text}"
                    }
                    span { class: "label-text my-2 font-semibold", "{question.question}" }
                    if draft.source_checklist().criteria.iter().any(|item| item.id == question.criterion_id && item.text != question.source_text) {
                        p { class: "text-warning text-sm", "The source changed. This answer is not used until you confirm it still applies." }
                        button { class: "btn btn-outline btn-sm", onclick: move |_| {
                            if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                                editor.confirm_question_source(question_id);
                            }
                        }, "Answer still applies to the new wording" }

                    }
                    button { class: "btn btn-ghost btn-sm self-start", onclick: move |_| {
                        if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                            editor.remove_question(question_id);
                        }
                    }, "Remove question" }
                    textarea { class: "textarea textarea-bordered", rows: 2,
                        initial_value: question.answer.clone(), placeholder: "Leave blank if this policy is not decided yet",
                        oninput: move |event| {
                            if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                                editor.set_question_answer(question_id, event.value());
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
pub fn Confirmation(exercise_id: Uuid) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let draft = state.read().configurations[&exercise_id].draft.clone();
    let pending = draft
        .preparation
        .questions
        .iter()
        .filter(|question| {
            question.answer.trim().is_empty()
                && draft
                    .checklist
                    .criteria
                    .iter()
                    .any(|item| item.id == question.criterion_id)
        })
        .count();
    let order = match draft.preparation.order {
        ChecklistOrder::Unspecified => {
            "Not specified; flag ordering ambiguity when it affects a decision"
        }
        ChecklistOrder::ListedOrder => "Every step must follow the listed order",
        ChecklistOrder::ExplicitOnly => "Only order explicitly required by a step",
    };
    let needs_review = draft.preparation.needs_reconciliation().unwrap_or(true);
    rsx! {
        section { class: "rounded-box border border-base-300 p-4",
            h3 { class: "text-lg font-semibold", "Confirm the exercise" }
            p { class: "mt-2", "{draft.preparation.scene_context}" }
            p { class: "mt-2 text-sm", "Order: {order}" }
            ol { class: "my-3 list-decimal space-y-2 pl-5",
                for criterion in &draft.checklist.criteria { li { "{criterion.text}" } }
            }
            p { class: "text-sm", "{draft.references.len()} reference images. {pending} pending clarification questions." }
            for question in draft.preparation.questions.iter().filter(|question| draft.checklist.criteria.iter().any(|item| item.id == question.criterion_id)) {
                div { class: "mt-2 text-sm",
                    p { class: "font-semibold", "{question.question}" }
                    if question.answer.trim().is_empty() { p { class: "text-warning", "Pending; no operator standard supplied" } }
                    else { p { "{question.answer}" } }
                }
            }
            p { class: "mt-2 text-sm opacity-70", "Confirmation saves an immutable version. Future edits create a new version; completed analyses keep their original inputs." }
            if needs_review {
                div { class: "alert alert-warning mt-3",
                    span { "The source, context or answers changed. Check that the final checklist still expresses your requirements." }
                    button { id: "exercise-reconcile", class: "btn btn-sm",
                        onclick: move |_| state.write().edit(exercise_id, |draft| { let _ = draft.preparation.reconcile(); }),
                        "I reviewed the final wording"
                    }
                }
            }
        }
    }
}
