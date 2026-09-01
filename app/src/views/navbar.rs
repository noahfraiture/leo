use crate::{
    Route, RuntimeAvailability,
    views::{Analyze, Monitor, Settings, Sidebar},
    workflow::Workflow,
};
use dioxus::prelude::*;

/// Wraps route navigation, route-specific controls, shared messages, and content.
#[component]
pub fn Layout() -> Element {
    let route = use_route::<Route>();
    let availability = use_context::<RuntimeAvailability>();

    rsx! {
        div {
            class: "flex min-h-screen flex-col gap-2 p-2 lg:h-screen lg:flex-row",
            aside {
                id: "sidebar",
                class: "flex shrink-0 flex-col bg-base-200 p-3 lg:w-80 lg:overflow-y-auto",
                div {
                    class: "shrink-0",
                    NavButton { route: route.clone() }
                }

                hr {
                    class: "border-t shrink-0 my-4",
                }

                div {
                    class: "min-h-0 flex-1 overflow-y-auto",
                    Sidebar { route: route.clone() }
                }
            }
            main {
                id: "body",
                class: "min-w-0 flex-1 overflow-y-auto",
                if matches!(&availability, RuntimeAvailability::Ready { .. })
                    && !matches!(&route, Route::Settings {})
                {
                    WorkflowMessage {}
                }
                Home { route: route.clone(), availability }
            }
        }
    }
}

#[component]
fn WorkflowMessage() -> Element {
    let workflow = use_context::<Signal<Workflow>>();
    let message = workflow.read().message.clone();

    rsx! {
        if let Some(message) = message {
            div {
                class: "alert alert-error m-2",
                role: "alert",
                aria_live: "assertive",
                span { "{message}" }
            }
        }
    }
}

#[component]
fn NavButton(route: Route) -> Element {
    rsx! {
        nav {
            id: "navbutton",
            class: "flex flex-wrap justify-center gap-2",
            aria_label: "Primary navigation",
            Link {
                to: Route::Monitor {},
                aria_current: if matches!(&route, Route::Monitor {}) { Some("page") } else { None },
                class: if matches!(&route, Route::Monitor {}) {
                    "btn btn-success"
                } else {
                    "btn"
                },
                "Monitor"
            }

            Link {
                to: Route::Analyze {},
                aria_current: if matches!(&route, Route::Analyze {}) { Some("page") } else { None },
                class: if matches!(&route, Route::Analyze {}) {
                    "btn btn-success"
                } else {
                    "btn"
                },
                "Analyze"
            }

            Link {
                to: Route::Settings {},
                aria_current: if matches!(&route, Route::Settings {}) { Some("page") } else { None },
                class: if matches!(&route, Route::Settings {}) {
                    "btn btn-success"
                } else {
                    "btn"
                },
                "Settings"
            }
        }
    }
}

#[component]
fn Home(route: Route, availability: RuntimeAvailability) -> Element {
    match route {
        Route::Settings {} => rsx! {
            Settings {}
        },
        Route::Monitor {} => match availability {
            RuntimeAvailability::Ready { camera_count } if camera_count > 0 => rsx! {
                Monitor {}
            },
            RuntimeAvailability::Ready { .. } => no_cameras(),
            RuntimeAvailability::SetupRequired => unavailable_route("Monitor", None),
            RuntimeAvailability::Failed { message } => unavailable_route("Monitor", Some(message)),
        },
        Route::Analyze {} => match availability {
            RuntimeAvailability::Ready { .. } => rsx! {
                Analyze {}
            },
            RuntimeAvailability::SetupRequired => unavailable_route("Analyze", None),
            RuntimeAvailability::Failed { message } => unavailable_route("Analyze", Some(message)),
        },
    }
}

fn no_cameras() -> Element {
    rsx! {
        section {
            class: "m-2 flex flex-col gap-3 rounded-box border border-base-300 p-5",
            aria_labelledby: "no-cameras-title",
            h1 {
                id: "no-cameras-title",
                class: "text-xl font-semibold",
                "No cameras are configured"
            }
            p { "Add a camera in Settings, save, then restart Leo before recording." }
            Link { class: "btn btn-primary self-start", to: Route::Settings {}, "Settings" }
        }
    }
}

fn unavailable_route(route_name: &'static str, failure: Option<String>) -> Element {
    rsx! {
        section {
            class: "m-2 flex flex-col gap-3 rounded-box border border-base-300 p-5",
            aria_labelledby: "unavailable-route-title",
            h1 {
                id: "unavailable-route-title",
                class: "text-xl font-semibold",
                "{route_name} is unavailable"
            }
            if let Some(message) = failure {
                div {
                    class: "alert alert-error",
                    role: "alert",
                    "Leo could not start: {message}"
                }
            }
            p { "Configure Leo, save, then restart to make this route available." }
            Link { class: "btn btn-primary self-start", to: Route::Settings {}, "Settings" }
        }
    }
}
