use dioxus::prelude::*;

use super::state::SettingsContext;

/// Renders a small settings-only summary for the route sidebar.
#[component]
pub fn SettingsSidebar() -> Element {
    let SettingsContext { state, .. } = use_context::<SettingsContext>();
    let (camera_count, status) = {
        let page = state.read();
        let status = if page.saving {
            "Saving settings..."
        } else if page.has_unsaved_changes() {
            "Unsaved changes"
        } else if page.restart_required {
            "Restart required"
        } else {
            "Ready to save"
        };
        (page.draft.cameras.len(), status)
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
            p { class: "text-sm", "{status}" }
        }
    }
}
