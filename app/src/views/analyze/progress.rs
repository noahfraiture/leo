use super::labels::{stage_label, timestamp};
use crate::{
    analysis::{self, AnalysisContext},
    inference::InferenceGate,
    operator::{OperatorState, SessionRunState},
    settings::ResolvedSettings,
};
use backend::analysis::{AnalysisWarning, JobState, RunSnapshot, RunStage};
use dioxus::prelude::*;
use uuid::Uuid;

/// Processing progress and retry controls remain separate from the candidate's judgments.
#[component]
pub fn RunProgress(session_id: Uuid, run_id: Uuid, snapshot: RunSnapshot) -> Element {
    let context = use_context::<AnalysisContext>();
    let operator = use_context::<Signal<OperatorState>>();
    let settings = use_context::<ResolvedSettings>();
    let gate = use_context::<InferenceGate>();
    let busy = gate.active.read().is_some();
    let recording = !matches!(operator.read().session, SessionRunState::Idle);
    let (history_loading, decision_saving) = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .map(|session| {
            (
                session.loading,
                session
                    .runs
                    .get(&run_id)
                    .is_some_and(|entry| entry.assessment_draft.saving),
            )
        })
        .unwrap_or_default();
    let total = snapshot.checkpoint.jobs.len();
    let complete = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.state == JobState::Complete)
        .count();
    let failed = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter_map(|job| match &job.state {
            JobState::Failed { message } => Some(message.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    rsx! {
        section { aria_label: "Analysis run progress", class: "flex flex-col gap-3",
            div { class: "flex flex-wrap items-center justify-between gap-2",
                h3 { class: "font-semibold", "{stage_label(snapshot.checkpoint.stage)}" }
                if snapshot.checkpoint.stage != RunStage::Ready {
                    button {
                        id: "analysis-resume",
                        class: "btn btn-outline btn-sm",
                        disabled: busy || recording || history_loading || decision_saving,
                        onclick: move |_| analysis::resume(
                            context,
                            operator,
                            settings.clone(),
                            gate,
                            session_id,
                            run_id,
                        ),
                        "Resume saved run"
                    }
                }
            }
            progress {
                class: "progress progress-primary w-full",
                value: complete,
                max: total.max(1),
                aria_label: "Completed model requests: {complete} of {total}",
            }
            p { class: "text-sm",
                "Completed model requests: {complete} of {total}. The scan is followed by one consolidation and optional bounded closer looks."
            }
            if snapshot.checkpoint.stage == RunStage::Paused {
                p { class: "alert alert-warning",
                    "Processing paused. Completed findings are preserved; unfinished requests are not candidate failures."
                }
                p { class: "text-sm",
                    "Resume keeps this run's original settings. If you change the model or its limits, start a new analysis to use those changes."
                }
            }
            if let Some(error) = snapshot.checkpoint.error {
                p { class: "alert alert-error", role: "alert", "{error}" }
            }
            if !failed.is_empty() {
                details {
                    summary { "Failed request details" }
                    ul { class: "mt-2 list-disc pl-5 text-sm",
                        for error in failed {
                            li { "{error}" }
                        }
                    }
                }
            }
            for warning in snapshot.checkpoint.evidence.warnings {
                match warning {
                    AnalysisWarning::RecordingGap { camera_id, start_offset_ms, end_offset_ms } => {
                        rsx! {
                            p { class: "alert alert-warning text-sm",
                                "Recording gap on camera {camera_id}: {timestamp(start_offset_ms)} to {timestamp(end_offset_ms)}."
                            }
                        }
                    }
                }
            }
        }
    }
}
