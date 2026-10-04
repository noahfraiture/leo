use backend::{
    analysis::EvidencePolicy,
    model::{ExecutionPolicy, ModelSnapshot},
    profiles::{ImageDetailPolicy, ImageSizePolicy},
};
use dioxus::prelude::*;

/// Inspectable limits from saved run inputs, independent of the current Settings draft.
#[component]
pub fn FrozenRunSettings(
    model: ModelSnapshot,
    execution: ExecutionPolicy,
    evidence: EvidencePolicy,
) -> Element {
    let rows = [(
        "Scan, consolidation and independent closer look",
        &model.profile,
    )];
    let image_size = match evidence.image_size {
        ImageSizePolicy::Original => "Original resolution".into(),
        ImageSizePolicy::MaximumLongEdge(pixels) => {
            format!("Maximum long edge: {pixels} px")
        }
    };
    let image_detail = match evidence.image_detail {
        ImageDetailPolicy::ProviderDefault => "Provider default",
        ImageDetailPolicy::Low => "Low",
        ImageDetailPolicy::High => "High",
    };
    rsx! {
        details {
            id: "analysis-frozen-settings",
            class: "rounded-box border border-base-300 p-3 text-sm",
            summary { class: "font-semibold", "Settings saved with this run" }
            p { class: "mt-3",
                "Resume keeps these models and limits. To use changed settings, start a new run after restarting Leo."
            }
            p { class: "mt-2",
                "Image limits include reference images when that option is enabled."
            }
            div { class: "mt-3 overflow-x-auto",
                table { class: "table table-sm",
                    thead {
                        tr {
                            th { "Role" }
                            th { "Model" }
                            th { "Images / request" }
                            th { "Output tokens / request" }
                            th { "Memory allowance" }
                            th { "Reasoning effort" }
                        }
                    }
                    tbody {
                        for (role, model) in rows {
                            tr {
                                td { "{role}" }
                                td {
                                    p { class: "font-medium", "{model.name}" }
                                    p { class: "break-all", "{model.model}" }
                                }
                                td { "{model.image_limit}" }
                                td {
                                    {
                                        model
                                            .max_output_tokens
                                            .map(|tokens| tokens.to_string())
                                            .unwrap_or_else(|| "Provider default".into())
                                    }
                                }
                                td { "{model.memory_cost_mib} MiB" }
                                td { {model.reasoning_effort.as_deref().unwrap_or("Provider default")} }
                            }
                        }
                    }
                }
            }
            div { class: "mt-3 flex flex-col gap-1",
                p {
                    "Window span: {evidence.max_window_span_ms} ms. Overlap: {evidence.overlap_frame_sets} frame sets."
                }
                p { "Images: {image_size}. Image detail: {image_detail}." }
                p {
                    "Memory budget: {execution.memory_budget_mib} MiB. Concurrent requests: {execution.max_concurrent_jobs}. Requests per model: {execution.max_jobs_per_model}."
                }
                p { "Request timeout: {execution.request_timeout_seconds} seconds." }
            }
        }
    }
}
