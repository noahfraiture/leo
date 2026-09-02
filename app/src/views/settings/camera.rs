use dioxus::prelude::*;

use super::state::{SettingsContext, SettingsField, camera_has_error};

/// Edits the ordered camera collection and the currently selected camera.
#[component]
pub fn CameraSettingsSection() -> Element {
    let SettingsContext { mut state, .. } = use_context::<SettingsContext>();
    let (cameras, selected_camera_id, selected) = {
        let page = state.read();
        (
            page.draft
                .cameras
                .iter()
                .cloned()
                .map(|camera| {
                    let has_error = camera_has_error(&page.field_errors, camera.id);
                    (camera, has_error)
                })
                .collect::<Vec<_>>(),
            page.selected_camera_id,
            page.selected_camera_id.and_then(|id| {
                page.draft
                    .cameras
                    .iter()
                    .find(|camera| camera.id == id)
                    .cloned()
                    .map(|camera| {
                        let name_error = page
                            .field_errors
                            .get(&SettingsField::CameraName(id))
                            .cloned();
                        let url_error = page
                            .field_errors
                            .get(&SettingsField::CameraRtspUrl(id))
                            .cloned();
                        let cadence_error = page
                            .field_errors
                            .get(&SettingsField::CameraSampleEvery(id))
                            .cloned();
                        (camera, name_error, url_error, cadence_error)
                    })
            }),
        )
    };

    rsx! {
        section {
            class: "rounded-box min-w-0 border border-base-300 p-4",
            aria_labelledby: "settings-cameras-title",
            div { class: "flex flex-wrap items-center justify-between gap-2",
                h2 {
                    id: "settings-cameras-title",
                    class: "text-xl font-semibold",
                    "Cameras"
                }
                button {
                    class: "btn btn-sm",
                    r#type: "button",
                    onclick: move |_| {
                        state.write().add_camera();
                    },
                    "Add camera"
                }
            }

            if cameras.is_empty() {
                p { class: "mt-3 text-sm", "No cameras are configured." }
            } else {
                ol { class: "mt-3 flex flex-col gap-2",
                    for (camera, has_error) in cameras {
                        li { key: "{camera.id}",
                            button {
                                class: if selected_camera_id == Some(camera.id) {
                                    "btn h-auto w-full justify-start border-primary bg-base-100 p-3 text-left"
                                } else {
                                    "btn btn-ghost h-auto w-full justify-start p-3 text-left"
                                },
                                r#type: "button",
                                aria_pressed: selected_camera_id == Some(camera.id),
                                onclick: move |_| state.write().selected_camera_id = Some(camera.id),
                                span { class: "min-w-0",
                                    span { class: "block font-medium", "{camera.name}" }
                                    span { class: "block text-xs font-normal", "Camera ID {camera.id}" }
                                    if has_error {
                                        span { class: "badge badge-error badge-outline mt-1", "Needs attention" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some((camera, name_error, url_error, cadence_error)) = selected {
                div { class: "mt-5 flex min-w-0 flex-col gap-4 border-t border-base-300 pt-5",
                    div { class: "flex flex-wrap items-center justify-between gap-2",
                        div {
                            h3 { class: "font-semibold", "Selected camera" }
                            p { class: "text-sm", "Immutable ID: {camera.id}" }
                        }
                        button {
                            class: "btn btn-error btn-outline btn-sm",
                            r#type: "button",
                            onclick: move |_| state.write().remove_selected_camera(),
                            "Remove camera"
                        }
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-name",
                            "Name"
                        }
                        input {
                            id: "camera-{camera.id}-name",
                            class: "input input-bordered w-full",
                            r#type: "text",
                            value: camera.name,
                            aria_invalid: name_error.as_ref().map(|_| "true"),
                            aria_describedby: name_error
                                .as_ref()
                                .map(|_| format!("camera-{}-name-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.name = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = name_error {
                            p {
                                id: "camera-{camera.id}-name-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-rtsp-url",
                            "RTSP URL"
                        }
                        input {
                            id: "camera-{camera.id}-rtsp-url",
                            class: "input input-bordered w-full",
                            r#type: "text",
                            value: camera.rtsp_url,
                            aria_invalid: url_error.as_ref().map(|_| "true"),
                            aria_describedby: url_error
                                .as_ref()
                                .map(|_| format!("camera-{}-rtsp-url-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.rtsp_url = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = url_error {
                            p {
                                id: "camera-{camera.id}-rtsp-url-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }

                    label { class: "flex items-center gap-2 text-sm",
                        input {
                            class: "checkbox checkbox-sm",
                            r#type: "checkbox",
                            checked: camera.initially_included_in_analysis,
                            onchange: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.initially_included_in_analysis = event.checked();
                                }
                            },
                        }
                        "Initially include in analysis"
                    }

                    div { class: "flex flex-col gap-1",
                        label {
                            class: "text-sm font-medium",
                            r#for: "camera-{camera.id}-sample-every",
                            "Sample every (seconds)"
                        }
                        input {
                            id: "camera-{camera.id}-sample-every",
                            class: "input input-bordered w-full",
                            r#type: "number",
                            min: "1",
                            step: "1",
                            value: camera.sample_every_secs,
                            aria_invalid: cadence_error.as_ref().map(|_| "true"),
                            aria_describedby: cadence_error
                                .as_ref()
                                .map(|_| format!("camera-{}-sample-every-error", camera.id)),
                            oninput: move |event| {
                                if let Some(camera) = state
                                    .write()
                                    .draft
                                    .cameras
                                    .iter_mut()
                                    .find(|draft| draft.id == camera.id)
                                {
                                    camera.sample_every_secs = event.value();
                                }
                            },
                        }
                        if let Some(ref error) = cadence_error {
                            p {
                                id: "camera-{camera.id}-sample-every-error",
                                class: "text-error text-sm",
                                "{error}"
                            }
                        }
                    }
                }
            }
        }
    }
}
