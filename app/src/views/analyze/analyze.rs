use std::path::PathBuf;

use backend::analysis::{AnalysisCheckpoint, AnalysisWarning, AnalyzeSession};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    analysis_task,
    workflow::{Error as WorkflowError, SessionRunState, Workflow},
};

/// Renders the selected completed session and its persisted analysis results.
#[component]
pub fn Analyze() -> Element {
    let workflow = use_context::<Signal<Workflow>>();
    let selected = {
        let state = workflow.read();
        state.selected_session_id.and_then(|session_id| {
            state
                .sessions
                .iter()
                .find(|row| row.stored.session.id == session_id)
                .map(|row| {
                    (
                        session_id,
                        row.stored.session.start_utc_ms,
                        row.stored.session.end_offset.as_millis(),
                        row.stored.session.cameras.len(),
                        row.stored.directory.clone(),
                        row.checkpoint.clone(),
                    )
                })
        })
    };

    rsx! {
        if let Some((session_id, start_utc_ms, duration_ms, camera_count, directory, checkpoint)) = selected {
            SelectedSession {
                key: "{session_id}",
                session_id,
                start_utc_ms,
                duration_ms,
                camera_count,
                directory,
                checkpoint,
            }
        } else {
            section {
                class: "m-2 rounded-box border border-base-300 p-5",
                aria_labelledby: "analyze-title",
                h1 { id: "analyze-title", class: "text-xl font-semibold", "Analyze sessions" }
                p { class: "mt-2 text-sm", "Select a completed session to analyze." }
            }
        }
    }
}

#[component]
fn SelectedSession(
    session_id: Uuid,
    start_utc_ms: i64,
    duration_ms: u128,
    camera_count: usize,
    directory: PathBuf,
    checkpoint: Result<Option<AnalysisCheckpoint>, String>,
) -> Element {
    let workflow = use_context::<Signal<Workflow>>();
    let (recording_blocked, running_id, model_config_error, analysis_error) = {
        let state = workflow.read();
        (
            !matches!(state.session, SessionRunState::Idle),
            state.running_analysis_id,
            state.model_config_error.clone(),
            state
                .analysis_error
                .as_ref()
                .filter(|(failed_id, _)| *failed_id == session_id)
                .map(|(_, message)| message.clone()),
        )
    };
    let running = running_id == Some(session_id);
    let complete = checkpoint
        .as_ref()
        .ok()
        .and_then(Option::as_ref)
        .is_some_and(|checkpoint| {
            !checkpoint.responses.is_empty()
                && checkpoint.responses.len() == checkpoint.total_batches
        });
    let invalid = checkpoint.is_err();
    let action_disabled = recording_blocked
        || running_id.is_some()
        || complete
        || invalid
        || model_config_error.is_some();
    let events_path = directory.join("events.jsonl");
    let recordings_path = directory.join("recordings");
    let marker_path = directory.join("recording-complete");
    let analysis_path = directory.join("analysis.json");

    rsx! {
        article {
            class: "m-2 flex flex-col gap-5",
            aria_labelledby: "analyze-title",
            header {
                h1 { id: "analyze-title", class: "text-xl font-semibold", "Analyze session" }
                p { class: "mt-1 break-all font-mono text-sm", "{session_id}" }
            }

            section {
                class: "rounded-box border border-base-300 p-4",
                aria_labelledby: "session-recap-title",
                h2 { id: "session-recap-title", class: "mb-3 text-lg font-semibold", "Session recap" }
                dl { class: "grid gap-x-4 gap-y-2 text-sm sm:grid-cols-[max-content_minmax(0,1fr)]",
                    dt { class: "font-medium", "Session UUID" }
                    dd { class: "break-all font-mono", "{session_id}" }
                    dt { class: "font-medium", "Start" }
                    dd { "UTC milliseconds: {start_utc_ms}" }
                    dt { class: "font-medium", "Duration" }
                    dd { "Duration: {duration_ms} ms" }
                    dt { class: "font-medium", "Cameras" }
                    dd { "Camera count: {camera_count}" }
                    dt { class: "font-medium", "Directory" }
                    dd { class: "break-all font-mono", "{directory.display()}" }
                    dt { class: "font-medium", "events.jsonl" }
                    dd { class: "break-all font-mono", "{events_path.display()}" }
                    dt { class: "font-medium", "recordings/" }
                    dd { class: "break-all font-mono", "{recordings_path.display()}" }
                    dt { class: "font-medium", "recording-complete" }
                    dd { class: "break-all font-mono", "{marker_path.display()}" }
                    dt { class: "font-medium", "analysis.json" }
                    dd { class: "break-all font-mono", "{analysis_path.display()}" }
                }
            }

            if recording_blocked {
                p { class: "alert alert-warning", "Analysis is unavailable while recording is active." }
            } else if running {
                p { class: "alert alert-info", role: "status", aria_live: "polite", "Analysis is running." }
            } else if running_id.is_some() {
                p { class: "alert alert-info", "Another session analysis is running." }
            }
            if let Some(error) = model_config_error {
                p { class: "alert alert-warning", "{error}" }
            }
            if !running && let Some(error) = analysis_error {
                p { class: "alert alert-error", role: "alert", "{error}" }
            }
            if let Err(error) = &checkpoint {
                div { class: "alert alert-error", role: "alert",
                    div {
                        p { class: "font-medium", "Invalid analysis checkpoint" }
                        p { "{error}" }
                        p { class: "break-all font-mono text-sm", "{analysis_path.display()}" }
                    }
                }
            }

            match checkpoint {
                Ok(Some(checkpoint)) => rsx! {
                    PersistedChecklist {
                        session_id,
                        checklist: checkpoint.checklist.clone(),
                        action_disabled,
                    }
                    CheckpointResults { checkpoint }
                },
                Ok(None) => rsx! {
                    NewChecklist { session_id, action_disabled }
                },
                Err(_) => rsx! {
                    InvalidChecklist { session_id }
                },
            }
        }
    }
}

