use super::labels::{stage_label, timeline};
use crate::{
    analysis::{self, AnalysisContext},
    inference::InferenceGate,
    operator::{OperatorState, SessionRunState},
    settings::ResolvedSettings,
};
use backend::analysis::{AnalysisWarning, JobKind, JobState, RunSnapshot, RunStage};
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
    let total = snapshot.checkpoint.jobs.len();
    let complete = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.state == JobState::Complete)
        .count();
    let reader_rows = snapshot
        .inputs
        .model_set
        .readers
        .iter()
        .map(|reader| {
            let jobs = snapshot
                .checkpoint
                .jobs
                .iter()
                .filter(|job| job.kind == JobKind::Reader && job.profile_id == reader.profile.id)
                .collect::<Vec<_>>();
            (
                reader.profile.name.clone(),
                jobs.iter()
                    .filter(|job| job.state == JobState::Complete)
                    .count(),
                jobs.len(),
                jobs.iter()
                    .filter(|job| matches!(job.state, JobState::Failed { .. }))
                    .count(),
            )
        })
        .collect::<Vec<_>>();
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
                        disabled: busy || recording,
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
                "Completed model requests: {complete} of {total}. Additional evidence can add requests."
            }
            table { class: "table table-sm",
                thead {
                    tr {
                        th { "Independent reader" }
                        th { "Progress" }
                        th { "Processing" }
                    }
                }
                tbody {
                    for (name, complete, total, failures) in reader_rows {
                        tr {
                            td { "{name}" }
                            td { "{complete} / {total}" }
                            td {
                                if failures > 0 {
                                    "{failures} request(s) need retry"
                                } else if complete == total && total > 0 {
                                    "Complete"
                                } else {
                                    "Pending or running"
                                }
                            }
                        }
                    }
                }
            }
            if snapshot.checkpoint.stage == RunStage::Paused {
                p { class: "alert alert-warning",
                    "Processing paused. Completed findings are preserved; unfinished requests are not candidate failures."
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
                                "Recording gap on camera {camera_id}: {timeline(start_offset_ms)} to {timeline(end_offset_ms)}."
                            }
                        }
                    }
                }
            }
        }
    }
}
