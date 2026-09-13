use dioxus::prelude::*;
use uuid::Uuid;

use super::{
    checklist::ChecklistEditor, references::ReferenceLibrary, state::ExerciseContext, tasks,
};

/// Configuration selection and the operator's explicit draft-save/publication actions.
#[component]
pub fn ExerciseEditor() -> Element {
    let context = use_context::<ExerciseContext>();
    let mut state = context.state;
    let (choices, selected_id, loading, creating, error) = {
        let page = state.read();
        (
            page.configurations
                .values()
                .map(|editor| (editor.draft.id, editor.draft.name.clone()))
                .collect::<Vec<_>>(),
            page.selected_id,
            page.loading,
            page.creating,
            page.error.clone(),
        )
    };
    let creator = context.clone();
    rsx! {
        section {
            class: "flex min-w-0 flex-col gap-4",
            aria_label: "Exercise configuration",
            div { class: "flex flex-wrap items-end gap-3",
                label { class: "form-control flex-1 min-w-48",
                    span { class: "label-text mb-1 font-semibold", "Exercise configuration" }
                    select {
                        id: "exercise-selector",
                        class: "select select-bordered w-full",
                        value: selected_id.map(|id| id.to_string()).unwrap_or_default(),
                        disabled: loading,
                        onchange: move |event| {
                            state.write().selected_id = Uuid::parse_str(&event.value()).ok();
                        },
                        if choices.is_empty() {
                            option { value: "", "Create your first configuration" }
                        }
                        for (id, name) in choices {
                            option { value: "{id}", "{name}" }
                        }
                    }
                }
                button {
                    id: "exercise-create",
                    class: "btn btn-outline",
                    disabled: loading || creating,
                    onclick: move |_| tasks::create(creator.clone()),
                    if creating {
                        "Creating..."
                    } else {
                        "New configuration"
                    }
                }
            }
            if let Some(error) = error {
                p { class: "alert alert-error", role: "alert", "{error}" }
            }
            if loading {
                p { role: "status", "Loading saved configurations..." }
            }
            if let Some(exercise_id) = selected_id {
                ConfigurationBody { key: "{exercise_id}", exercise_id }
            } else if !loading {
                p { class: "text-sm opacity-70",
                    "Create an exercise, enter its checklist, and add any useful reference images."
                }
            }
        }
    }
}

#[component]
fn ConfigurationBody(exercise_id: Uuid) -> Element {
    let context = use_context::<ExerciseContext>();
    let mut state = context.state;
    let Some((name, dirty, published, busy, error, valid)) =
        state.read().configurations.get(&exercise_id).map(|editor| {
            (
                editor.draft.name.clone(),
                editor.draft != editor.saved,
                editor
                    .published
                    .as_ref()
                    .is_some_and(|revision| revision.edit_version == editor.saved.edit_version),
                editor.saving || editor.importing,
                editor.error.clone(),
                editor.draft.validate_for_publication().is_ok(),
            )
        })
    else {
        return rsx! {};
    };
    let draft_saver = context.clone();
    let publisher = context.clone();
    rsx! {
        label { class: "form-control",
            span { class: "label-text mb-1", "Exercise name" }
            input {
                id: "exercise-name",
                class: "input input-bordered w-full",
                value: name,
                oninput: move |event| state.write().edit(exercise_id, |draft| draft.name = event.value()),
            }
        }
        ChecklistEditor { exercise_id }
        ReferenceLibrary { exercise_id }
        if let Some(error) = error {
            p { class: "alert alert-error", role: "alert", "{error}" }
        }
        div { class: "flex flex-wrap items-center gap-3 border-t border-base-300 pt-4",
            button {
                id: "exercise-save-draft",
                class: "btn btn-outline",
                disabled: busy || !dirty,
                onclick: move |_| tasks::save(draft_saver.clone(), exercise_id, false),
                "Save draft"
            }
            button {
                id: "exercise-publish",
                class: "btn btn-primary",
                disabled: busy || !valid || (!dirty && published),
                onclick: move |_| tasks::save(publisher.clone(), exercise_id, true),
                "Save configuration"
            }
            p { class: "text-sm", role: "status",
                if busy {
                    "Saving or importing..."
                } else if dirty {
                    "Unsaved changes. Save configuration before analysis."
                } else if published {
                    "Configuration saved and ready for analysis."
                } else {
                    "Draft saved. Save configuration when the checklist and references are ready."
                }
            }
        }
    }
}
