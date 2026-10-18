use super::{
    citations::{CitationLinks, citation_labels},
    correction::CorrectionEditor,
    labels::{judgment_label, timestamp},
};
use crate::{
    analysis::{self, AnalysisContext},
    inference::InferenceGate,
    media::MediaRegistry,
    operator::{OperatorState, SessionRunState},
    settings::ResolvedSettings,
};
use backend::analysis::{Finding, JobKind, JobOutput, RunSnapshot, RunStage};
use dioxus::prelude::*;
use uuid::Uuid;

/// Original candidate footage, preserved findings and operator correction for the selected step.
#[component]
pub fn EvidenceViewer(session_id: Uuid, run_id: Uuid) -> Element {
    let context = use_context::<AnalysisContext>();
    let gate = use_context::<InferenceGate>();
    let operator = use_context::<Signal<OperatorState>>();
    let recording = !matches!(operator.read().session, SessionRunState::Idle);
    let settings = use_context::<ResolvedSettings>();
    let history_loading = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .is_some_and(|session| session.loading);
    use_effect(move || analysis::load_references(context, session_id, run_id));
    let entry = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .cloned();
    let Some(entry) = entry else {
        return rsx! {};
    };
    let Some(criterion_id) = entry.selected_criterion else {
        return rsx! {};
    };
    let review_saving = entry.review_saving();
    let snapshot = entry.snapshot;
    let Some(criterion) = snapshot
        .inputs
        .exercise
        .checklist
        .criteria
        .iter()
        .find(|item| item.id == criterion_id)
    else {
        return rsx! {};
    };
    let assessment = snapshot
        .checkpoint
        .assessments
        .iter()
        .find(|item| item.criterion_id == criterion_id);
    let used_recheck = snapshot
        .checkpoint
        .jobs
        .iter()
        .any(|job| job.kind == JobKind::Recheck && job.criterion_id == Some(criterion_id));
    let recheck_disabled = used_recheck
        || entry.assessment_draft.saving
        || recording
        || history_loading
        || gate.active.read().is_some()
        || snapshot.checkpoint.stage != RunStage::Ready
        || assessment.is_none_or(|finding| finding.configuration_issue.is_some());
    let selected_frame = entry.playback.frame_id.clone();
    let reviewed = snapshot.is_reviewed(criterion_id);
    rsx! {
        // Replace every review control when selection changes, retaining each draft in run state.
        for criterion_id in [criterion_id] {
            section { key: "{criterion_id}", class: "flex min-w-0 flex-col gap-4 border-t border-base-300 pt-4", aria_label: "Evidence and human review",
                h3 { class: "font-semibold", "Inspect step: {criterion.text}" }
                CandidatePlayback { session_id, run_id, criterion_id, snapshot: snapshot.clone() }
                if let Some(assessment) = assessment {
                    div { class: "rounded-box bg-base-200 p-3 text-sm",
                        h4 { class: "font-semibold", "Current model result: {judgment_label(assessment.judgment)}" }
                        p { class: "mt-1", "{assessment.reason}" }
                        if let Some(issue) = &assessment.configuration_issue {
                            p { class: "text-warning mt-2", "Configuration issue: {issue}" }
                            p { "Clarify the exercise and start a new run. This is not a student failure." }
                        }
                    }
                    div { class: "grid gap-3 md:grid-cols-2",
                        FindingCard { title: "Initial assessment", finding: assessment.initial.clone(), snapshot: snapshot.clone(), session_id, run_id }
                        if let Some(finding) = &assessment.recheck {
                            FindingCard { title: "Independent closer look", finding: finding.clone(), snapshot: snapshot.clone(), session_id, run_id }
                        }
                    }
                    if let Some(note) = &assessment.recheck_note { p { class: "text-sm text-warning", "{note}" } }
                    button { id: "analysis-recheck", class: "btn btn-outline self-start", disabled: recheck_disabled,
                        onclick: {
                            let settings = settings.clone();
                            let selected_frame = selected_frame.clone();
                            move |_| analysis::recheck(context, operator, settings.clone(), gate, session_id, run_id, criterion_id, selected_frame.clone())
                        },
                        if used_recheck { "Closer look already used" } else { "Take one independent closer look" }
                    }
                    p { class: "text-xs opacity-70", "The closer look uses one episode around cited evidence, or the timestamp you selected above. It does not receive the initial verdict. Failed processing can be resumed." }
                }
                details { class: "rounded-box border border-base-300 p-3 text-sm",
                    summary { class: "font-semibold", "Observations from the scan" }
                    for response in &snapshot.responses {
                        if let JobOutput::Scan(completion) = &response.output {
                            for observation in completion.value.observations.iter().filter(|observation| observation.criterion_id == criterion_id.to_string()) {
                                div { class: "mt-3",
                                    p { "{observation.description}" }
                                    CitationLinks { session_id, run_id, citations: citation_labels(&snapshot, &observation.evidence_ids) }
                                }
                            }
                        }
                    }
                }
                section { aria_label: "Frozen reference examples",
                    h4 { class: "font-semibold", "Reference examples" }
                    p { class: "text-sm opacity-70", "Instructor examples are separate from candidate evidence. Reference pixels sent to the model: {snapshot.inputs.evidence_policy.reference_images}." }
                    if entry.references_loading { p { role: "status", "Loading references..." } }
                    if let Some(error) = &entry.reference_error { p { class: "text-error", "{error}" } }
                    div { class: "mt-3 flex flex-wrap gap-3",
                        for reference in snapshot.inputs.exercise.references.iter().filter(|reference| reference.criterion_ids.contains(&criterion_id)) {
                            figure { class: "w-48 rounded-box border border-base-300 p-3",
                                if let Some(preview) = entry.reference_previews.get(&reference.image.id) {
                                    img { class: "max-h-40 w-full object-contain", src: preview.clone(), alt: "Reference: {reference.object_label}" }
                                }
                                figcaption { class: "mt-2 text-sm",
                                    p { class: "font-semibold", "{reference.object_label}" }
                                    p { "{reference.state_label}" }
                                    p { "{reference.purpose}" }
                                }
                            }
                        }
                    }
                }
                CorrectionEditor { session_id, run_id, criterion_id, snapshot: snapshot.clone() }
                button { id: "analysis-mark-reviewed", class: "btn btn-outline self-start",
                    disabled: reviewed || assessment.is_none() || review_saving,
                    onclick: move |_| analysis::mark_reviewed(context, session_id, run_id, criterion_id),
                    if reviewed { "Reviewed by you" } else { "Mark this criterion reviewed" }
                }
            }
        }
    }
}

