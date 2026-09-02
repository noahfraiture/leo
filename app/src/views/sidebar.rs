//! Persistent navigation and route-specific operator controls.

use crate::{
    Route, RuntimeAvailability,
    views::{SettingsSidebar, analyze, monitor},
};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(route: Route) -> Element {
    rsx! {
        aside {
            id: "sidebar",
            class: "flex shrink-0 flex-col bg-base-200 p-3 lg:w-80 lg:overflow-y-auto",
            div {
                class: "shrink-0",
                PrimaryNavigation { route: route.clone() }
            }

            hr {
                class: "border-t shrink-0 my-4",
            }

            div {
                class: "min-h-0 flex-1 overflow-y-auto",
                RoutePanel { route }
            }
        }
    }
}

#[component]
fn PrimaryNavigation(route: Route) -> Element {
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
fn RoutePanel(route: Route) -> Element {
    let availability = use_context::<RuntimeAvailability>();
    match (route, availability) {
        (Route::Monitor {}, RuntimeAvailability::Ready { camera_count }) if camera_count > 0 => {
            rsx! { monitor::Sidebar {} }
        }
        (Route::Analyze {}, RuntimeAvailability::Ready { .. }) => rsx! { analyze::Sidebar {} },
        (Route::Settings {}, _) | (Route::Monitor {}, _) | (Route::Analyze {}, _) => {
            rsx! { SettingsSidebar {} }
        }
    }
}
