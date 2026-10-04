use std::time::Duration;

use backend::{
    exercise::ChecklistProposal,
    model::{ModelClient, ScheduledJob, plan_execution},
};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    inference::InferenceGate,
    settings::{ResolvedSettings, WorkflowSettings},
};

use super::state::ExerciseContext;

/// Explicit model selection and a temporary wording proposal, separate from the saved checklist.
#[component]
pub fn ChecklistAssistance(exercise_id: Uuid) -> Element {
    let context = use_context::<ExerciseContext>();
    let mut state = context.state;
    let gate = use_context::<InferenceGate>();
    let active_task = gate.active.read().as_ref().map(|task| task.label.clone());
    let active = use_context::<ResolvedSettings>();
    let workflow = active.settings.workflow;
    let Some((formatting, proposal, stale, ready, selected_model)) =
        state.read().configurations.get(&exercise_id).map(|editor| {
            (
                editor.formatting,
                editor.draft.preparation.proposal.clone(),
                editor
                    .draft
                    .preparation
                    .proposal
                    .as_ref()
                    .is_some_and(|proposal| proposal.validate_against(&editor.draft).is_err()),
                !editor.draft.checklist.criteria.is_empty()
                    && editor
                        .draft
                        .checklist
                        .criteria
                        .iter()
                        .all(|item| !item.text.trim().is_empty()),
                state
                    .read()
                    .formatter_model_id
                    .clone()
                    .or_else(|| workflow.models.first().map(|model| model.id.clone())),
            )
        })
    else {
        return rsx! {};
    };
    let choices = workflow
        .models
        .iter()
        .map(|model| (model.id.clone(), model.name.clone()))
        .collect::<Vec<_>>();
    let requester = context.clone();
    let request_model = selected_model.clone();
    rsx! {
        section {
            class: "mt-4 rounded-box border border-base-300 p-3",
            aria_label: "Checklist formatting assistance",
            h4 { class: "font-semibold", "Format and review" }
            p { class: "mt-1 text-sm opacity-70",
                "The model suggests clearer wording and raises questions. Review the proposal, then apply only what you want to your editable checklist."
            }
            div { class: "mt-3 flex flex-wrap items-end gap-2",
                label { class: "form-control min-w-48 flex-1",
                    span { class: "label-text mb-1", "Checklist review model" }
                    select {
                        id: "exercise-formatter-model",
                        class: "select select-bordered w-full",
                        value: selected_model.clone().unwrap_or_default(),
                        disabled: formatting,
                        onchange: move |event| state.write().formatter_model_id = Some(event.value()),
                        if choices.is_empty() {
                            option { value: "", "Add a model in Settings, then restart" }
                        }
                        for (id, name) in choices {
                            option { key: "{id}", value: "{id}", selected: selected_model.as_ref() == Some(&id), "{name}" }
                        }
                    }
                }
                button {
                    id: "exercise-format",
                    class: "btn btn-outline",
                    disabled: formatting || active_task.is_some() || !ready || request_model.is_none(),
                    onclick: move |_| {
                        if let Some(model_id) = request_model.clone() {
                            request(requester.clone(), exercise_id, workflow.clone(), model_id, gate);
                        }
                    },
                    if formatting {
                        "Reviewing..."
                    } else {
                        "Format and review"
                    }
                }
            }
            if let Some(ref active_task) = active_task {
                p { class: "mt-2 text-sm", role: "status", "Model task in progress: {active_task}" }
            }
            if let Some(proposal) = proposal {
                if stale {
                    p { class: "alert alert-warning mt-3",
                        "This proposal belongs to an earlier draft. Request a new review to apply suggestions to the current checklist."
                    }
                }
                div { class: "mt-3 flex flex-col gap-3",
                    for (index, item) in proposal.criteria.iter().enumerate() {
                        ProposalItem {
                            key: "{index}",
                            exercise_id,
                            index,
                            source: item.source_ranges
                                .iter()
                                .filter_map(|range| proposal.source_text.get(range.start..range.end))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            text: item.text.clone(),
                        }
                    }
                }
                ReviewComments { proposal: proposal.clone() }
                div { class: "mt-3 flex gap-2",
                    button {
                        id: "exercise-apply-proposal",
                        class: "btn btn-primary btn-sm",
                        disabled: stale,
                        onclick: move |_| {
                            if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                                if let Err(error) = editor.apply_proposal() {
                                    editor.error = Some(error);
                                }
                            }
                        },
                        "Apply to editable checklist"
                    }
                    button {
                        class: "btn btn-ghost btn-sm",
                        onclick: move |_| {
                            if let Some(editor) = state.write().configurations.get_mut(&exercise_id) {
                                editor.draft.preparation.proposal = None;
                            }
                        },
                        "Dismiss proposal"
                    }
                }
            }

        }
    }
}

