use super::connections::Connections;
use super::policy::Execution;
use super::profiles::Models;
use super::sets::ModelSets;
use dioxus::prelude::*;

/// Configures connections and models, with resources and older saved sets kept alongside them.
#[component]
pub fn ModelSettingsSection() -> Element {
    rsx! {
        section { class: "rounded-box border border-base-300 p-4 flex flex-col gap-5",
            h2 { class: "text-xl font-semibold", "Analysis models" }
            p { class: "text-sm",
                "Name your connections and add their models. Choose a model in Analyze for checklist review or recording analysis."
            }
            Connections {}
            Models {}
            ModelSets {}
            Execution {}
        }
    }
}
