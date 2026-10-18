use std::path::PathBuf;

use backend::analysis::RunStore;
use dioxus::prelude::*;
use uuid::Uuid;

use super::{AnalysisContext, SessionAnalyses};

/// Load a recording's history off the UI thread, without selecting a different recording or run.
pub fn load_session(
    mut context: AnalysisContext,
    session_id: Uuid,
    directory: PathBuf,
    refresh: bool,
) {
    let generations = {
        let mut page = context.state.write();
        let session = page.sessions.entry(session_id).or_default();
        if session.loading || (session.loaded && !refresh) {
            return;
        }
        session.directory = directory.clone();
        session.loading = true;
        session.error = None;
        session
            .runs
            .iter()
            .map(|(id, entry)| (*id, entry.snapshot_generation))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result =
            tokio::task::spawn_blocking(move || RunStore::new(directory).history(session_id)).await;
        let mut page = context.state.write();
        let result = result
            .map_err(|_| "Could not load analysis history.".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
        match result {
            Ok(history) => {
                let runs = history.runs;
                let newest = runs.first().map(|run| run.inputs.id);
                for run in runs {
                    let generation = generations.get(&run.inputs.id).copied();
                    page.apply_history_snapshot(run, generation);
                }
                if let Some(session) = page.sessions.get_mut(&session_id) {
                    if session.selected_run_id.is_none() {
                        session.selected_run_id = newest;
                    }
                    session.archived = history.archived;
                    session.loaded = true;
                    session.loading = false;
                }
            }
            Err(error) => {
                let session = page
                    .sessions
                    .entry(session_id)
                    .or_insert_with(SessionAnalyses::default);
                session.loading = false;
                session.error = Some(error);
            }
        }
    });
}
