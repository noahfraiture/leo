//! Root-scoped analysis execution and checkpoint projection.

use backend::analysis::AnalyzeSession;
use dioxus::prelude::{Signal, WritableExt};
use uuid::Uuid;

use super::OperatorState;

/// Runs one analysis independently of route lifetimes and projects durable snapshots.
pub fn spawn_analysis(
    mut operator: Signal<OperatorState>,
    request: AnalyzeSession,
    session_id: Uuid,
) {
    tracing::info!(%session_id, "analysis started");
    dioxus::dioxus_core::spawn_forever(async move {
        let mut checkpoint_operator = operator;
        let result = backend::analysis::analyze_session(request, move |checkpoint| {
            checkpoint_operator.write().apply_checkpoint(checkpoint);
        })
        .await;

        match result {
            Ok(checkpoint) => tracing::info!(
                %session_id,
                completed_batches = checkpoint.responses.len(),
                total_batches = checkpoint.total_batches,
                "analysis completed"
            ),
            Err(error) => {
                tracing::error!(%session_id, error = %error, "analysis failed");
                operator
                    .write()
                    .analysis_failed(session_id, error.to_string());
            }
        }
    });
}