#[component]
fn NewChecklist(session_id: Uuid, action_disabled: bool) -> Element {
    let mut checklist = use_signal(String::new);

    rsx! {
        section {
            class: "flex flex-col gap-2",
            aria_labelledby: "analysis-checklist-label",
            label {
                id: "analysis-checklist-label",
                class: "font-medium",
                r#for: "analysis-checklist",
                "Correct-sequence checklist"
            }
            textarea {
                id: "analysis-checklist",
                class: "textarea textarea-bordered min-h-36 w-full",
                value: checklist(),
                oninput: move |event| checklist.set(event.value()),
            }
            AnalysisAction {
                session_id,
                checklist: checklist(),
                label: "Analyze",
                disabled: action_disabled,
            }
        }
    }
}

#[component]
fn PersistedChecklist(session_id: Uuid, checklist: String, action_disabled: bool) -> Element {
    let action_checklist = checklist.clone();

    rsx! {
        section {
            class: "flex flex-col gap-2",
            aria_labelledby: "analysis-checklist-label",
            label {
                id: "analysis-checklist-label",
                class: "font-medium",
                r#for: "analysis-checklist",
                "Correct-sequence checklist"
            }
            textarea {
                id: "analysis-checklist",
                class: "textarea textarea-bordered min-h-36 w-full",
                readonly: true,
                value: checklist.clone(),
            }
            AnalysisAction {
                session_id,
                checklist: action_checklist,
                label: "Resume",
                disabled: action_disabled,
            }
        }
    }
}

#[component]
fn InvalidChecklist(session_id: Uuid) -> Element {
    rsx! {
        section {
            class: "flex flex-col gap-2",
            aria_labelledby: "analysis-checklist-label",
            label {
                id: "analysis-checklist-label",
                class: "font-medium",
                r#for: "analysis-checklist",
                "Correct-sequence checklist"
            }
            textarea {
                id: "analysis-checklist",
                class: "textarea textarea-bordered min-h-36 w-full",
                readonly: true,
                disabled: true,
            }
            AnalysisAction {
                session_id,
                checklist: String::new(),
                label: "Analyze",
                disabled: true,
            }
        }
    }
}

