use crate::{Route, views::analyze, views::monitor};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(route: Route) -> Element {
    match route {
        Route::Monitor {} => monitor::Sidebar(),
        Route::Analyze {} => analyze::Sidebar(),
    }
}
