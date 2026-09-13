use std::collections::HashSet;

use super::labels::timestamp;
use crate::{
    analysis::{self, AnalysisContext},
    media::MediaRegistry,
};
use backend::analysis::RunSnapshot;
use dioxus::prelude::*;
use uuid::Uuid;

/// Opens one registered candidate citation in the current run's playback viewer.
#[component]
pub fn EvidenceButton(session_id: Uuid, run_id: Uuid, frame_id: String, label: String) -> Element {
    let context = use_context::<AnalysisContext>();
    let media = use_context::<MediaRegistry>();
    rsx! { button { class: "btn btn-outline btn-sm", "data-evidence-id": frame_id.clone(),
        onclick: move |_| analysis::open_evidence(context, media.clone(), session_id, run_id, frame_id.clone(), 5_000, 10_000), "{label}"
    } }
}

/// The same candidate links are used for model findings and preserved human corrections.
#[component]
pub fn CitationLinks(session_id: Uuid, run_id: Uuid, citations: Vec<(String, String)>) -> Element {
    rsx! { div { class: "mt-2 flex max-h-40 flex-wrap gap-2 overflow-y-auto",
        for (id, label) in citations { EvidenceButton { key: "{id}", session_id, run_id, frame_id: id, label } }
    } }
}

/// Resolve unique candidate IDs in recording-time and camera order, excluding unknown IDs.
pub fn citation_labels(snapshot: &RunSnapshot, ids: &[String]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut frames: Vec<_> = ids
        .iter()
        .filter_map(|id| snapshot.checkpoint.evidence.frame(id))
        .filter(|frame| seen.insert(&frame.id))
        .collect();
    frames.sort_by_key(|frame| (frame.session_offset_ms, frame.camera_id));
    frames
        .into_iter()
        .map(|frame| {
            (
                frame.id.clone(),
                format!(
                    "Camera {} · {}",
                    frame.camera_id,
                    timestamp(frame.session_offset_ms)
                ),
            )
        })
        .collect()
}
