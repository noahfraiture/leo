use crate::{
    Route, RuntimeAvailability,
    operator::OperatorState,
    views::{Analyze, Monitor, Settings, Sidebar},
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
            Sidebar { route: route.clone() }
            main {
                id: "body",
                class: "min-w-0 flex-1 overflow-y-auto",
                if matches!(&availability, RuntimeAvailability::Ready { .. })
                    && !matches!(&route, Route::Settings {})
                {
                    OperatorAlert {}
                }
                RouteContent { route: route.clone(), availability }
            }
        }
    }
}

/// Reads ready-only operator state and renders its route-independent failure alert.
///
/// Keeping this context consumer behind a component boundary lets setup and failed shells render
/// the same layout without installing operational contexts.
#[component]
fn OperatorAlert() -> Element {
    let operator = use_context::<Signal<OperatorState>>();
    let message = operator.read().message.clone();

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
fn RouteContent(route: Route, availability: RuntimeAvailability) -> Element {
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
