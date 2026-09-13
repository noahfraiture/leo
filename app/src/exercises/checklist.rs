use backend::exercise::Criterion;
use dioxus::prelude::*;
use uuid::Uuid;

use super::state::ExerciseContext;

/// Literal operator checklist editing. Reordering keeps identity; deletion clears links explicitly.
#[component]
pub fn ChecklistEditor(exercise_id: Uuid) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let Some((criteria, pasted, pending_delete, linked_count)) =
        state.read().configurations.get(&exercise_id).map(|editor| {
            let linked_count = editor
                .draft
                .references
                .iter()
                .filter(|reference| {
                    editor
                        .pending_delete
                        .is_some_and(|id| reference.criterion_ids.contains(&id))
                })
                .count();
            (
                editor.draft.checklist.criteria.clone(),
                editor.pasted_text.clone(),
                editor.pending_delete,
                linked_count,
            )
        })
    else {
        return rsx! {};
    };
    let count = criteria.len();
    rsx! {
        section {
            class: "rounded-box border border-base-300 p-4",
            aria_label: "Checklist editor",
            h3 { class: "text-lg font-semibold", "1. Checklist" }
            p { class: "mt-1 text-sm opacity-70",
                "This is the only checklist used for analysis. Keep each step's wording under your control."
            }
            div { class: "mt-4 flex flex-col gap-3",
                for (position, criterion) in criteria.into_iter().enumerate() {
                    CriterionRow {
                        key: "{criterion.id}",
                        exercise_id,
                        criterion,
                        position,
                        count,
                    }
                }
            }
            if let Some(criterion_id) = pending_delete {
                div { class: "alert alert-warning mt-3", role: "alert",
                    span { "Delete this step and remove its links from {linked_count} reference images?" }
                    button {
                        class: "btn btn-sm",
                        onclick: move |_| state.write().delete_criterion(exercise_id, criterion_id),
                        "Delete step and links"
                    }
                    button {
                        class: "btn btn-ghost btn-sm",
                        onclick: move |_| {
                            if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                                editor.pending_delete = None;
                            }
                        },
                        "Cancel"
                    }
                }
            }
            label { class: "form-control mt-4",
                span { class: "label-text mb-1", "Paste steps to add, one per line" }
                textarea {
                    id: "exercise-pasted-checklist",
                    class: "textarea textarea-bordered w-full",
                    rows: 4,
                    value: pasted.clone(),
                    placeholder: "Paste the checklist here, or add a blank step below.",
                    oninput: move |event| {
                        if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                            editor.pasted_text = event.value();
                        }
                    },
                }
            }
            div { class: "mt-2 flex gap-2",
                button {
                    id: "exercise-add-pasted",
                    class: "btn btn-outline btn-sm",
                    disabled: pasted.trim().is_empty(),
                    onclick: move |_| {
                        if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                            let added = editor
                                .pasted_text
                                .lines()
                                .filter(|line| !line.trim().is_empty())
                                .map(|line| Criterion {
                                    id: Uuid::new_v4(),
                                    text: line.to_owned(),
                                })
                                .collect::<Vec<_>>();
                            editor.draft.checklist.criteria.extend(added);
                            editor.pasted_text.clear();
                        }
                    },
                    "Add pasted steps"
                }
                button {
                    class: "btn btn-ghost btn-sm",
                    onclick: move |_| {
                        state
                            .write()
                            .edit(
                                exercise_id,
                                |draft| {
                                    draft
                                        .checklist
                                        .criteria
                                        .push(Criterion {
                                            id: Uuid::new_v4(),
                                            text: String::new(),
                                        });
                                },
                            )
                    },
                    "Add blank step"
                }
            }
            super::formatting::ChecklistAssistance { exercise_id }
        }
    }
}

#[component]
fn CriterionRow(exercise_id: Uuid, criterion: Criterion, position: usize, count: usize) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let criterion_id = criterion.id;
    rsx! {
        div {
            class: "flex items-start gap-2",
            "data-criterion-id": "{criterion_id}",
            span { class: "pt-3 text-sm font-semibold", "{position + 1}." }
            textarea {
                class: "textarea textarea-bordered min-w-0 flex-1",
                aria_label: "Step {position + 1}",
                rows: 2,
                value: criterion.text,
                oninput: move |event| {
                    state
                        .write()
                        .edit(
                            exercise_id,
                            |draft| {
                                if let Some(item) = draft
                                    .checklist
                                    .criteria
                                    .iter_mut()
                                    .find(|item| item.id == criterion_id)
                                {
                                    item.text = event.value();
                                }
                            },
                        )
                },
            }
            div { class: "flex flex-col gap-1",
                button {
                    class: "btn btn-ghost btn-xs",
                    disabled: position == 0,
                    aria_label: "Move step {position + 1} up",
                    onclick: move |_| {
                        state
                            .write()
                            .edit(
                                exercise_id,
                                |draft| {
                                    if let Some(index) = draft
                                        .checklist
                                        .criteria
                                        .iter()
                                        .position(|item| item.id == criterion_id)
                                        .filter(|index| *index > 0)
                                    {
                                        draft.checklist.criteria.swap(index, index - 1);
                                    }
                                },
                            )
                    },
                    "Up"
                }
                button {
                    class: "btn btn-ghost btn-xs",
                    disabled: position + 1 >= count,
                    aria_label: "Move step {position + 1} down",
                    onclick: move |_| {
                        state
                            .write()
                            .edit(
                                exercise_id,
                                |draft| {
                                    if let Some(index) = draft
                                        .checklist
                                        .criteria
                                        .iter()
                                        .position(|item| item.id == criterion_id)
                                        .filter(|index| *index + 1 < draft.checklist.criteria.len())
                                    {
                                        draft.checklist.criteria.swap(index, index + 1);
                                    }
                                },
                            )
                    },
                    "Down"
                }
                button {
                    class: "btn btn-ghost btn-xs text-error",
                    aria_label: "Delete step {position + 1}",
                    onclick: move |_| {
                        if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                            editor.pending_delete = Some(criterion_id);
                        }
                    },
                    "Delete"
                }
            }
        }
    }
}
