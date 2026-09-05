use dioxus::prelude::*;

use crate::{components::CameraFeed, operator::OperatorState, preview::PreviewState};

const PREVIEW_ERROR_GUIDANCE: &str = "Check the MediaMTX version and PATH, free the preview ports if occupied, or fix the reported configuration or filesystem issue, then restart the app.";

/// Renders the configured preview grid independently from recorder availability.
#[component]
pub fn Monitor() -> Element {
    let mut operator = use_context::<Signal<OperatorState>>();

    match use_context::<PreviewState>() {
        PreviewState::NoCameras => rsx! {
            p { class: "p-2", "No cameras are configured" }
        },
        PreviewState::Unavailable { message } => {
            let cameras = {
                let state = operator.read();
                state
                    .cameras
                    .iter()
                    .map(|camera| {
                        (
                            camera.config.id,
                            camera.config.name.clone(),
                            state.selected_camera_id == Some(camera.config.id),
                            format!(
                                "{} ({})",
                                camera.config.name,
                                if state.selected_camera_id == Some(camera.config.id) {
                                    "Selected"
                                } else {
                                    "Select"
                                }
                            ),
                        )
                    })
                    .collect::<Vec<_>>()
            };

            rsx! {
                section { class: "flex flex-col gap-4 p-2",
                    div {
                        class: "alert alert-warning",
                        role: "alert",
                        aria_live: "assertive",
                        span { "Live preview is unavailable: {message}. {PREVIEW_ERROR_GUIDANCE}" }
                    }
                    div { class: "flex flex-wrap gap-2",
                        for (camera_id, name, selected, button_label) in cameras {
                            button {
                                class: "btn btn-sm",
                                r#type: "button",
                                aria_label: format!("{} {name}", if selected { "Selected" } else { "Select" }),
                                aria_pressed: selected,
                                onclick: move |_| {
                                    let mut state = operator.write();
                                    if let Err(error) = state.select_camera(camera_id) {
                                        state.set_transient_message(Some(error.to_string()));
                                    }
                                },
                                "{button_label}"
                                MonitoringBadge { camera_id }
                            }
                        }
                    }
                }
            }
        }
        PreviewState::Ready { feeds, script_url } => {
            let cards = {
                let state = operator.read();
                feeds
                    .into_iter()
                    .map(|feed| {
                        let camera = state
                            .cameras
                            .iter()
                            .find(|camera| camera.config.id == feed.camera_id)
                            .expect("preview camera IDs should match operator-state camera IDs");
                        (
                            feed,
                            state.selected_camera_id == Some(camera.config.id),
                            camera.participating,
                            camera.recorder_status,
                        )
                    })
                    .collect::<Vec<_>>()
            };

            rsx! {
                section { class: "p-2", aria_labelledby: "camera-monitor-title",
                    h1 {
                        id: "camera-monitor-title",
                        class: "mb-3 text-xl font-semibold",
                        "Camera monitor"
                    }
                    script { src: script_url, defer: true }
                    div { class: "grid grid-cols-1 gap-4 lg:grid-cols-2",
                        for (feed, selected, participating, recorder_status) in cards {
                            div {
                                key: "{feed.camera_id}",
                                class: "flex flex-col gap-2",
                                MonitoringBadge { camera_id: feed.camera_id }
                                CameraFeed {
                                    feed,
                                    selected,
                                    participating,
                                    recorder_status,
                                    on_select: move |camera_id| {
                                        let mut state = operator.write();
                                        if let Err(error) = state.select_camera(camera_id) {
                                            state.set_transient_message(Some(error.to_string()));
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/monitor.rs"]
mod tests;

#[component]
fn MonitoringBadge(camera_id: u32) -> Element {
    let operator = use_context::<Signal<OperatorState>>();
    let state = operator.read();
    let id = state
        .cameras
        .iter()
        .find(|c| c.config.id == camera_id)
        .map(|c| c.active_monitoring_profile_id);
    let name = state
        .monitoring_profiles
        .iter()
        .find(|p| Some(p.id) == id)
        .map(|p| p.name.as_str())
        .unwrap_or("Monitoring unavailable");
    let label = if state.metadata_error.is_some() {
        format!("Last saved: {name}")
    } else {
        name.to_owned()
    };
    rsx! {
        span {
            class: "badge badge-outline",
            aria_label: "Monitoring profile: {label}",
            "{label}"
        }
    }
}
