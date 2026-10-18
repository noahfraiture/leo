use crate::views::SettingsContext;
use dioxus::prelude::*;

use super::numbers::NumberField;

/// Scheduling is expressed as one memory allowance and bounded request parallelism.
#[component]
pub fn Execution() -> Element {
    let mut state = use_context::<SettingsContext>().state;
    let settings = state.read().draft.workflow.clone();
    rsx! {
        div { class: "flex flex-col gap-2",
            h3 { class: "font-semibold", "4. Resources and evidence" }
            NumberField {
                name: "memory",
                label: "Memory available for models (MiB, maximum 49152)",
                value: Some(u64::from(settings.execution.memory_budget_mib)),
                minimum: 0,
                maximum: 49152,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.execution.memory_budget_mib = value as u32;
                    }
                },
            }
            p { class: "text-sm opacity-70",
                "Checklist review and analysis use one model at a time. Its estimated memory allowance must fit this budget; hosted models use zero local memory. Leave room for the operating system and other apps. Your local server must manage unused models; Leo cannot unload them."
            }
            NumberField {
                name: "concurrent",
                label: "Maximum simultaneous requests",
                value: Some(settings.execution.max_concurrent_jobs as u64),
                minimum: 1,
                maximum: 16,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.execution.max_concurrent_jobs = value as usize;
                    }
                },
            }
            NumberField {
                name: "per-model",
                label: "Requests per model",
                value: Some(settings.execution.max_jobs_per_model as u64),
                minimum: 1,
                maximum: 16,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.execution.max_jobs_per_model = value as usize;
                    }
                },
            }
            p { class: "text-xs opacity-70",
                "The current workflow sends one request at a time. These saved limits do not enable parallel analysis."
            }
            NumberField {
                name: "timeout",
                label: "Request timeout (seconds)",
                value: Some(settings.execution.request_timeout_seconds),
                minimum: 1,
                maximum: u64::MAX,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.execution.request_timeout_seconds = value;
                    }
                },
            }
            NumberField {
                name: "span",
                label: "Maximum span of a frame window (milliseconds)",
                value: Some(settings.evidence.max_prompt_span_ms),
                minimum: 1,
                maximum: u64::MAX,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.evidence.max_prompt_span_ms = value;
                    }
                },
            }
            NumberField {
                name: "overlap",
                label: "Overlapping timestamps between windows",
                value: Some(settings.evidence.overlap_frame_sets as u64),
                minimum: 0,
                maximum: usize::MAX as u64,
                onvalid: move |value| {
                    if let Some(value) = value {
                        state.write().draft.workflow.evidence.overlap_frame_sets = value as usize;
                    }
                },
            }
            p { class: "text-xs opacity-70",
                "References count toward each request's image limit. Sampling intervals and camera participation come from the recording."
            }
        }
    }
}
