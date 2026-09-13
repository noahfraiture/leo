use super::evaluation::Evaluation;
use crate::{
    analysis::{AnalysisContext, AnalysisTab},
    exercises::ExerciseEditor,
};
use dioxus::prelude::*;

/// Separates reusable exercise preparation from evaluating a selected recording.
#[component]
pub fn Analyze() -> Element {
    let AnalysisContext { mut state } = use_context::<AnalysisContext>();
    let tab = state.read().tab;
    let configuration_error = use_context::<crate::settings::ResolvedSettings>().analysis_error;
    rsx! {
        article { class: "m-2 flex min-w-0 flex-col gap-5",
            header {
                h1 { id: "analyze-title", class: "text-xl font-semibold", "Analyze exercises" }
                p { class: "mt-1 text-sm opacity-70",
                    "Prepare one checklist and its reference images, then evaluate recordings against that configuration."
                }
            }
            if let Some(error) = configuration_error {
                p { class: "alert alert-warning", role: "alert",
                    "Analysis settings need attention: {error}. Open Settings, correct the model configuration, save, and restart Leo."
                }
            }
            div {
                class: "tabs tabs-boxed self-start",
                role: "tablist",
                aria_label: "Analysis workflow",
                button {
                    id: "analysis-configure-tab",
                    role: "tab",
                    class: if tab == AnalysisTab::Configure { "tab tab-active" } else { "tab" },
                    aria_selected: tab == AnalysisTab::Configure,
                    onclick: move |_| state.write().tab = AnalysisTab::Configure,
                    "1. Exercise setup"
                }
                button {
                    id: "analysis-evaluate-tab",
                    role: "tab",
                    class: if tab == AnalysisTab::Evaluate { "tab tab-active" } else { "tab" },
                    aria_selected: tab == AnalysisTab::Evaluate,
                    onclick: move |_| state.write().tab = AnalysisTab::Evaluate,
                    "2. Evaluate recording"
                }
            }
            match tab {
                AnalysisTab::Configure => rsx! {
                    ExerciseEditor {}
                },
                AnalysisTab::Evaluate => rsx! {
                    Evaluation {}
                },
            }
        }
    }
}
