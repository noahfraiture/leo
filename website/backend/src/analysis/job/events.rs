use crate::{analysis::job::AnalysisJobError, db};

#[derive(Clone, Copy, Default)]
pub(super) struct EventNumbers {
    pub attempt: Option<i64>,
    pub attempts: Option<i64>,
    pub payload_bytes: Option<i64>,
    pub offset_bytes: Option<i64>,
    pub size_bytes: Option<i64>,
    pub duration_ms: Option<i64>,
}

pub(super) async fn record_analysis_event(
    db: &db::Database,
    analysis: &db::analysis::Analysis,
    stage: &str,
    level: &str,
    message: &str,
    numbers: EventNumbers,
) -> Result<(), AnalysisJobError> {
    db::analysis::AnalysisEvent::record(
        db,
        db::analysis::NewAnalysisEvent {
            analysis_key: analysis.key(),
            provider: analysis.provider.clone(),
            stage: stage.to_owned(),
            level: level.to_owned(),
            message: message.to_owned(),
            attempt: numbers.attempt,
            attempts: numbers.attempts,
            payload_bytes: numbers.payload_bytes,
            offset_bytes: numbers.offset_bytes,
            size_bytes: numbers.size_bytes,
            duration_ms: numbers.duration_ms,
        },
    )
    .await?;

    Ok(())
}
