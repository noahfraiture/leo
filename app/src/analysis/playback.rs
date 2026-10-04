use backend::{
    analysis::{RunStore, prepare_playback},
    exercise::preview_reference_bytes,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use dioxus::prelude::*;
use uuid::Uuid;

use crate::media::MediaRegistry;

use super::AnalysisContext;

/// Resolve a registered candidate citation to a short playable clip. Slow older requests stay hidden.
pub fn open_evidence(
    mut context: AnalysisContext,
    media: MediaRegistry,
    session_id: Uuid,
    run_id: Uuid,
    frame_id: String,
    before_ms: u64,
    after_ms: u64,
) {
    let request_id = Uuid::new_v4();
    let (directory, manifest) = {
        let mut page = context.state.write();
        let Some(session) = page.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(entry) = session.runs.get_mut(&run_id) else {
            return;
        };
        entry.playback.frame_id = Some(frame_id.clone());
        entry.playback.request_id = Some(request_id);
        entry.playback.before_ms = before_ms;
        entry.playback.after_ms = after_ms;
        entry.playback.clip = None;
        entry.playback.url = None;
        entry.playback.loading = true;
        entry.playback.error = None;
        (
            session.directory.clone(),
            entry.snapshot.checkpoint.evidence.clone(),
        )
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = tokio::task::spawn_blocking(move || {
            let cache = RunStore::new(&directory)
                .directory(run_id)
                .map_err(|error| error.to_string())?
                .join("cache/playback");
            let clip = prepare_playback(
                &directory, &manifest, &frame_id, before_ms, after_ms, &cache,
            )
            .map_err(|error| error.to_string())?;
            let url = media.register(&clip.path)?;
            Ok::<_, String>((clip, url))
        })
        .await
        .map_err(|_| "Playback preparation could not finish.".to_owned())
        .and_then(|result| result);
        if let Some(entry) = context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.runs.get_mut(&run_id))
        {
            if entry.playback.request_id != Some(request_id) {
                return;
            }
            entry.playback.loading = false;
            match result {
                Ok((clip, url)) => {
                    entry.playback.clip = Some(clip);
                    entry.playback.url = Some(url);
                }
                Err(error) => entry.playback.error = Some(error),
            }
        }
    });
}

/// Thumbnails come from this run's frozen reference copies, never the currently edited library.
pub fn load_references(mut context: AnalysisContext, session_id: Uuid, run_id: Uuid) {
    let (directory, references) = {
        let mut page = context.state.write();
        let Some(session) = page.sessions.get_mut(&session_id) else {
            return;
        };
        let Some(entry) = session.runs.get_mut(&run_id) else {
            return;
        };
        if entry.references_loading
            || entry.reference_previews.len() == entry.snapshot.inputs.exercise.references.len()
        {
            return;
        }
        entry.references_loading = true;
        entry.reference_error = None;
        (
            session.directory.clone(),
            entry.snapshot.inputs.exercise.references.clone(),
        )
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = tokio::task::spawn_blocking(move || {
            let store = RunStore::new(directory);
            let snapshot = store.load(run_id).map_err(|error| error.to_string())?;
            let mut previews = std::collections::BTreeMap::new();
            for reference in references {
                let original = store
                    .read_reference(&snapshot, reference.image.id)
                    .map_err(|error| error.to_string())?;
                let preview =
                    preview_reference_bytes(&original, 320).map_err(|error| error.to_string())?;
                previews.insert(
                    reference.image.id,
                    format!("data:image/png;base64,{}", STANDARD.encode(preview.png)),
                );
            }
            Ok::<_, String>(previews)
        })
        .await
        .map_err(|_| "Reference images could not load.".to_owned())
        .and_then(|result| result);
        if let Some(entry) = context
            .state
            .write()
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.runs.get_mut(&run_id))
        {
            entry.references_loading = false;
            match result {
                Ok(previews) => entry.reference_previews = previews,
                Err(error) => entry.reference_error = Some(error),
            }
        }
    });
}
