//! Route-independent recording tasks and recorder-event coordination.

use std::{
    fs,
    future::Future,
    path::{Path, PathBuf},
};

use backend::{
    recording::{Error as RecorderError, RecorderEvent, RecordingSegment},
    session::{OperatorAction, SessionController, mark_recording_complete},
};
use dioxus::prelude::{ReadableExt, Signal, WritableExt};

use super::{
    FaultSessionRequest, OperatorState, SessionRunState, StartSessionRequest, StopSessionRequest,
};

/// Starts one route-independent session task after claiming the synchronous transition.
pub fn spawn_start_session(mut operator: Signal<OperatorState>, utc_ms: i64) {
    let request = {
        let mut state = operator.write();
        match state.begin_start(utc_ms) {
            Ok(request) => request,
            Err(error) => {
                state.set_transient_message(Some(error.to_string()));
                return;
            }
        }
    };
    let start_recorder = request.recorder.clone();
    let recording_cameras = request.recording_cameras.clone();
    let recordings_root = request.directory.join("recordings");
    let current_operator = operator;

    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_start_session_with(
            request,
            async move {
                start_recorder
                    .start(recording_cameras, recordings_root)
                    .await
            },
            move |directory| {
                let state = current_operator.read();
                start_is_current(&state, directory)
            },
        )
        .await;
        apply_start_outcome(&mut operator.write(), outcome);
    });
}

/// Stops one active session without tying finalization to a route lifetime.
pub fn spawn_stop_session(mut operator: Signal<OperatorState>) {
    let request = {
        let mut state = operator.write();
        match state.begin_stop() {
            Ok(request) => {
                state.set_transient_message(None);
                request
            }
            Err(error) => {
                state.set_transient_message(Some(error.to_string()));
                return;
            }
        }
    };
    let recorder = request.recorder.clone();

    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_stop_session_with(request, async move { recorder.stop().await }).await;
        apply_stop_outcome(&mut operator.write(), outcome);
    });
}

/// Runs the single cleanup request already claimed by [`OperatorState::begin_fault`].
pub fn spawn_fault_cleanup(mut operator: Signal<OperatorState>, request: FaultSessionRequest) {
    let recorder = request.recorder.clone();
    dioxus::dioxus_core::spawn_forever(async move {
        let outcome = run_fault_session_with(request, async move { recorder.stop().await }).await;
        apply_fault_outcome(&mut operator.write(), outcome);
    });
}

/// Applies one recorder event and returns a newly claimed fatal cleanup, if any.
pub fn handle_recorder_event(
    operator: &mut OperatorState,
    event: RecorderEvent,
) -> Option<FaultSessionRequest> {
    operator.apply_recorder_event(&event);
    match event {
        RecorderEvent::Status { .. } => None,
        RecorderEvent::Faulted { camera_id, message } => {
            let request = operator.begin_fault(message, true);
            if request.is_some() {
                tracing::error!(camera_id, "fatal recorder event claimed by operator state");
            }
            request
        }
    }
}

/// Faults an in-flight session when its only recorder event boundary disappears.
pub fn handle_recorder_event_channel_closed(
    operator: &mut OperatorState,
) -> Option<FaultSessionRequest> {
    let message = "Recorder runtime stopped unexpectedly.".to_owned();
    let cleanup = operator.begin_fault(message.clone(), true);
    if cleanup.is_none() {
        operator.set_transient_message(Some(message));
    }
    cleanup
}

/// Saves participation without coupling metadata failure to recorder shutdown.
pub fn set_participation(
    mut operator: Signal<OperatorState>,
    camera_id: u32,
    enabled: bool,
) -> Result<(), super::Error> {
    let result = operator.write().set_participation(camera_id, enabled);
    operator
        .write()
        .set_transient_message(result.as_ref().err().map(ToString::to_string));
    result
}

/// Applies one monitoring profile to the selected cameras without affecting capture.
pub fn set_monitoring_profile(
    mut operator: Signal<OperatorState>,
    camera_ids: Vec<u32>,
    profile_id: u32,
) -> Result<(), super::Error> {
    let result = operator
        .write()
        .set_monitoring_profile(camera_ids, profile_id);
    operator
        .write()
        .set_transient_message(result.as_ref().err().map(ToString::to_string));
    result
}

enum StartOutcome {
    Active {
        directory: PathBuf,
        controller: Option<SessionController>,
        metadata_error: Option<String>,
    },
    Idle {
        directory: PathBuf,
        message: String,
    },
    Faulted {
        directory: PathBuf,
        message: String,
    },
    Superseded,
}

enum StopOutcome {
    Completed { warning: Option<String> },
    Faulted { directory: PathBuf, message: String },
}

struct FaultOutcome {
    directory: PathBuf,
    message: String,
}

