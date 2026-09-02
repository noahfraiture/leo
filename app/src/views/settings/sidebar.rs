use dioxus::prelude::*;

use super::state::SettingsContext;

/// Renders a small settings-only summary for the route sidebar.
#[component]
pub fn SettingsSidebar() -> Element {
    let SettingsContext { state, .. } = use_context::<SettingsContext>();
    let (camera_count, restart_required) = {
        let page = state.read();
        (page.draft.cameras.len(), page.restart_required)
    };

    rsx! {
        section {
            class: "flex flex-col gap-3",
            aria_labelledby: "settings-sidebar-title",
            h2 {
                id: "settings-sidebar-title",
                class: "text-lg font-semibold",
                "Settings"
            }
            p { class: "text-sm", "Configured cameras: {camera_count}" }
            p { class: "text-sm",
                if restart_required {
                    "Restart required"
                } else {
                    "Ready to save"
                }
            }
        }
    }
}
