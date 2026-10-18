use std::path::PathBuf;

use backend::analysis::RunStore;
use dioxus::prelude::*;
use uuid::Uuid;

use super::AnalysisContext;

/// Load a recording's history off the UI thread, without selecting a different recording or run.
pub fn load_session(
    mut context: AnalysisContext,
    session_id: Uuid,
    directory: PathBuf,
    refresh: bool,
) {
    let Some((id, generations)) =
        context
            .state
            .write()
            .begin_history_load(session_id, directory.clone(), refresh)
    else {
        return;
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result =
            tokio::task::spawn_blocking(move || RunStore::new(directory).history(session_id)).await;
        let mut page = context.state.write();
        let result = result
            .map_err(|_| "Could not load analysis history.".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
        page.finish_history_load(session_id, id, result, &generations);
    });
}
