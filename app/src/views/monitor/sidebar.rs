use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backend::recording::RecorderStatus;
use dioxus::prelude::*;

use crate::operator::{self, OperatorState, SessionRunState};

/// Renders recording lifecycle and selected-camera controls for the Monitor route.
#[component]
pub fn Sidebar() -> Element {
    let mut operator_state = use_context::<Signal<OperatorState>>();
    let state = operator_state.read();
    let cameras = state
        .cameras
        .iter()
        .map(|camera| {
            (
                camera.config.id,
                camera.config.name.clone(),
                camera.recorder_status,
            )
        })
        .collect::<Vec<_>>();
    let selected = state.selected_camera_id.and_then(|camera_id| {
        state
            .cameras
            .iter()
            .find(|camera| camera.config.id == camera_id)
            .map(|camera| {
                (
                    camera.config.id,
                    camera.config.name.clone(),
                    camera.participating,
                    camera.active_monitoring_profile_id,
                )
            })
    });

    if cameras.is_empty() {
        drop(state);
        return rsx! {
            section {
                class: "flex flex-col gap-3",
                aria_labelledby: "recording-controls-title",
                h2 {
                    id: "recording-controls-title",
                    class: "text-lg font-semibold",
                    "Recording"
                }
                p { role: "status", "No cameras are configured" }
            }
        };
    }

    match &state.session {
        SessionRunState::Idle => {
            let session_root = state.session_root.display().to_string();
            let analysis_running = state.running_analysis_id.is_some();
            drop(state);

            rsx! {
                section {
                    class: "flex flex-col gap-5",
                    aria_labelledby: "recording-controls-title",
                    h2 {
                        id: "recording-controls-title",
                        class: "text-lg font-semibold",
                        "Recording"
                    }
                    p {
                        class: "text-sm",
                        role: "status",
                        aria_live: "polite",
                        "Session idle"
                    }
                    button {
                        class: "btn btn-primary w-full",
                        r#type: "button",
                        disabled: analysis_running,
                        onclick: move |_| {
                            let utc_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .ok()
                                .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok());
                            if let Some(utc_ms) = utc_ms {
                                operator::spawn_start_session(operator_state, utc_ms);
                            } else {
                                operator_state
                                    .write()
                                    .set_transient_message(
                                        Some("The system clock cannot start a session.".into()),
                                    );
                            }
                        },
                        "Start session"
                    }
                    if analysis_running {
                        p { class: "text-sm", "Start is unavailable while analysis is running." }
                    }
                    if let Some((camera_id, name, participating, profile_id)) = selected {
                        SelectedCameraControls {
                            key: "{camera_id}",
                            camera_id,
                            name,
                            participating,
                            profile_id,
                        }
                    }
                    div { class: "flex flex-col gap-1 text-sm",
                        h3 { class: "font-medium", "Session root" }
                        p { class: "break-all", "{session_root}" }
                    }
                }
            }
        }
        SessionRunState::Starting { directory } => {
            let directory = directory.display().to_string();
            drop(state);

            rsx! {
                section {
                    class: "flex flex-col gap-5",
                    aria_labelledby: "recording-controls-title",
                    h2 {
                        id: "recording-controls-title",
                        class: "text-lg font-semibold",
                        "Recording"
                    }
                    p {
                        class: "text-sm",
                        role: "status",
                        aria_live: "polite",
                        "Starting session"
                    }
                    button {
                        class: "btn btn-primary w-full",
                        r#type: "button",
                        disabled: true,
                        "Start session"
                    }
                    RecorderStatuses { title: "Camera readiness", cameras }
                    div { class: "flex flex-col gap-1 text-sm",
                        h3 { class: "font-medium", "Staging directory" }
                        p { class: "break-all", "{directory}" }
                    }
                }
            }
        }
        SessionRunState::Active { directory, .. } => {
            let directory = directory.display().to_string();
            drop(state);

            rsx! {
                section {
                    class: "flex flex-col gap-5",
                    aria_labelledby: "recording-controls-title",
                    h2 {
                        id: "recording-controls-title",
                        class: "text-lg font-semibold",
                        "Recording"
                    }
                    p {
                        class: "text-sm",
                        role: "status",
                        aria_live: "polite",
                        "Session active"
                    }
                    ElapsedTime {}
                    RecorderStatuses { title: "Recorder health", cameras }
                    if let Some((camera_id, name, participating, profile_id)) = selected {
                        SelectedCameraControls {
                            key: "{camera_id}",
                            camera_id,
                            name,
                            participating,
                            profile_id,
                        }
                    }
                    button {
                        class: "btn btn-error w-full",
                        r#type: "button",
                        onclick: move |_| operator::spawn_stop_session(operator_state),
                        "Stop session"
                    }
                    div { class: "flex flex-col gap-1 text-sm",
                        h3 { class: "font-medium", "Session directory" }
                        p { class: "break-all", "{directory}" }
                    }
                }
            }
        }
        SessionRunState::Stopping { directory } => {
            let directory = directory.display().to_string();
            drop(state);

            rsx! {
                section {
                    class: "flex flex-col gap-5",
                    aria_labelledby: "recording-controls-title",
                    h2 {
                        id: "recording-controls-title",
                        class: "text-lg font-semibold",
                        "Recording"
                    }
                    p {
                        class: "text-sm",
                        role: "status",
                        aria_live: "polite",
                        "Finalizing session"
                    }
                    RecorderStatuses { title: "Recorder finalization", cameras }
                    button {
                        class: "btn btn-error w-full",
                        r#type: "button",
                        disabled: true,
                        "Stop session"
                    }
                    div { class: "flex flex-col gap-1 text-sm",
                        h3 { class: "font-medium", "Session directory" }
                        p { class: "break-all", "{directory}" }
                    }
                }
            }
        }
        SessionRunState::Faulted { directory, .. } => {
            let directory = directory.display().to_string();
            drop(state);

            rsx! {
                section {
                    class: "flex flex-col gap-4",
                    aria_labelledby: "recording-controls-title",
                    h2 {
                        id: "recording-controls-title",
                        class: "text-lg font-semibold",
                        "Recording"
                    }
                    div { class: "rounded-box border border-error/30 bg-error/10 p-3 text-sm",
                        div { class: "flex flex-col gap-2",
                            p { class: "font-medium", "Session faulted" }
                            p {
                                "Recorder cleanup was attempted; inspect the session directory, then restart Leo before starting another session."
                            }
                        }
                    }
                    div { class: "flex flex-col gap-1 text-sm",
                        h3 { class: "font-medium", "Faulted session directory" }
                        p { class: "break-all", "{directory}" }
                    }
                }
            }
        }
    }
}

