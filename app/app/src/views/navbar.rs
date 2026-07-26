use crate::{
    Route,
    views::{Analyze, Monitor, Sidebar},
};
use dioxus::prelude::*;

/// Layout is the main layout component that wraps the sidebar and the content.
// The idea is to have a layout like
// | nav button      | page content
// |                 | page content
// | sidebar content | page content
// | sidebar content | page content
// | sidebar content | page content
#[component]
pub fn Layout() -> Element {
    let route = use_route::<Route>();
    rsx! {
        div {
            class: "flex h-screen p-2",
            div { // fixed size in width
                id: "sidebar",
                class: "flex w-64 shrink-0 flex-col p-2 bg-base-300 card",
                div { // fixed size in heigh
                    class: "shrink-0",
                    NavButton { route: route.clone() }
                }

                hr {
                    class: "border-t shrink-0 my-4",
                }

                div { // rest of the heigh of the sidebar
                    class: "min-h-0 flex-1",
                    Sidebar { route: route.clone() }
                }

                hr {
                    class: "border-t shrink-0 my-4",
                }

                div {
                    class: "shrink-0",
                    Settings { }
                }
            }
            div { // rest of the width of the screen
                id: "body",
                class: "min-w-0 flex-1",
                Home { route: route.clone() }
            }
        }
    }
}

#[component]
fn NavButton(route: Route) -> Element {
    rsx! {
        div {
            id: "navbutton",
            class: "flex gap-2 justify-center",
            Link {
                to: Route::Monitor {},
                class: if matches!(&route, Route::Monitor {}) {
                    "btn btn-success"
                } else {
                    "btn"
                },
                "Monitor"
            }

            Link {
                to: Route::Analyze {},
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

#[component]
fn Settings() -> Element {
    rsx! {
        div {
            class: "btn btn-primary",
            "Settings"
        }
    }
}