#[component]
fn ProposalItem(exercise_id: Uuid, index: usize, source: String, text: String) -> Element {
    let ExerciseContext { mut state, .. } = use_context::<ExerciseContext>();
    rsx! {
        div { class: "grid gap-2 md:grid-cols-2",
            div { class: "rounded bg-base-200 p-3",
                p { class: "text-xs font-semibold", "Your current wording" }
                p { class: "mt-1 whitespace-pre-wrap text-sm", "{source}" }
            }
            label { class: "form-control",
                span { class: "label-text mb-1", "Suggested step {index + 1} (editable)" }
                textarea {
                    class: "textarea textarea-bordered w-full",
                    rows: 3,
                    value: text,
                    oninput: move |event| {
                        if let Some(item) = state
                            .write()
                            .configurations
                            .get_mut(&exercise_id)
                            .and_then(|editor| editor.draft.preparation.proposal.as_mut())
                            .and_then(|proposal| proposal.criteria.get_mut(index))
                        {
                            item.text = event.value();
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn ReviewComments(proposal: ChecklistProposal) -> Element {
    rsx! {
        if !proposal.comments.is_empty() {
            div { class: "mt-3 rounded bg-base-200 p-3",
                h5 { class: "text-sm font-semibold", "Questions and suggestions" }
                ul { class: "mt-2 list-disc space-y-2 pl-5 text-sm",
                    for comment in proposal.comments {
                        li {
                            span { class: "font-semibold",
                                {
                                    match comment.kind {
                                        backend::exercise::ProposalCommentKind::Ambiguity => "Question: ",
                                        backend::exercise::ProposalCommentKind::SourceDiscrepancy => {
                                            "Source mismatch: "
                                        }
                                        backend::exercise::ProposalCommentKind::SuggestedChange => "Suggestion: ",
                                    }
                                }
                            }
                            if let Some(index) = comment.proposed_item {
                                span { class: "font-semibold", "Step {index + 1}: " }
                            }
                            "{comment.text}"
                        }
                    }
                }
            }
        }
    }
}

fn request(
    mut context: ExerciseContext,
    exercise_id: Uuid,
    workflow: WorkflowSettings,
    model_id: String,
    gate: InferenceGate,
) {
    let lease = match gate.acquire("checklist review") {
        Ok(lease) => lease,
        Err(error) => {
            if let Some(editor) = context.state.write().configurations.get_mut(&exercise_id) {
                editor.error = Some(error);
            }
            return;
        }
    };
    let draft = {
        let mut page = context.state.write();
        let Some(editor) = page.configurations.get_mut(&exercise_id) else {
            return;
        };
        if editor.formatting {
            return;
        }
        if editor.draft.preparation.source.is_none() {
            editor.draft.preparation.source = Some(editor.draft.checklist.clone());
            if let Err(error) = editor.draft.preparation.reconcile() {
                editor.error = Some(error.to_string());
                return;
            }
        }
        editor.formatting = true;
        editor.error = None;
        editor.draft.clone()
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let _lease = lease;
        let result = async {
            let model = workflow
                .models
                .iter()
                .find(|profile| profile.id == model_id)
                .ok_or("The checklist review model is missing.")?
                .resolve(&workflow.providers)
                .map_err(|error| error.to_string())?;
            plan_execution(
                &[ScheduledJob {
                    id: "checklist-format".into(),
                    profile_id: model.profile.id.clone(),
                }],
                std::slice::from_ref(&model),
                &workflow.execution,
            )
            .map_err(|error| error.to_string())?;
            let provider = workflow
                .providers
                .iter()
                .find(|provider| provider.id == model.provider.id)
                .ok_or("The formatter provider is missing.")?;
            let client = ModelClient::new(
                provider,
                &model.profile,
                Duration::from_secs(workflow.execution.request_timeout_seconds),
            )
            .map_err(|error| error.to_string())?;
            backend::exercise::format_checklist(&draft, &client)
                .await
                .map(|completion| completion.value)
                .map_err(|error| error.to_string())
        }
        .await;
        if let Some(editor) = context.state.write().configurations.get_mut(&exercise_id) {
            editor.formatting = false;
            match result {
                Ok(proposal) => {
                    if let Err(error) = editor.receive_proposal(proposal) {
                        editor.error = Some(error);
                    }
                }
                Err(error) => editor.error = Some(error),
            }
        }
    });
}