#[component]
fn RecorderStatuses(title: &'static str, cameras: Vec<(u32, String, RecorderStatus)>) -> Element {
    rsx! {
        section {
            class: "flex flex-col gap-2",
            aria_label: "{title}",
            role: "status",
            aria_live: "polite",
            h3 { class: "text-sm font-medium", "{title}" }
            ul { class: "flex flex-col gap-2",
                for (camera_id, name, status) in cameras {
                    li {
                        key: "{camera_id}",
                        class: "flex items-center justify-between gap-2 text-sm",
                        span { "{name}" }
                        span {
                            class: "badge badge-outline",
                            aria_label: "{name} recorder status: {recorder_status_label(status)}",
                            "{recorder_status_label(status)}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SelectedCameraControls(
    camera_id: u32,
    name: String,
    participating: bool,
    profile_id: u32,
) -> Element {
    let operator_state = use_context::<Signal<OperatorState>>();
    let mut chosen_profile = use_signal(move || profile_id);
    let state = operator_state.read();
    let profiles = state.monitoring_profiles.clone();
    let current_name = profiles
        .iter()
        .find(|p| p.id == profile_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Unavailable".into());
    let warning = state.metadata_error.clone().or_else(|| {
        state.monitoring_config_error.as_ref().map(|error| {
            format!("Recording remains available. Monitoring metadata needs correction: {error}")
        })
    });
    let disabled = warning.is_some()
        || !matches!(
            state.session,
            SessionRunState::Idle
                | SessionRunState::Active {
                    controller: Some(_),
                    ..
                }
        );
    let all_cameras = state
        .cameras
        .iter()
        .map(|camera| camera.config.id)
        .collect::<Vec<_>>();
    drop(state);
    let participation_action = if participating {
        "Exclude from analysis"
    } else {
        "Include in analysis"
    };
    rsx! {
        section { class: "flex flex-col gap-3 border-t border-base-300 pt-4",
            h3 { class: "font-medium", "Selected camera" }
            p { class: "text-sm", "{name}" }
            span { class: "badge badge-outline", "{current_name}" }
            if let Some(warning) = warning {
                p { class: "alert alert-warning text-sm", role: "status", "{warning}" }
            }
            button {
                class: "btn btn-outline btn-sm w-full",
                r#type: "button",
                disabled,
                onclick: move |_| {
                    let _ = operator::set_participation(operator_state, camera_id, !participating);
                },
                "{participation_action}"
            }
            label {
                class: "text-sm font-medium",
                r#for: "monitoring-profile-{camera_id}",
                "Monitoring profile"
            }
            select {
                id: "monitoring-profile-{camera_id}",
                class: "select select-bordered w-full",
                disabled,
                value: "{chosen_profile}",
                onchange: move |event| chosen_profile.set(event.value().parse().unwrap_or(0)),
                option { value: "0", "Select a profile" }
                for profile in profiles {
                    option { value: "{profile.id}", "{profile.name}" }
                }
            }
            button {
                class: "btn btn-sm w-full",
                r#type: "button",
                disabled,
                onclick: move |_| {
                    let _ = operator::set_monitoring_profile(
                        operator_state,
                        vec![camera_id],
                        chosen_profile(),
                    );
                },
                "Apply to selected camera"
            }
            button {
                class: "btn btn-outline btn-sm w-full",
                r#type: "button",
                disabled,
                onclick: move |_| {
                    let _ = operator::set_monitoring_profile(
                        operator_state,
                        all_cameras.clone(),
                        chosen_profile(),
                    );
                },
                "Apply to all cameras"
            }
        }
    }
}

#[component]
fn ElapsedTime() -> Element {
    let operator = use_context::<Signal<OperatorState>>();
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick += 1;
        }
    });
    let _ = tick();
    let elapsed = {
        let state = operator.read();
        match &state.session {
            SessionRunState::Active { started_at, .. } => started_at.elapsed(),
            _ => Duration::ZERO,
        }
    };

    rsx! {
        p { class: "font-mono text-lg", "Elapsed time: {format_elapsed(elapsed)}" }
    }
}

fn recorder_status_label(status: RecorderStatus) -> &'static str {
    match status {
        RecorderStatus::Starting => "Starting",
        RecorderStatus::Recording => "Recording",
        RecorderStatus::Reconnecting => "Reconnecting",
        RecorderStatus::Stopped => "Idle",
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
#[path = "tests/sidebar.rs"]
mod tests;
