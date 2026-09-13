use super::{
    assessment::AssessmentList,
    evidence::EvidenceViewer,
    labels::{recorded_at, stage_label, timeline},
    progress::RunProgress,
    setup::RunSetup,
};
use crate::{
    analysis::{self, AnalysisContext},
    operator::OperatorState,
};
use dioxus::prelude::*;
use std::path::PathBuf;
use uuid::Uuid;

/// Recording selection, immutable run history, and the selected run's review workspace.
#[component]
pub fn Evaluation() -> Element {
    let operator = use_context::<Signal<OperatorState>>();
    let selected = {
        let operator = operator.read();
        operator
            .selected_session_id
            .and_then(|id| {
                operator
                    .sessions
                    .iter()
                    .find(|row| row.stored.session.id == id)
            })
            .map(|row| {
                (
                    row.stored.session.id,
                    row.stored.directory.clone(),
                    row.stored.session.start_utc_ms,
                    row.stored.session.end_offset.as_millis() as u64,
                    row.stored.session.cameras.len(),
                )
            })
    };
    rsx! {
        if let Some((session_id, directory, start_ms, duration_ms, cameras)) = selected {
            SelectedRecording {
                key: "{session_id}",
                session_id,
                directory,
                start_ms,
                duration_ms,
                cameras,
            }
        } else {
            p { class: "rounded-box border border-base-300 p-5",
                "Select a completed session from the recording list."
            }
        }
    }
}

#[component]
fn SelectedRecording(
    session_id: Uuid,
    directory: PathBuf,
    start_ms: i64,
    duration_ms: u64,
    cameras: usize,
) -> Element {
    let context = use_context::<AnalysisContext>();
    let directory_loader = directory.clone();
    use_effect(move || {
        analysis::load_session(context, session_id, directory_loader.clone(), false)
    });
    let (selected_run, loading, error) = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .map(|session| {
            (
                session.selected_run_id,
                session.loading,
                session.error.clone(),
            )
        })
        .unwrap_or_default();
    rsx! {
        header { class: "rounded-box bg-base-200 p-4",
            h2 { class: "text-lg font-semibold", "Recording: {recorded_at(start_ms)}" }
            p { class: "mt-1 text-sm", "Duration: {timeline(duration_ms)}. Cameras: {cameras}." }
            details { class: "mt-2 text-xs opacity-70",
                summary { "Recording details" }
                p { class: "mt-1 break-all", "{directory.display()}" }
                p { "{session_id}" }
            }
        }
        RunSetup { session_id }
        if loading {
            p { role: "status", "Loading saved runs..." }
        }
        if let Some(error) = error {
            p { class: "alert alert-error", role: "alert", "{error}" }
        }
        RunHistory { session_id, directory }
        if let Some(run_id) = selected_run {
            SelectedRun { key: "{run_id}", session_id, run_id }
        } else if !loading {
            p { class: "text-sm opacity-70", "No analysis runs saved for this recording yet." }
        }
    }
}

#[component]
fn RunHistory(session_id: Uuid, directory: PathBuf) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut state = context.state;
    let (mut runs, selected) = state
        .read()
        .sessions
        .get(&session_id)
        .map(|session| {
            (
                session
                    .runs
                    .values()
                    .map(|entry| {
                        let run = &entry.snapshot;
                        (
                            run.inputs.created_utc_ms,
                            run.inputs.id,
                            run.inputs.exercise.name.clone(),
                            run.checkpoint.stage,
                        )
                    })
                    .collect::<Vec<_>>(),
                session.selected_run_id,
            )
        })
        .unwrap_or_default();
    runs.sort_by_key(|run| std::cmp::Reverse((run.0, run.1)));
    rsx! {
        div { class: "flex flex-wrap items-end gap-2",
            label { class: "form-control min-w-48 flex-1",
                span { class: "label-text mb-1 font-semibold", "Saved analysis runs" }
                select {
                    id: "analysis-run-history",
                    class: "select select-bordered w-full",
                    value: selected.map(|id| id.to_string()).unwrap_or_default(),
                    onchange: move |event| {
                        if let Some(session) = state.write().sessions.get_mut(&session_id) {
                            session.selected_run_id = Uuid::parse_str(&event.value()).ok();
                        }
                    },
                    if runs.is_empty() {
                        option { value: "", "No runs yet" }
                    }
                    for (time, id, name, stage) in runs {
                        option { value: "{id}",
                            "{recorded_at(time as i64)} · {name} · {stage_label(stage)}"
                        }
                    }
                }
            }
            button {
                class: "btn btn-ghost btn-sm",
                onclick: move |_| analysis::load_session(context, session_id, directory.clone(), true),
                "Refresh history"
            }
        }
    }
}

#[component]
fn SelectedRun(session_id: Uuid, run_id: Uuid) -> Element {
    let context = use_context::<AnalysisContext>();
    let snapshot = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .map(|entry| entry.snapshot.clone());
    let Some(snapshot) = snapshot else {
        return rsx! {};
    };
    rsx! {
        section {
            class: "flex min-w-0 flex-col gap-4 rounded-box border border-base-300 p-4",
            aria_label: "Saved run review",
            div {
                h2 { class: "text-lg font-semibold", "{snapshot.inputs.exercise.name}" }
                p { class: "mt-1 text-sm opacity-70",
                    "This run uses its saved checklist, reference images, and model set: {snapshot.inputs.model_set.name}."
                }
            }
            RunProgress { session_id, run_id, snapshot: snapshot.clone() }
            AssessmentList { session_id, run_id, snapshot }
            EvidenceViewer { session_id, run_id }
        }
    }
}