fn prepare_analysis_action(
    workflow: &mut Workflow,
    expected_session_id: Uuid,
    checklist: String,
) -> Result<(Uuid, AnalyzeSession), WorkflowError> {
    let session_id = workflow
        .selected_session_id
        .filter(|selected_id| *selected_id == expected_session_id)
        .ok_or(WorkflowError::AnalysisSessionNotSelected)?;
    let request = workflow.begin_analysis(checklist)?;
    Ok((session_id, request))
}

#[component]
fn AnalysisAction(
    session_id: Uuid,
    checklist: String,
    label: &'static str,
    disabled: bool,
) -> Element {
    let mut workflow = use_context::<Signal<Workflow>>();

    rsx! {
        button {
            id: "analysis-action",
            class: "btn btn-primary self-start",
            r#type: "button",
            disabled,
            onclick: move |_| {
                let prepared = {
                    let mut state = workflow.write();
                    prepare_analysis_action(&mut state, session_id, checklist.clone())
                };
                match prepared {
                    Ok((session_id, request)) => {
                        workflow.write().set_transient_message(None);
                        analysis_task::spawn_analysis(workflow, request, session_id);
                    }
                    Err(error) => workflow
                        .write()
                        .set_transient_message(Some(error.to_string())),
                }
            },
            "{label}"
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt, time::Duration};

    use backend::{
        recording::{RecorderSettings, spawn_for_test},
        session::{OperatorAction, SessionCamera, SessionController, mark_recording_complete},
    };
    use uuid::Uuid;

    use super::prepare_analysis_action;
    use crate::{
        camera_config::CameraConfig,
        workflow::{Error, Workflow},
    };

    #[test]
    fn analysis_preparation_binds_the_rendered_and_selected_session_id() {
        let temporary = tempfile::tempdir().expect("temporary root should be created");
        let executable = temporary.path().join("successful-preflight");
        fs::write(&executable, "#!/bin/sh\nexit 0\n")
            .expect("fake preflight executable should be written");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("fake preflight executable should be executable");
        let (runtime, recorder, _events) = spawn_for_test(
            RecorderSettings {
                io_timeout: Duration::from_secs(1),
                retry_delay: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(1),
            },
            executable.clone(),
            executable,
        )
        .expect("test recorder runtime should start");
        let session_root = temporary.path().join("sessions");
        let directory = session_root.join("current");
        fs::create_dir_all(&directory).expect("session directory should be created");
        let mut controller = SessionController::create(
            directory.join("events.jsonl"),
            vec![SessionCamera {
                id: 1,
                name: "Salon 1".into(),
                enabled: true,
                sample_every: Duration::from_secs(1),
            }],
        )
        .expect("completed session should start");
        controller
            .apply(OperatorAction::EndSession)
            .expect("completed session should end");
        drop(controller);
        mark_recording_complete(&directory).expect("session should be marked complete");
        let mut workflow = Workflow::new(
            vec![CameraConfig {
                id: 1,
                name: "Salon 1".into(),
                rtsp_url: "rtsp://camera-one.example/live".into(),
                enabled: true,
                sample_every_ms: 1_000,
            }],
            session_root,
            recorder,
            Some(crate::test_openai_config()),
        )
        .expect("workflow should load the completed session");
        let selected_id = workflow
            .selected_session_id
            .expect("completed session should be selected");
        let stale_render_id = Uuid::nil();
        assert_ne!(stale_render_id, selected_id);

        let mismatch = prepare_analysis_action(
            &mut workflow,
            stale_render_id,
            "Complete the exercise".into(),
        );

        assert!(matches!(mismatch, Err(Error::AnalysisSessionNotSelected)));
        assert_eq!(workflow.running_analysis_id, None);

        let (routed_id, request) =
            prepare_analysis_action(&mut workflow, selected_id, "Complete the exercise".into())
                .expect("matching selection should claim analysis");
        assert_eq!(routed_id, selected_id);
        assert_eq!(request.directory, directory);
        assert_eq!(request.checklist, "Complete the exercise");
        assert_eq!(workflow.running_analysis_id, Some(routed_id));

        runtime.shutdown().expect("runtime should shut down");
    }
}

#[component]
fn CheckpointResults(checkpoint: AnalysisCheckpoint) -> Element {
    let completed = checkpoint.responses.len();
    let total = checkpoint.total_batches;
    let warnings = checkpoint
        .warnings
        .iter()
        .map(|warning| match warning {
            AnalysisWarning::RecordingGap {
                camera_id,
                start_offset_ms,
                end_offset_ms,
            } => (*camera_id, *start_offset_ms, *end_offset_ms),
        })
        .collect::<Vec<_>>();
    let observations = checkpoint
        .responses
        .iter()
        .flat_map(|response| &response.observations)
        .map(|observation| {
            (
                observation.timestamp.clone(),
                observation.description.clone(),
            )
        })
        .collect::<Vec<_>>();
    let latest = checkpoint.responses.last().cloned();

    rsx! {
        section {
            class: "flex flex-col gap-4 rounded-box border border-base-300 p-4",
            aria_labelledby: "analysis-results-title",
            h2 { id: "analysis-results-title", class: "text-lg font-semibold", "Analysis results" }
            div { class: "flex flex-col gap-2",
                progress {
                    class: "progress progress-primary w-full",
                    value: "{completed}",
                    max: "{total}",
                    aria_label: "Analysis progress: {completed} of {total} batches",
                }
                p { class: "text-sm", "Completed batches: {completed} of {total}" }
            }
            if !warnings.is_empty() {
                section { class: "flex flex-col gap-2", aria_labelledby: "recording-warnings-title",
                    h3 { id: "recording-warnings-title", class: "font-semibold", "Recording warnings" }
                    ul { class: "flex flex-col gap-2",
                        for (camera_id, start_offset_ms, end_offset_ms) in warnings {
                            li {
                                class: "alert alert-warning text-sm",
                                "Recording gap: camera {camera_id}, {start_offset_ms} ms to {end_offset_ms} ms"
                            }
                        }
                    }
                }
            }
            if completed == 0 {
                p { class: "text-sm", "No completed batches yet." }
            } else {
                section { class: "flex flex-col gap-2", aria_labelledby: "observations-title",
                    h3 { id: "observations-title", class: "font-semibold", "Observations" }
                    if observations.is_empty() {
                        p { class: "text-sm", "No observations reported." }
                    } else {
                        ol { class: "flex flex-col gap-2",
                            for (index, (timestamp, description)) in observations.into_iter().enumerate() {
                                li { key: "{index}", class: "rounded-box bg-base-200 p-3 text-sm",
                                    p { class: "font-mono font-medium", "{timestamp}" }
                                    p { "{description}" }
                                }
                            }
                        }
                    }
                }
                if let Some(latest) = latest {
                    section { class: "flex flex-col gap-2", aria_labelledby: "sequence-summary-title",
                        h3 { id: "sequence-summary-title", class: "font-semibold", "Latest sequence summary" }
                        p { class: "text-sm", "{latest.sequence_summary}" }
                    }
                    section { class: "flex flex-col gap-2", aria_labelledby: "checklist-progress-title",
                        h3 { id: "checklist-progress-title", class: "font-semibold", "Latest checklist status" }
                        ul { class: "flex flex-col gap-2",
                            for (index, item) in latest.checklist_progress.into_iter().enumerate() {
                                li { key: "{index}", class: "rounded-box border border-base-300 p-3 text-sm",
                                    div { class: "flex flex-wrap items-center justify-between gap-2",
                                        p { class: "font-medium", "{item.item}" }
                                        span { class: "badge badge-outline", "{item.status}" }
                                    }
                                    p { class: "mt-2", "{item.note}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
