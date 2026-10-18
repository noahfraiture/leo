use backend::exercise::{Criterion, ReferenceAsset};
use dioxus::prelude::*;
use uuid::Uuid;

use super::{state::ExerciseContext, tasks};

/// Manual image upload, labels, and criterion associations. This flow never requests model output.
#[component]
pub fn ReferenceLibrary(exercise_id: Uuid) -> Element {
    let context = use_context::<ExerciseContext>();
    let state = context.state;
    let mut picker_key = use_signal(|| 0_u64);
    let Some((references, criteria, previews, busy)) =
        state.read().configurations.get(&exercise_id).map(|editor| {
            (
                editor.draft.references.clone(),
                editor.draft.checklist.criteria.clone(),
                editor.previews.clone(),
                editor.importing || editor.saving,
            )
        })
    else {
        return rsx! {};
    };
    rsx! {
        section {
            class: "rounded-box border border-base-300 p-4",
            aria_label: "Reference library",
            h3 { class: "text-lg font-semibold", "2. Reference images" }
            p { class: "mt-1 text-sm opacity-70",
                "Upload images that clarify objects or states. Label them yourself and link them to the relevant checklist steps."
            }
            // Remount after selection so the native chooser clears its previous filename.
            for picker_key_value in [picker_key()] {
                label { key: "{picker_key_value}", class: "form-control mt-3",
                    span { class: "label-text mb-1", "Upload a JPEG or PNG image" }
                    input {
                        id: "exercise-reference-upload",
                        class: "file-input file-input-bordered w-full",
                        r#type: "file",
                        accept: "image/jpeg,image/png",
                        disabled: busy,
                        onchange: {
                            let context = context.clone();
                            move |event| {
                                if let Some(file) = event.files().into_iter().next() {
                                    tasks::import(context.clone(), exercise_id, file.path());
                                }
                                let next = picker_key().wrapping_add(1);
                                picker_key.set(next);
                            }
                        },
                    }
                }
            }
            div { class: "mt-4 flex flex-col gap-4",
                for reference in references {
                    ReferenceCard {
                        key: "{reference.image.id}",
                        exercise_id,
                        preview: previews.get(&reference.image.id).cloned(),
                        reference,
                        criteria: criteria.clone(),
                    }
                }
            }
        }
    }
}

#[component]
fn ReferenceCard(
    exercise_id: Uuid,
    reference: ReferenceAsset,
    criteria: Vec<Criterion>,
    preview: Option<String>,
) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let asset_id = reference.image.id;
    rsx! {
        article {
            class: "rounded-box border border-base-300 p-3",
            "data-reference-id": "{asset_id}",
            div { class: "flex flex-wrap gap-4",
                div { class: "w-40 shrink-0",
                    if let Some(preview) = preview {
                        img {
                            class: "max-h-40 w-full rounded object-contain",
                            src: preview,
                            alt: "Reference image: {reference.object_label}",
                        }
                    } else {
                        p { class: "text-error text-sm", "Preview unavailable" }
                    }
                    p { class: "mt-2 text-xs opacity-70", "Reference only. Not candidate evidence." }
                    button {
                        class: "btn btn-ghost btn-sm mt-2 text-error",
                        onclick: move |_| {
                            state.write().remove_reference(exercise_id, asset_id);
                        },
                        "Remove reference"
                    }
                }
                div { class: "flex min-w-48 flex-1 flex-col gap-2",
                    ReferenceText {
                        exercise_id,
                        asset_id,
                        field: ReferenceField::Object,
                        label: "Object name",
                        value: reference.object_label,
                    }
                    ReferenceText {
                        exercise_id,
                        asset_id,
                        field: ReferenceField::State,
                        label: "State shown (optional)",
                        value: reference.state_label,
                    }
                    ReferenceText {
                        exercise_id,
                        asset_id,
                        field: ReferenceField::Purpose,
                        label: "What should this image help identify?",
                        value: reference.purpose,
                    }
                }
            }
            fieldset { class: "mt-3 flex flex-col gap-1",
                legend { class: "mb-2 text-sm font-semibold", "Linked checklist steps" }
                if criteria.is_empty() {
                    p { class: "text-sm", "Add checklist steps to link this image." }
                }
                for (index, criterion) in criteria.into_iter().enumerate() {
                    ReferenceLink {
                        key: "{criterion.id}",
                        exercise_id,
                        asset_id,
                        checked: reference.criterion_ids.contains(&criterion.id),
                        criterion,
                        index,
                    }
                }
                if reference.criterion_ids.is_empty() {
                    p { class: "mt-1 text-sm text-warning",
                        "Choose at least one step before saving the configuration."
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ReferenceField {
    Object,
    State,
    Purpose,
}

#[component]
fn ReferenceText(
    exercise_id: Uuid,
    asset_id: Uuid,
    field: ReferenceField,
    label: &'static str,
    value: String,
) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    rsx! {
        label { class: "form-control",
            span { class: "label-text mb-1", "{label}" }
            input {
                class: "input input-bordered w-full",
                initial_value: value,
                oninput: move |event| {
                    state
                        .write()
                        .edit(
                            exercise_id,
                            |draft| {
                                if let Some(reference) = draft
                                    .references
                                    .iter_mut()
                                    .find(|reference| reference.image.id == asset_id)
                                {
                                    match field {
                                        ReferenceField::Object => {
                                            reference.object_label = event.value();
                                        }
                                        ReferenceField::State => {
                                            reference.state_label = event.value();
                                        }
                                        ReferenceField::Purpose => reference.purpose = event.value(),
                                    }
                                }
                            },
                        )
                },
            }
        }
    }
}

#[component]
fn ReferenceLink(
    exercise_id: Uuid,
    asset_id: Uuid,
    criterion: Criterion,
    index: usize,
    checked: bool,
) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    let criterion_id = criterion.id;
    let text = criterion.text.chars().take(120).collect::<String>();
    rsx! {
        label { class: "flex items-start gap-2 text-sm",
            input {
                class: "checkbox checkbox-sm",
                r#type: "checkbox",
                checked,
                onchange: move |event| {
                    state
                        .write()
                        .edit(
                            exercise_id,
                            |draft| {
                                if let Some(reference) = draft
                                    .references
                                    .iter_mut()
                                    .find(|reference| reference.image.id == asset_id)
                                {
                                    reference.criterion_ids.retain(|id| *id != criterion_id);
                                    if event.checked() {
                                        reference.criterion_ids.push(criterion_id);
                                    }
                                }
                            },
                        )
                },
            }
            span { "{index + 1}. {text}" }
        }
    }
}
