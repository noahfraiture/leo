use crate::{Route, views::analyze, views::monitor};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(route: Route) -> Element {
    match route {
        Route::Monitor {} => rsx! { monitor::Sidebar {} },
        Route::Analyze {} => rsx! { analyze::Sidebar {} },
    }
}
