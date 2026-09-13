use crate::{
    analysis::{self, AnalysisContext, AnalysisTab},
    exercises::ExerciseContext,
    inference::InferenceGate,
    operator::{OperatorState, SessionRunState},
    settings::ResolvedSettings,
};
use dioxus::prelude::*;
use uuid::Uuid;

/// New runs freeze the selected saved exercise and active model set, preserving previous results.
#[component]
pub fn RunSetup(session_id: Uuid) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut state = context.state;
    let exercises = use_context::<ExerciseContext>();
    let mut exercise_state = exercises.state;
    let operator = use_context::<Signal<OperatorState>>();
    let settings = use_context::<ResolvedSettings>();
    let gate = use_context::<InferenceGate>();
    let recording = !matches!(operator.read().session, SessionRunState::Idle);
    let active = gate
        .active
        .read()
        .as_ref()
        .map(|activity| activity.label.clone());
    let selected_set = state.read().model_set_id.clone().or_else(|| {
        settings
            .settings
            .workflow
            .model_sets
            .first()
            .map(|set| set.id.clone())
    });
    let resolved = selected_set
        .as_ref()
        .ok_or_else(|| "Add a model set in Settings and restart Leo.".to_owned())
        .and_then(|id| settings.settings.workflow.resolve(id));
    let reader_names = resolved
        .as_ref()
        .map(|models| {
            models
                .readers
                .iter()
                .map(|model| model.profile.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let revision = exercise_state.read().selected_revision();
    let (exercise_choices, selected_exercise) = {
        let page = exercise_state.read();
        (
            page.configurations
                .values()
                .map(|editor| (editor.draft.id, editor.draft.name.clone()))
                .collect::<Vec<_>>(),
            page.selected_id,
        )
    };
    let model_choices = settings
        .settings
        .workflow
        .model_sets
        .iter()
        .map(|set| (set.id.clone(), set.name.clone()))
        .collect::<Vec<_>>();
    let disabled = recording || active.is_some() || revision.is_none() || resolved.is_err();
    let selected_for_start = selected_set.clone();
    rsx! {
        section {
            class: "rounded-box border border-base-300 p-4",
            aria_label: "Start an analysis run",
            h2 { class: "text-lg font-semibold", "New analysis" }
            div { class: "mt-3 grid gap-3 md:grid-cols-2",
                label { class: "form-control",
                    span { class: "label-text mb-1", "Exercise configuration" }
                    select {
                        id: "analysis-exercise",
                        class: "select select-bordered w-full",
                        value: selected_exercise.map(|id| id.to_string()).unwrap_or_default(),
                        onchange: move |event| {
                            exercise_state.write().selected_id = Uuid::parse_str(&event.value()).ok();
                        },
                        if exercise_choices.is_empty() {
                            option { value: "", "Create an exercise first" }
                        }
                        for (id, name) in exercise_choices {
                            option { value: "{id}", "{name}" }
                        }
                    }
                }
                label { class: "form-control",
                    span { class: "label-text mb-1", "Analysis model set" }
                    select {
                        id: "analysis-model-set",
                        class: "select select-bordered w-full",
                        value: selected_set.unwrap_or_default(),
                        onchange: move |event| state.write().model_set_id = Some(event.value()),
                        if model_choices.is_empty() {
                            option { value: "", "Configure models in Settings" }
                        }
                        for (id, name) in model_choices {
                            option { value: "{id}", "{name}" }
                        }
                    }
                }
            }
            if let Ok(ref models) = resolved {
                p { class: "mt-3 text-sm",
                    "Independent readers: {reader_names}. Evidence reviewer: {models.reviewer.profile.name}."
                }
            }
            if let Err(ref error) = resolved {
                p { class: "mt-3 text-sm text-warning", "{error}" }
            }
            if revision.is_none() {
                p { class: "mt-3 text-sm",
                    "Save the current exercise configuration before running an analysis."
                }
                button {
                    class: "btn btn-ghost btn-sm",
                    onclick: move |_| state.write().tab = AnalysisTab::Configure,
                    "Open exercise setup"
                }
            }
            if recording {
                p { class: "alert alert-warning mt-3",
                    "Analysis is unavailable while recording is active."
                }
            }
            if let Some(active) = active {
                p { class: "mt-3 text-sm", role: "status", "Model task in progress: {active}" }
            }
            button {
                id: "analysis-start",
                class: "btn btn-primary mt-3",
                disabled,
                onclick: move |_| {
                    if let Some(id) = selected_for_start.clone() {
                        analysis::start(
                            context,
                            operator,
                            exercises.clone(),
                            settings.clone(),
                            gate,
                            session_id,
                            id,
                        );
                    }
                },
                "Start new analysis"
            }
        }
    }
}
