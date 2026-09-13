use super::citations::{CitationLinks, citation_labels};
use super::labels::{judgment_label, recorded_at, timestamp};
use crate::analysis::{self, AnalysisContext};
use backend::analysis::{Judgment, RunSnapshot};
use dioxus::prelude::*;
use uuid::Uuid;

/// Human judgments append to a separate history and always leave the model's assessment visible.
#[component]
pub fn CorrectionEditor(
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
    snapshot: RunSnapshot,
) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut state = context.state;
    let (draft, current_frame) = state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .map(|entry| {
            (
                entry
                    .corrections
                    .get(&criterion_id)
                    .cloned()
                    .unwrap_or_default(),
                entry.playback.frame_id.clone(),
            )
        })
        .unwrap_or_default();
    let has_current = current_frame
        .as_ref()
        .is_some_and(|id| !draft.evidence_ids.contains(id));
    let selected_value = match draft.judgment {
        Judgment::Met => "met",
        Judgment::NotMet => "notMet",
        Judgment::CannotVerify => "cannotVerify",
    };
    let saved = snapshot
        .corrections
        .iter()
        .filter(|correction| correction.criterion_id == criterion_id)
        .collect::<Vec<_>>();
    rsx! {
        section {
            class: "rounded-box border border-base-300 p-4",
            aria_label: "Operator correction",
            h4 { class: "font-semibold", "Your assessment" }
            p { class: "mt-1 text-sm opacity-70",
                "Add a correction with your reason. The model result stays visible and your decision is saved separately."
            }
            div { class: "mt-3 flex flex-col gap-3",
                label { class: "form-control",
                    span { class: "label-text mb-1", "Your judgment" }
                    select {
                        id: "analysis-correction-judgment",
                        class: "select select-bordered w-full",
                        value: selected_value,
                        onchange: move |event| {
                            state
                                .write()
                                .edit_correction(
                                    session_id,
                                    run_id,
                                    criterion_id,
                                    |draft| {
                                        draft.judgment = match event.value().as_str() {
                                            "met" => Judgment::Met,
                                            "notMet" => Judgment::NotMet,
                                            _ => Judgment::CannotVerify,
                                        };
                                    },
                                )
                        },
                        option { value: "met", "Met" }
                        option { value: "notMet", "Not met" }
                        option { value: "cannotVerify", "Cannot verify" }
                    }
                }
                label { class: "form-control",
                    span { class: "label-text mb-1", "Reason for your correction" }
                    textarea {
                        id: "analysis-correction-reason",
                        class: "textarea textarea-bordered w-full",
                        rows: 3,
                        value: draft.reason.clone(),
                        oninput: move |event| {
                            state
                                .write()
                                .edit_correction(
                                    session_id,
                                    run_id,
                                    criterion_id,
                                    |draft| draft.reason = event.value(),
                                )
                        },
                    }
                }
                label { class: "flex items-start gap-2 text-sm",
                    input {
                        class: "checkbox checkbox-sm",
                        r#type: "checkbox",
                        checked: draft.outside_information,
                        onchange: move |event| {
                            state
                                .write()
                                .edit_correction(
                                    session_id,
                                    run_id,
                                    criterion_id,
                                    |draft| draft.outside_information = event.checked(),
                                )
                        },
                    }
                    span { "This correction uses information outside the video." }
                }
                button {
                    class: "btn btn-outline btn-sm self-start",
                    disabled: !has_current,
                    onclick: move |_| {
                        if let Some(id) = current_frame.clone() {
                            state
                                .write()
                                .edit_correction(
                                    session_id,
                                    run_id,
                                    criterion_id,
                                    |draft| {
                                        if !draft.evidence_ids.contains(&id) {
                                            draft.evidence_ids.push(id);
                                        }
                                    },
                                );
                        }
                    },
                    "Attach selected timestamp"
                }
                div { class: "flex flex-wrap gap-2",
                    for id in draft.evidence_ids {
                        CorrectionCitation {
                            session_id,
                            run_id,
                            criterion_id,
                            label: snapshot
                                .checkpoint
                                .evidence
                                .frame(&id)
                                .map(|frame| {
                                    format!(
                                        "Camera {} · {}",
                                        frame.camera_id,
                                        timestamp(frame.session_offset_ms),
                                    )
                                })
                                .unwrap_or_else(|| "Unknown evidence".into()),
                            frame_id: id,
                        }
                    }
                }
                if let Some(error) = draft.error {
                    p { class: "alert alert-error", role: "alert", "{error}" }
                }
                button {
                    id: "analysis-save-correction",
                    class: "btn btn-primary self-start",
                    disabled: draft.saving || draft.reason.trim().is_empty(),
                    onclick: move |_| analysis::save_correction(context, session_id, run_id, criterion_id),
                    if draft.saving {
                        "Saving correction..."
                    } else {
                        "Save my assessment"
                    }
                }
            }
            if !saved.is_empty() {
                details { class: "mt-4", open: true,
                    summary { class: "font-semibold", "Saved correction history" }
                    ol { class: "mt-2 flex flex-col gap-2",
                        for correction in saved {
                            li { class: "rounded bg-base-200 p-3 text-sm",
                                p { class: "font-semibold",
                                    "{judgment_label(correction.judgment)} · {recorded_at(correction.created_utc_ms as i64)}"
                                }
                            p { class: "mt-1", "{correction.reason}" }
                            CitationLinks { session_id, run_id, citations: citation_labels(&snapshot, &correction.evidence_ids) }
                                if correction.uses_outside_information {
                                    p { class: "mt-1 text-warning",
                                        "Uses information outside the video."
                                    }
                                }
                                if correction.supersedes.is_some() {
                                    p { class: "mt-1 text-xs opacity-70",
                                        "Replaces an earlier human judgment; history retained."
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn CorrectionCitation(
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
    frame_id: String,
    label: String,
) -> Element {
    let AnalysisContext { mut state } = use_context::<AnalysisContext>();
    rsx! {
        button {
            class: "badge badge-outline h-auto gap-2 p-2",
            aria_label: "Remove correction citation {label}",
            onclick: move |_| {
                state
                    .write()
                    .edit_correction(
                        session_id,
                        run_id,
                        criterion_id,
                        |draft| draft.evidence_ids.retain(|id| id != &frame_id),
                    )
            },
            "{label} ×"
        }
    }
}
