use std::collections::BTreeSet;

use super::{
    citations::{CitationLinks, EvidenceButton, citation_labels},
    correction::CorrectionEditor,
    labels::{judgment_label, timestamp},
};
use crate::{
    analysis::{self, AnalysisContext},
    media::MediaRegistry,
};
use backend::analysis::{ReviewDispositionKind, RunSnapshot};
use dioxus::prelude::*;
use uuid::Uuid;

/// Candidate playback, independent observations, and reference examples for one selected criterion.
#[component]
pub fn EvidenceViewer(session_id: Uuid, run_id: Uuid) -> Element {
    let context = use_context::<AnalysisContext>();
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
    let snapshot = &entry.snapshot;
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
    rsx! {
        section {
            class: "flex min-w-0 flex-col gap-4 border-t border-base-300 pt-4",
            aria_label: "Evidence and human review",
            h3 { class: "font-semibold", "Inspect step: {criterion.text}" }
            CandidatePlayback {
                session_id,
                run_id,
                criterion_id,
                snapshot: snapshot.clone(),
            }
            if let Some(assessment) = assessment {
                div { class: "rounded-box bg-base-200 p-3 text-sm",
                    h4 { class: "font-semibold",
                        "Consolidated assessment: {judgment_label(assessment.judgment)}"
                    }
                    p { class: "mt-1", "{assessment.reason}" }
                    if !assessment.coverage_note.is_empty() {
                        p { class: "mt-2", "Coverage: {assessment.coverage_note}" }
                    }
                    for fact in &assessment.unresolved_facts {
                        p { class: "mt-1 text-warning", "Unresolved: {fact}" }
                    }
                    if assessment.additional_evidence_round_used {
                        p { class: "mt-2", "An additional evidence round was used." }
                    }
                    if let Some(job_id) = &assessment.final_job_id {
                        p { class: "mt-2", "Final visual comparison: {job_window(snapshot, job_id)}" }
                    } else {
                        p { class: "mt-2",
                            "The required visual comparison could not be completed. The result remains Cannot verify."
                        }
                    }
                }
            }
            Findings {
                session_id,
                run_id,
                criterion_id,
                snapshot: snapshot.clone(),
            }
            SelectionReasoning {
                session_id,
                run_id,
                criterion_id,
                snapshot: snapshot.clone(),
            }
            section { aria_label: "Frozen reference examples",
                h4 { class: "font-semibold", "Reference examples used for this run" }
                p { class: "mt-1 text-sm opacity-70",
                    "These examples explain objects and states. They do not show the candidate's performance."
                }
                if entry.references_loading {
                    p { role: "status", "Loading references..." }
                }
                if let Some(error) = entry.reference_error {
                    p { class: "text-error text-sm", "{error}" }
                }
                div { class: "mt-3 flex flex-wrap gap-3",
                    for reference in snapshot
                        .inputs
                        .exercise
                        .references
                        .iter()
                        .filter(|reference| reference.criterion_ids.contains(&criterion_id))
                    {
                        figure { class: "w-48 rounded-box border border-base-300 p-3",
                            if let Some(preview) = entry.reference_previews.get(&reference.image.id) {
                                img {
                                    class: "max-h-40 w-full object-contain",
                                    src: preview.clone(),
                                    alt: "Reference: {reference.object_label}",
                                }
                            }
                            figcaption { class: "mt-2 text-sm",
                                p { class: "font-semibold", "{reference.object_label}" }
                                p { "{reference.state_label}" }
                                p { class: "mt-1", "{reference.purpose}" }
                            }
                        }
                    }
                }
            }
            CorrectionEditor {
                session_id,
                run_id,
                criterion_id,
                snapshot: snapshot.clone(),
            }
        }
    }
}

#[component]
fn SelectionReasoning(
    session_id: Uuid,
    run_id: Uuid,
    criterion_id: Uuid,
    snapshot: RunSnapshot,
) -> Element {
    let selection = snapshot.selection_results.iter().find(|result| {
        snapshot
            .checkpoint
            .jobs
            .iter()
            .any(|job| job.id == result.job_id && job.criterion_id == criterion_id)
    });
    let Some(selection) = selection else {
        return rsx! {};
    };
    rsx! {
        details { class: "rounded-box border border-base-300 p-3 text-sm",
            summary { class: "font-semibold", "Why these images were chosen for the final comparison" }
            p { class: "mt-3 opacity-70",
                "This planning step reads the saved reviews. The final visual reviewer inspects the chosen images."
            }
            p { class: "mt-2", "{selection.completion.value.reason}" }
            for disposition in &selection.completion.value.reviews {
                div { class: "mt-3 border-t border-base-300 pt-2",
                    p { class: "font-semibold", "{disposition_label(disposition.kind)}" }
                    p { "{disposition.reason}" }
                    if disposition.required_for_decision {
                        p { class: "mt-1", "Required to decide this step." }
                    }
                    CitationLinks {
                        session_id,
                        run_id,
                        citations: citation_labels(&snapshot, &disposition.decisive_ids),
                    }
                }
            }
        }
    }
}

