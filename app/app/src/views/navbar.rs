use crate::{
    Route,
    views::{Analyze, Monitor, Sidebar},
    workflow::Workflow,
};
use dioxus::prelude::*;

/// Wraps route navigation, route-specific controls, shared messages, and content.
#[component]
pub fn Layout() -> Element {
    let route = use_route::<Route>();
    let workflow = use_context::<Signal<Workflow>>();
    let message = workflow.read().message.clone();

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
                if let Some(message) = message {
                    div {
                        class: "alert alert-error m-2",
                        role: "alert",
                        aria_live: "assertive",
                        span { "{message}" }
                    }
                }
                Home { route: route.clone() }
            }
        }
    }
}

#[component]
fn NavButton(route: Route) -> Element {
    rsx! {
        nav {
            id: "navbutton",
            class: "flex gap-2 justify-center",
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
        }
    }
}

#[component]
fn Home(route: Route) -> Element {
    match route {
        Route::Monitor {} => rsx! {
            Monitor { }
        },
        Route::Analyze {} => rsx! {
            Analyze { }
        },
    }
}
