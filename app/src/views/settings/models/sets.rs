use crate::views::SettingsContext;
use backend::model::ModelSet;
use dioxus::prelude::*;

/// A set chooses independent readers and a visual reviewer; formatting is a separate model role.
#[component]
pub fn ModelSets() -> Element {
    let mut state = use_context::<SettingsContext>().state;
    let settings = state.read().draft.workflow.clone();
    rsx! {
        div { class: "flex flex-col gap-3",
            h3 { class: "font-semibold", "3. Model sets" }
            for (i, set) in settings.model_sets.iter().enumerate() {
                div {
                    key: "{set.id}",
                    class: "rounded-box border border-base-300 p-3 flex flex-col gap-2",
                    label {
                        "Set name"
                        input {
                            class: "input input-bordered w-full",
                            value: "{set.name}",
                            oninput: move |e| state.write().draft.workflow.model_sets[i].name = e.value(),
                        }
                    }
                    fieldset {
                        legend { class: "font-medium", "Independent readers" }
                        for (m, model) in settings.models.iter().enumerate() {
                            label { class: "flex items-center gap-2 py-1",
                                input {
                                    r#type: "checkbox",
                                    class: "checkbox checkbox-sm",
                                    checked: set.reader_ids.contains(&model.id),
                                    onchange: move |e| {
                                        let mut page = state.write();
                                        let id = page.draft.workflow.models[m].id.clone();
                                        let readers = &mut page.draft.workflow.model_sets[i].reader_ids;
                                        readers.retain(|r| r != &id);
                                        if e.checked() {
                                            readers.push(id);
                                        }
                                    },
                                }
                                "{model.name}"
                            }
                        }
                    }
                    label {
                        "Visual reviewer"
                        select {
                            class: "select select-bordered w-full",
                            value: "{set.reviewer_id}",
                            onchange: move |e| state.write().draft.workflow.model_sets[i].reviewer_id = e.value(),
                            option { value: "", selected: set.reviewer_id.is_empty(), "Select reviewer" }
                            for model in &settings.models {
                                option { key: "{model.id}", value: "{model.id}", selected: model.id == set.reviewer_id, "{model.name}" }
                            }
                        }
                    }
                    label {
                        "Checklist assistant"
                        select {
                            class: "select select-bordered w-full",
                            value: "{set.formatter_id}",
                            onchange: move |e| state.write().draft.workflow.model_sets[i].formatter_id = e.value(),
                            option { value: "", selected: set.formatter_id.is_empty(), "Select checklist model" }
                            for model in &settings.models {
                                option { key: "{model.id}", value: "{model.id}", selected: model.id == set.formatter_id, "{model.name}" }
                            }
                        }
                    }
                    p { class: "text-xs opacity-70",
                        "The reviewer examines visual evidence for every criterion, including when readers agree."
                    }
                    if let Err(error) = settings.resolve(&set.id) {
                        p { class: "text-warning text-sm", "{error}" }
                    }
                    button {
                        class: "btn btn-ghost btn-sm self-start",
                        r#type: "button",
                        onclick: move |_| {
                            state.write().draft.workflow.model_sets.remove(i);
                        },
                        "Remove set"
                    }
                }
            }
            button {
                class: "btn btn-outline btn-sm self-start",
                r#type: "button",
                onclick: move |_| {
                    state
                        .write()
                        .draft
                        .workflow
                        .model_sets
                        .push(ModelSet {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: "New model set".into(),
                            reader_ids: vec![],
                            reviewer_id: String::new(),
                            formatter_id: String::new(),
                        })
                },
                "Add model set"
            }
        }
    }
}
