use crate::components::sidebar;
use crate::{
    Route,
    views::{Analyze, Monitor, Sidebar},
};
use dioxus::prelude::*;

/// Layout is the main layout component that wraps the sidebar and the content.
#[component]
pub fn Layout() -> Element {
    let route = use_route::<Route>();
    rsx! {
        div {
            sidebar::SidebarProvider {
                sidebar::Sidebar {
                    side:  sidebar::SidebarSide::Left,
                    variant: sidebar::SidebarVariant::Sidebar,
                    sidebar::SidebarHeader {
                        NavButton { route: route.clone() },
                    },
                    sidebar::SidebarContent {
                        Sidebar { route: route.clone() },
                    },
                    sidebar::SidebarFooter { },
                }
                sidebar::SidebarInset {
                    Home { route: route },
                }
            }
        }
    }
}

#[component]
fn NavButton(route: Route) -> Element {
    rsx! {
        div {
            id: "navbutton",
            class: "flex gap-2",
            Link {
                to: Route::Monitor {},
                class: if matches!(&route, Route::Monitor {}) {
                    "btn btn-primary"
                } else {
                    "btn btn-secondary"
                },
                "Monitor"
            }

            Link {
                to: Route::Analyze {},
                class: if matches!(&route, Route::Analyze {}) {
                    "btn btn-primary"
                } else {
                    "btn btn-secondary"
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
