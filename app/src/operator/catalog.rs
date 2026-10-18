//! Background session discovery with ownership separate from the Analyze route.

use backend::session::list_sessions;
use dioxus::prelude::*;

use super::OperatorState;

/// Refresh saved recordings without blocking UI callbacks or tying completion to navigation.
pub fn spawn_session_refresh(mut operator: Signal<OperatorState>) {
    let Some((id, root)) = operator.write().begin_session_refresh() else {
        return;
    };
    dioxus::dioxus_core::spawn_forever(async move {
        let result = tokio::task::spawn_blocking(move || list_sessions(&root))
            .await
            .map_err(|_| "Session discovery could not finish.".to_string())
            .and_then(|result| result.map_err(|error| error.to_string()));
        operator.write().finish_session_refresh(id, result);
    });
}
