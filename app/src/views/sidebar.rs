use crate::{
    Route, RuntimeAvailability,
    views::{SettingsSidebar, analyze, monitor},
};
use dioxus::prelude::*;

#[component]
pub fn Sidebar(route: Route) -> Element {
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