async fn run_start_session_with<StartFuture, Continue>(
    request: StartSessionRequest,
    start: StartFuture,
    continue_start: Continue,
) -> StartOutcome
where
    StartFuture: Future<Output = Result<(), RecorderError>>,
    Continue: FnOnce(&Path) -> bool,
{
    let camera_ids = request
        .recording_cameras
        .iter()
        .map(|camera| camera.id)
        .collect::<Vec<_>>();
    tracing::info!(
        path = %request.directory.display(),
        camera_ids = ?camera_ids,
        "recorder start awaiting all cameras"
    );
    if let Err(error) = start.await {
        let message = format!("Session start failed: {error}");
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "recorder start rolled back"
        );
        if matches!(error, RecorderError::RecorderStartupCleanupFailed) {
            return StartOutcome::Faulted {
                directory: request.directory,
                message,
            };
        }
        return remove_failed_start(request.directory, message);
    }
    if !continue_start(&request.directory) {
        tracing::info!(
            path = %request.directory.display(),
            "session start continuation owned by fault cleanup"
        );
        return StartOutcome::Superseded;
    }

    let metadata = match request.metadata_error {
        Some(error) => Err(error),
        None => SessionController::create(
            request.events_path,
            request.session_cameras,
            request.monitoring_profiles,
        )
        .map_err(|error| error.to_string()),
    };
    let (controller, metadata_error) = match metadata {
        Ok(controller) => (Some(controller), None),
        Err(error) => (
            None,
            Some(format!(
                "Recording continues. Session metadata start failed: {error}. This session needs repair before analysis."
            )),
        ),
    };
    StartOutcome::Active {
        directory: request.directory,
        controller,
        metadata_error,
    }
}

fn remove_failed_start(directory: PathBuf, message: String) -> StartOutcome {
    match fs::remove_dir_all(&directory) {
        Ok(()) => StartOutcome::Idle { directory, message },
        Err(error) => StartOutcome::Idle {
            message: format!(
                "{message}; failed to remove staging directory {}: {error}",
                directory.display()
            ),
            directory,
        },
    }
}

fn apply_start_outcome(operator: &mut OperatorState, outcome: StartOutcome) {
    match outcome {
        StartOutcome::Active {
            directory,
            controller,
            metadata_error,
        } => {
            operator.finish_start(directory, controller);
            operator.metadata_error = metadata_error;
        }
        StartOutcome::Idle { directory, message } => operator.fail_start(&directory, message),
        StartOutcome::Faulted { directory, message } => operator.finish_fault(directory, message),
        StartOutcome::Superseded => {}
    }
}

fn start_is_current(operator: &OperatorState, directory: &Path) -> bool {
    matches!(
        &operator.session,
        SessionRunState::Starting { directory: current } if current == directory
    )
}

async fn run_stop_session_with<StopFuture>(
    mut request: StopSessionRequest,
    stop: StopFuture,
) -> StopOutcome
where
    StopFuture: Future<Output = Result<Vec<RecordingSegment>, RecorderError>>,
{
    let end = match request.controller.as_mut() {
        Some(controller) => controller.apply(OperatorAction::EndSession),
        None => Err(backend::session::Error::MissingSessionStart),
    };
    if let Err(error) = &end {
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "session end event failed; recorder Stop still required"
        );
    } else {
        tracing::info!(path = %request.directory.display(), "session end event written");
    }

    let stopped = stop.await;
    if let Err(error) = &stopped {
        tracing::error!(
            path = %request.directory.display(),
            error = %error,
            "recorder Stop failed"
        );
    } else {
        tracing::info!(path = %request.directory.display(), "recorder Stop completed");
    }

    let message = match (end, stopped) {
        (Ok(()), Ok(_)) => match mark_recording_complete(&request.directory) {
            Ok(()) => {
                tracing::info!(
                    path = %request.directory.display(),
                    "recording completion marker written"
                );
                return StopOutcome::Completed { warning: None };
            }
            Err(error) => {
                return StopOutcome::Completed {
                    warning: Some(format!(
                        "Recording saved; metadata needs repair. Completion marker failed: {error}"
                    )),
                };
            }
        },
        (Err(error), Ok(_)) => {
            return StopOutcome::Completed {
                warning: Some(format!(
                    "Recording saved; metadata needs repair. Session end event failed: {error}"
                )),
            };
        }
        (Ok(()), Err(error)) => format!("Recorder Stop failed: {error}"),
        (Err(end_error), Err(stop_error)) => {
            format!("Session end event failed: {end_error}; recorder Stop failed: {stop_error}")
        }
    };
    StopOutcome::Faulted {
        directory: request.directory,
        message,
    }
}

fn apply_stop_outcome(operator: &mut OperatorState, outcome: StopOutcome) {
    match outcome {
        StopOutcome::Completed { warning } => {
            operator.metadata_error = None;
            if let Err(error) = operator.finish_stop() {
                operator.set_transient_message(Some(format!(
                    "Completed session refresh failed: {error}"
                )));
            }
            if let Some(warning) = warning {
                operator.message = Some(warning);
            }
        }
        StopOutcome::Faulted { directory, message } => operator.finish_fault(directory, message),
    }
}

async fn run_fault_session_with<StopFuture>(
    mut request: FaultSessionRequest,
    stop: StopFuture,
) -> FaultOutcome
where
    StopFuture: Future<Output = Result<Vec<RecordingSegment>, RecorderError>>,
{
    let end_error = request
        .controller
        .as_mut()
        .and_then(|controller| controller.apply(OperatorAction::EndSession).err());
    let stop_error = stop.await.err();
    let mut message = request.message;
    if let Some(error) = end_error {
        message.push_str(&format!("; session end event failed: {error}"));
    }
    if let Some(error) = stop_error {
        message.push_str(&format!("; recorder cleanup failed: {error}"));
    }
    tracing::error!(
        path = %request.directory.display(),
        append_end = request.controller.is_some(),
        cleanup_failed = message.contains("cleanup failed"),
        "fatal session cleanup finished"
    );
    FaultOutcome {
        directory: request.directory,
        message,
    }
}

fn apply_fault_outcome(operator: &mut OperatorState, outcome: FaultOutcome) {
    operator.finish_fault(outcome.directory, outcome.message);
}

#[cfg(all(test, unix))]
#[path = "tests/recording.rs"]
mod tests;
