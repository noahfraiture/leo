use crate::{
    Route,
    views::{Analyze, Monitor, Sidebar},
};
use dioxus::prelude::*;

#[component]
pub fn NavBar() -> Element {
    let route = use_route::<Route>();
    rsx! {
        div {
            NavButton {},
            Sidebar { route: route.clone() },
            Home { route: route },
        }
    }
}

#[component]
fn NavButton() -> Element {
    rsx! {
        div {
            id: "navbutton",
            Link {
                to: Route::Monitor {},
                "Monitor"
            }
            Link {
                to: Route::Analyze {},
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
