//! Background analysis job execution and provider dispatch.

use std::time::Instant;

use serde_json::json;

use crate::{
    analysis::{
        self as ai_analysis,
        job::{
            AnalysisJobError,
            events::{EventNumbers, record_analysis_event},
        },
        request::AnalysisTelemetry,
    },
    db,
    media::AnalysisVideo,
};

pub async fn run_analysis_job(
    db: &db::Database,
    analysis: &db::analysis::Analysis,
) -> Result<(), AnalysisJobError> {
    let started_at = Instant::now();
    let telemetry = AnalysisTelemetry::new(analysis.key(), analysis.provider.clone())
        .with_canary(analysis.is_canary);
    telemetry.log(
        "info",
        "analysis_job",
        "started",
        [("provider", json!(analysis.provider))],
    );
    analysis.mark_running(db).await?;
    record_analysis_event(
        db,
        analysis,
        "running",
        "info",
        "analysis started",
        EventNumbers::default(),
    )
    .await?;

    let mut videos = Vec::with_capacity(analysis.video_keys.len());
    for key in &analysis.video_keys {
        let Some(video) = db::video::Video::read_by_file_key(db, key).await? else {
            return Err(AnalysisJobError::BadRequest("selected video was not found"));
        };
        videos.push(AnalysisVideo {
            name: video.video.name,
            bytes: video.bytes,
        });
    }
    let total_video_bytes = videos.iter().map(|asset| asset.bytes.len() as i64).sum();
    record_analysis_event(
        db,
        analysis,
        "videos_loaded",
        "info",
        "selected videos loaded",
        EventNumbers {
            size_bytes: Some(total_video_bytes),
            ..EventNumbers::default()
        },
    )
    .await?;

    let provider = ai_analysis::provider_from_value(&analysis.provider)?;
    let response = ai_analysis::analyze_videos_with_telemetry(
        provider,
        videos,
        analysis.prompt.clone(),
        ai_analysis::request::AnalysisSettings {
            frame_sample_rate_fps: analysis.frame_sample_rate_fps,
        },
        telemetry,
    )
    .await?;
    analysis.complete(db, response).await?;
    record_analysis_event(
        db,
        analysis,
        "complete",
        "info",
        "analysis completed",
        EventNumbers {
            duration_ms: Some(started_at.elapsed().as_millis() as i64),
            ..EventNumbers::default()
        },
    )
    .await?;

    Ok(())
}