fn disposition_label(kind: ReviewDispositionKind) -> &'static str {
    match kind {
        ReviewDispositionKind::Supporting => "Supporting evidence",
        ReviewDispositionKind::Contradictory => "Contradicting evidence",
        ReviewDispositionKind::Irrelevant => "Not relevant to this decision",
        ReviewDispositionKind::Unresolved => "Unresolved evidence",
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
    let video_id = format!("analysis-evidence-video-{run_id}");
    rsx! {
        section { class: "min-w-0", aria_label: "Candidate recording playback",
            h4 { class: "font-semibold", "Candidate footage" }
            div { class: "mt-2 flex flex-wrap gap-2",
                for id in citations {
                    if let Some(frame) = snapshot.checkpoint.evidence.frame(&id) {
                        EvidenceButton {
                            session_id,
                            run_id,
                            frame_id: id.clone(),
                            label: format!("Camera {} · {}", frame.camera_id, timestamp(frame.session_offset_ms)),
                        }
                    }
                }
            }
            label { class: "form-control mt-3",
                span { class: "label-text mb-1", "Browse sampled candidate frames" }
                select {
                    class: "select select-bordered w-full",
                    value: selected_frame.unwrap_or_default(),
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
                        option { value: "{frame.id}",
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
                video {
                    key: "{url}",
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

fn criterion_citations(snapshot: &RunSnapshot, criterion_id: Uuid) -> BTreeSet<String> {
    let jobs = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.criterion_id == criterion_id)
        .map(|job| job.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut citations = snapshot
        .checkpoint
        .assessments
        .iter()
        .filter(|assessment| assessment.criterion_id == criterion_id)
        .flat_map(|assessment| assessment.evidence_ids.clone())
        .collect::<BTreeSet<_>>();
    for result in &snapshot.reader_results {
        if jobs.contains(result.job_id.as_str()) {
            citations.extend(result.completion.value.evidence_ids.clone());
        }
    }
    for result in &snapshot.review_results {
        if jobs.contains(result.job_id.as_str()) {
            citations.extend(result.completion.value.evidence_ids.clone());
        }
    }
    citations
}

#[component]
fn Findings(session_id: Uuid, run_id: Uuid, criterion_id: Uuid, snapshot: RunSnapshot) -> Element {
    let jobs = snapshot
        .checkpoint
        .jobs
        .iter()
        .filter(|job| job.criterion_id == criterion_id)
        .collect::<Vec<_>>();
    rsx! {
        details { class: "rounded-box border border-base-300 p-3",
            summary { class: "font-semibold", "Independent findings and evidence reviews" }
            div { class: "mt-3 flex flex-col gap-3",
                for result in snapshot
                    .reader_results
                    .iter()
                    .filter(|result| jobs.iter().any(|job| job.id == result.job_id))
                {
                    div { class: "rounded bg-base-200 p-3 text-sm",
                        p { class: "font-semibold",
                            "Reader: {jobs.iter().find(|job| job.id == result.job_id).map(|job| model_name(&snapshot, &job.profile_id)).unwrap_or_default()} · {judgment_label(result.completion.value.judgment)}"
                        }
                        p { class: "mt-1 text-xs opacity-70",
                            "Input window: {job_window(&snapshot, &result.job_id)}"
                        }
                        p { class: "mt-1", "{result.completion.value.reason}" }
                        CitationLinks {
                            session_id,
                            run_id,
                            citations: citation_labels(&snapshot, &result.completion.value.evidence_ids),
                        }
                        for fact in &result.completion.value.facts {
                            div { class: "mt-3 border-t border-base-300 pt-2",
                                p { "Observed: {fact.description}" }
                                if let Some(attempt) = &fact.attempt {
                                    p { class: "mt-1 text-xs", "Attempt: {attempt}" }
                                }
                                if let Some(identity) = &fact.identity {
                                    p { class: "mt-1 text-xs", "Object or identity: {identity}" }
                                }
                                CitationLinks {
                                    session_id,
                                    run_id,
                                    citations: citation_labels(&snapshot, &fact.evidence_ids),
                                }
                            }
                        }
                    }
                }
                for result in snapshot
                    .review_results
                    .iter()
                    .filter(|result| jobs.iter().any(|job| job.id == result.job_id))
                {
                    div { class: "rounded border border-base-300 p-3 text-sm",
                        p { class: "font-semibold",
                            "Evidence reviewer · {judgment_label(result.completion.value.judgment)}"
                        }
                        p { class: "mt-1 text-xs opacity-70",
                            "Input window: {job_window(&snapshot, &result.job_id)}"
                        }
                        p { class: "mt-1", "{result.completion.value.reason}" }
                        CitationLinks {
                            session_id,
                            run_id,
                            citations: citation_labels(&snapshot, &result.completion.value.evidence_ids),
                        }
                        for fact in &result.completion.value.facts {
                            div { class: "mt-3 border-t border-base-300 pt-2",
                                p { "Observed: {fact.description}" }
                                if let Some(attempt) = &fact.attempt {
                                    p { class: "mt-1 text-xs", "Attempt: {attempt}" }
                                }
                                if let Some(identity) = &fact.identity {
                                    p { class: "mt-1 text-xs", "Object or identity: {identity}" }
                                }
                                CitationLinks {
                                    session_id,
                                    run_id,
                                    citations: citation_labels(&snapshot, &fact.evidence_ids),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn job_window(snapshot: &RunSnapshot, job_id: &str) -> String {
    let Some(job) = snapshot.checkpoint.jobs.iter().find(|job| job.id == job_id) else {
        return "Unavailable".into();
    };
    let times = job
        .frame_ids
        .iter()
        .filter_map(|id| {
            snapshot
                .checkpoint
                .evidence
                .frame(id)
                .map(|frame| frame.session_offset_ms)
        })
        .collect::<Vec<_>>();
    match (times.iter().min(), times.iter().max()) {
        (Some(start), Some(end)) => format!("{} to {}", timestamp(*start), timestamp(*end)),
        _ => "No candidate frames".into(),
    }
}

fn model_name(snapshot: &RunSnapshot, profile_id: &str) -> String {
    snapshot
        .inputs
        .model_set
        .readers
        .iter()
        .chain(std::iter::once(&snapshot.inputs.model_set.reviewer))
        .find(|model| model.profile.id == profile_id)
        .map(|model| model.profile.name.clone())
        .unwrap_or_else(|| profile_id.into())
}