#[component]
fn FindingCard(
    title: &'static str,
    finding: Finding,
    snapshot: RunSnapshot,
    session_id: Uuid,
    run_id: Uuid,
) -> Element {
    rsx! {
        div { class: "rounded-box border border-base-300 p-3 text-sm",
            h4 { class: "font-semibold", "{title}: {judgment_label(finding.judgment)}" }
            p { class: "mt-2", "{finding.reason}" }
            CitationLinks { session_id, run_id, citations: citation_labels(&snapshot, &finding.evidence_ids) }
        }
    }
}
#[component]
fn CandidatePlayback(
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
    snapshot: RunSnapshot,
) -> Element {
    let context = use_context::<AnalysisContext>();
    let mut playback_state = context.state;
    let media = use_context::<MediaRegistry>();
    let playback = context
        .state
        .read()
        .sessions
        .get(&session_id)
        .and_then(|session| session.runs.get(&run_id))
        .map(|entry| entry.playback.clone())
        .unwrap_or_default();
    let citations = criterion_citations(&snapshot, criterion_id);
    let selected_frame = playback.frame_id.clone();
    let picker_media = media.clone();
    let before_media = media.clone();
    let after_media = media.clone();
    let before_frame = selected_frame.clone();
    let after_frame = selected_frame.clone();
    let before_ms = playback.before_ms;
    let after_ms = playback.after_ms;
    let request_id = playback.request_id;
    // Delayed metadata seeks can address only the player for their original request.
    let video_id = format!("analysis-evidence-video-{run_id}-{request_id:?}");
    rsx! {
        section { class: "min-w-0", aria_label: "Candidate recording playback",
            h4 { class: "font-semibold", "Candidate footage" }
            CitationLinks { session_id, run_id, citations }
            label { class: "form-control mt-3",
                span { class: "label-text mb-1", "Browse sampled candidate frames" }
                select {
                    class: "select select-bordered w-full",
                    value: selected_frame.clone().unwrap_or_default(),
                    onchange: move |event| {
                        let id = event.value();
                        if !id.is_empty() {
                            analysis::open_evidence(
                                context,
                                picker_media.clone(),
                                session_id,
                                run_id,
                                id,
                                5_000,
                                10_000,
                            );
                        }
                    },
                    option { value: "", "Choose a timestamp" }
                    for frame in &snapshot.checkpoint.evidence.frames {
                        option { key: "{frame.id}", value: "{frame.id}", selected: selected_frame.as_ref() == Some(&frame.id),
                            "Camera {frame.camera_id} · {timestamp(frame.session_offset_ms)}"
                        }
                    }
                }
            }
            if playback.loading {
                p { class: "mt-3", role: "status", "Preparing candidate footage..." }
            }
            if let Some(error) = playback.error {
                p { class: "alert alert-error mt-3", role: "alert", "{error}" }
            }
            if let (Some(clip), Some(url)) = (playback.clip, playback.url) {
                // Each selection resets playback even when its cached asset URL is reused.
                video {
                    key: "{request_id:?}",
                    id: video_id.clone(),
                    class: "mt-3 max-h-96 w-full rounded bg-black",
                    src: url.clone(),
                    controls: true,
                    preload: "metadata",
                    onerror: move |_| {
                        if let Some(entry) = playback_state
                            .write()
                            .sessions
                            .get_mut(&session_id)
                            .and_then(|session| session.runs.get_mut(&run_id))
                        {
                            if entry.playback.request_id == request_id {
                                entry.playback.error = Some(
                                    "The video player could not load or decode this clip.".into(),
                                );
                            }
                        }
                    },
                    onloadedmetadata: move |_| {
                        let script = format!(
                            "const video = document.getElementById({}); if (video && video.getAttribute('src') === {}) video.currentTime = {};",
                            serde_json::to_string(&video_id).expect("video ID serializes"),
                            serde_json::to_string(&url).expect("video URL serializes"),
                            clip.citation_offset_ms as f64 / 1000.0,
                        );
                        spawn(async move {
                            let _ = document::eval(&script).await;
                        });
                    },
                }
                p { class: "mt-2 text-sm",
                    "Camera {clip.camera_id}. Clip covers {timestamp(clip.session_start_ms)} to {timestamp(clip.session_start_ms + clip.duration_ms)} in the recording."
                }
                div { class: "mt-2 flex gap-2",
                    button {
                        class: "btn btn-outline btn-sm",
                        disabled: before_ms + after_ms >= 60_000,
                        onclick: move |_| {
                            if let Some(id) = before_frame.clone() {
                                analysis::open_evidence(
                                    context,
                                    before_media.clone(),
                                    session_id,
                                    run_id,
                                    id,
                                    (before_ms + 10_000).min(60_000 - after_ms),
                                    after_ms,
                                );
                            }
                        },
                        "More before"
                    }
                    button {
                        class: "btn btn-outline btn-sm",
                        disabled: before_ms + after_ms >= 60_000,
                        onclick: move |_| {
                            if let Some(id) = after_frame.clone() {
                                analysis::open_evidence(
                                    context,
                                    after_media.clone(),
                                    session_id,
                                    run_id,
                                    id,
                                    before_ms,
                                    (after_ms + 10_000).min(60_000 - before_ms),
                                );
                            }
                        },
                        "More after"
                    }
                }
                p { class: "mt-2 text-xs opacity-70",
                    "Playback stays within this recording segment and shows up to one minute of context."
                }
            }
        }
    }
}

fn criterion_citations(snapshot: &RunSnapshot, criterion_id: Uuid) -> Vec<(String, String)> {
    let mut ids = snapshot
        .checkpoint
        .assessments
        .iter()
        .filter(|assessment| assessment.criterion_id == criterion_id)
        .flat_map(|assessment| assessment.evidence_ids.clone())
        .collect::<Vec<_>>();
    for response in &snapshot.responses {
        if let JobOutput::Scan(completion) = &response.output {
            ids.extend(
                completion
                    .value
                    .observations
                    .iter()
                    .filter(|observation| observation.criterion_id == criterion_id.to_string())
                    .flat_map(|observation| observation.evidence_ids.clone()),
            );
        }
    }
    citation_labels(snapshot, &ids)
}
