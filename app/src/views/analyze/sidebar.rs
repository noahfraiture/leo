use super::labels::recorded_at;
use crate::analysis::{AnalysisContext, AnalysisPageState, AnalysisTab};
use crate::operator::{OperatorState, SessionRunState};
use dioxus::prelude::*;

/// Renders completed sessions and their derived analysis state.
#[component]
pub fn Sidebar() -> Element {
    let mut operator = use_context::<Signal<OperatorState>>();
    let AnalysisContext {
        state: mut analysis_state,
    } = use_context::<AnalysisContext>();
    let rows = {
        let state = operator.read();
        state
            .sessions
            .iter()
            .map(|row| {
                let session_id = row.stored.session.id;
                (
                    session_id,
                    row.stored.session.start_utc_ms,
                    row_status(&analysis_state.read(), session_id),
                    state.selected_session_id == Some(session_id),
                )
            })
            .collect::<Vec<_>>()
    };

    let incomplete = {
        let state = operator.read();
        // A healthy capture has no end marker yet. Keep its row for any later fault or repair.
        let current_capture = match &state.session {
            SessionRunState::Starting { directory }
            | SessionRunState::Active { directory, .. }
            | SessionRunState::Stopping { directory }
                if state.metadata_error.is_none() =>
            {
                Some(directory)
            }
            _ => None,
        };
        state
            .incomplete_sessions
            .iter()
            .filter(|directory| Some(*directory) != current_capture)
            .cloned()
            .collect::<Vec<_>>()
    };
    let refreshing = operator.read().session_refresh_id.is_some();
    let finalizing = matches!(operator.read().session, SessionRunState::Stopping { .. });
    let refresh_completed = operator
        .read()
        .completed_session_refresh_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    rsx! {
        section {
            class: "flex flex-col gap-4",
            aria_labelledby: "completed-sessions-title",
            div { class: "flex items-center justify-between gap-2",
                h2 {
                    id: "completed-sessions-title",
                    class: "text-lg font-semibold",
                    "Completed sessions"
                }
                button {
                    id: "session-refresh",
                    "data-refresh-completed": refresh_completed,
                    class: "btn btn-sm",
                    r#type: "button",
                    disabled: refreshing || finalizing,
                    onclick: move |_| crate::operator::spawn_session_refresh(operator),
                    "Refresh sessions"
                }
            }
            if refreshing {
                p { class: "text-sm", role: "status", "Refreshing sessions..." }
            } else if finalizing {
                p { class: "text-sm", role: "status", "Sessions will refresh when recording finishes." }
            }
            if rows.is_empty() {
                p { class: "text-sm", "No completed sessions found." }
            } else {
                ul { class: "flex flex-col gap-2",
                    for (session_id, start_utc_ms, status, selected) in rows {
                        li { key: "{session_id}",
                            button {
                                class: if selected { "btn h-auto w-full justify-start border-primary bg-base-100 p-3 text-left" } else { "btn btn-ghost h-auto w-full justify-start p-3 text-left" },
                                r#type: "button",
                                aria_label: "Session {session_id}, {recorded_at(start_utc_ms)}, status: {status}",
                                aria_pressed: selected,
                                onclick: move |_| {
                                    let mut state = operator.write();
                                    state.selected_session_id = Some(session_id);
                                    state.set_transient_message(None);
                                    analysis_state.write().tab = AnalysisTab::Evaluate;
                                },
                                span { class: "flex min-w-0 flex-1 flex-col gap-1",
                                    span { class: "text-xs font-normal", "{recorded_at(start_utc_ms)}" }
                                    span { class: "badge badge-outline", "{status}" }
                                }
                            }
                        }
                    }
                }
            }
            if !incomplete.is_empty() {
                h3 { class: "font-semibold", "Recordings needing repair" }
                for directory in incomplete {
                    div { class: "rounded-box border border-base-300 p-3 text-sm",
                        p { "Recording metadata or identity needs repair before analysis." }
                        p { class: "break-all", "{directory.display()}" }
                        button {
                            class: "btn btn-outline btn-sm mt-2",
                            r#type: "button",
                            onclick: move |_| crate::operator::spawn_open_folder(operator, directory.clone()),
                            "Open recording folder"
                        }
                    }
                }
            }

        }
    }
}

fn row_status(page: &AnalysisPageState, session_id: uuid::Uuid) -> &'static str {
    if page
        .active
        .is_some_and(|active| active.session_id == session_id)
    {
        return "Running";
    }
    let Some(session) = page.sessions.get(&session_id) else {
        return "Select to load runs";
    };
    if session.error.is_some() {
        return "Needs attention";
    }
    let latest = session
        .runs
        .values()
        .filter(|entry| entry.available)
        .max_by_key(|entry| entry.snapshot.inputs.created_utc_ms);
    match latest.map(|entry| entry.snapshot.checkpoint.stage) {
        Some(backend::analysis::RunStage::Ready) => "Ready for review",
        Some(backend::analysis::RunStage::Paused) => "Processing paused",
        Some(_) => "Incomplete run",
        None if !session.archived.is_empty() => "Needs attention",
        None => "Not analyzed",
    }
}

#[cfg(test)]
#[path = "tests/sidebar.rs"]
mod tests;
