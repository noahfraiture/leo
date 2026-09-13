use super::connections::Connections;
use super::policy::Execution;
use super::profiles::Models;
use super::sets::ModelSets;
use dioxus::prelude::*;

/// Keeps model setup in one ordered flow: connection, model, model set, resource allowance.
#[component]
pub fn ModelSettingsSection() -> Element {
    rsx! {
        section { class: "rounded-box border border-base-300 p-4 flex flex-col gap-5",
            h2 { class: "text-xl font-semibold", "Analysis models" }
            p { class: "text-sm",
                "Name your connections, add their models, then choose independent readers and a reviewer."
            }
            Connections {}
            Models {}
            ModelSets {}
            Execution {}
        }
    }
}
