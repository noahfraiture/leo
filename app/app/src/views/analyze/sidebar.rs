use crate::workflow::Workflow;
use backend::analysis::AnalysisCheckpoint;
use dioxus::prelude::*;

/// Renders completed sessions and their derived analysis state.
#[component]
pub fn Sidebar() -> Element {
    let mut workflow = use_context::<Signal<Workflow>>();
    let rows = {
        let state = workflow.read();
        state
            .sessions
            .iter()
            .map(|row| {
                let session_id = row.stored.session.id;
                (
                    session_id,
                    row.stored.session.start_utc_ms,
                    row_status(
                        &row.checkpoint,
                        state.running_analysis_id == Some(session_id),
                        state
                            .analysis_error
                            .as_ref()
                            .is_some_and(|(failed_id, _)| *failed_id == session_id),
                    ),
                    state.selected_session_id == Some(session_id),
                )
            })
            .collect::<Vec<_>>()
    };

    rsx! {
        section {
            class: "flex flex-col gap-4",
            aria_labelledby: "completed-sessions-title",
            div {
                class: "flex items-center justify-between gap-2",
                h2 {
                    id: "completed-sessions-title",
                    class: "text-lg font-semibold",
                    "Completed sessions"
                }
                button {
                    class: "btn btn-sm",
                    r#type: "button",
                    onclick: move |_| {
                        let mut state = workflow.write();
                        if let Err(error) = state.refresh_sessions() {
                            state.set_transient_message(Some(error.to_string()));
                        } else {
                            state.set_transient_message(None);
                        }
                    },
                    "Refresh sessions"
                }
            }
            if rows.is_empty() {
                p { class: "text-sm", "No completed sessions found." }
            } else {
                ul { class: "flex flex-col gap-2",
                    for (session_id, start_utc_ms, status, selected) in rows {
                        li { key: "{session_id}",
                            button {
                                class: if selected {
                                    "btn h-auto w-full justify-start border-primary bg-base-100 p-3 text-left"
                                } else {
                                    "btn btn-ghost h-auto w-full justify-start p-3 text-left"
                                },
                                r#type: "button",
                                aria_label: "Session {session_id}, UTC milliseconds: {start_utc_ms}, status: {status}",
                                aria_pressed: selected,
                                onclick: move |_| {
                                    let mut state = workflow.write();
                                    state.selected_session_id = Some(session_id);
                                    state.set_transient_message(None);
                                },
                                span { class: "flex min-w-0 flex-1 flex-col gap-1",
                                    span { class: "text-xs font-normal", "UTC milliseconds: {start_utc_ms}" }
                                    span { class: "badge badge-outline", "{status}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn row_status(
    checkpoint: &Result<Option<AnalysisCheckpoint>, String>,
    running: bool,
    failed: bool,
) -> &'static str {
    if checkpoint.is_err() {
        "Invalid checkpoint"
    } else if running {
        "Running"
    } else if failed {
        "Failed"
    } else if let Ok(Some(checkpoint)) = checkpoint {
        if checkpoint.responses.is_empty() || checkpoint.responses.len() < checkpoint.total_batches
        {
            "In progress"
        } else if checkpoint.warnings.is_empty() {
            "Complete"
        } else {
            "Complete with warning"
        }
    } else {
        "Not started"
    }
}

#[cfg(test)]
mod tests {
    use backend::analysis::{
        ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint, AnalysisIdentity, AnalysisResponse,
        AnalysisWarning,
    };
    use uuid::Uuid;

    use super::row_status;

    fn checkpoint(
        total_batches: usize,
        response_count: usize,
        warnings: Vec<AnalysisWarning>,
    ) -> Result<Option<AnalysisCheckpoint>, String> {
        let response = AnalysisResponse {
            observations: Vec::new(),
            sequence_summary: String::new(),
            checklist_progress: Vec::new(),
        };
        Ok(Some(AnalysisCheckpoint {
            schema_version: ANALYSIS_SCHEMA_VERSION,
            session_id: Uuid::from_u128(1),
            analysis_identity: AnalysisIdentity {
                model: "test-model".into(),
                endpoint_id: "test-endpoint".into(),
            },
            checklist: "Complete the exercise".into(),
            plan_fingerprint: "0123456789abcdef".into(),
            total_batches,
            warnings,
            responses: vec![response; response_count],
        }))
    }

    #[test]
    fn row_status_uses_the_approved_priority_and_zero_response_rule() {
        let none = Ok(None);
        let invalid = Err("invalid".into());
        let zero = checkpoint(2, 0, Vec::new());
        let partial = checkpoint(2, 1, Vec::new());
        let complete = checkpoint(1, 1, Vec::new());
        let warning = checkpoint(
            1,
            1,
            vec![AnalysisWarning::RecordingGap {
                camera_id: 1,
                start_offset_ms: 0,
                end_offset_ms: 1,
            }],
        );

        assert_eq!(row_status(&invalid, true, true), "Invalid checkpoint");
        assert_eq!(row_status(&none, true, true), "Running");
        assert_eq!(row_status(&none, false, true), "Failed");
        assert_eq!(row_status(&zero, false, false), "In progress");
        assert_eq!(row_status(&partial, false, false), "In progress");
        assert_eq!(row_status(&complete, false, false), "Complete");
        assert_eq!(row_status(&warning, false, false), "Complete with warning");
        assert_eq!(row_status(&none, false, false), "Not started");
    }
}
