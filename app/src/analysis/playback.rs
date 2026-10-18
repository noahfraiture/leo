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
        // Repeated citation clicks must not launch duplicate hash reads and conversions.
        if entry.playback.loading
            && entry.playback.frame_id.as_ref() == Some(&frame_id)
            && entry.playback.before_ms == before_ms
            && entry.playback.after_ms == after_ms
        {
            return;
        }
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
    let expected_directory = directory.clone();
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
            .filter(|session| session.directory == expected_directory)
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

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::dioxus_core::NoOpMutations;
    use std::{cell::RefCell, rc::Rc};

    fn root(holder: Rc<RefCell<Option<AnalysisContext>>>) -> Element {
        super::super::use_analysis_context();
        *holder.borrow_mut() = Some(use_context::<AnalysisContext>());
        rsx! {}
    }

    #[test]
    fn playback_preparation_reuses_only_an_identical_pending_request() {
        for (frame, before, after, loading, duplicate) in [
            ("candidate-1", 5000, 10000, true, true),
            ("candidate-2", 5000, 10000, true, false),
            ("candidate-1", 15000, 10000, true, false),
            ("candidate-1", 5000, 20000, true, false),
            ("candidate-1", 5000, 10000, false, false),
        ] {
            let holder = Rc::new(RefCell::new(None));
            let mut dom = VirtualDom::new_with_props(root, holder.clone());
            dom.rebuild(&mut NoOpMutations);
            let mut context = holder.borrow().unwrap();
            let session_id = Uuid::new_v4();
            let run_id = Uuid::new_v4();
            let original_request = Uuid::new_v4();
            dom.in_runtime(|| {
                let mut page = context.state.write();
                page.apply_snapshot(super::super::test_snapshot(
                    session_id,
                    run_id,
                    backend::analysis::RunStage::Ready,
                ));
                let playback = &mut page
                    .sessions
                    .get_mut(&session_id)
                    .unwrap()
                    .runs
                    .get_mut(&run_id)
                    .unwrap()
                    .playback;
                playback.frame_id = Some("candidate-1".into());
                playback.request_id = Some(original_request);
                playback.before_ms = 5000;
                playback.after_ms = 10000;
                playback.loading = loading;
                playback.error = (!loading).then(|| "Previous attempt failed".into());
            });

            // Inspect admission before polling root tasks; no media or model I/O is needed.
            dom.in_runtime(|| {
                open_evidence(
                    context,
                    MediaRegistry::default(),
                    session_id,
                    run_id,
                    frame.into(),
                    before,
                    after,
                )
            });

            let page = context.state.read();
            let playback = &page.sessions[&session_id].runs[&run_id].playback;
            assert_eq!(
                playback.request_id == Some(original_request),
                duplicate,
                "frame={frame}, before={before}, after={after}, loading={loading}"
            );
            assert!(playback.loading);
            assert!(playback.error.is_none());
        }
    }
}
