use serde_json::json;
use tempfile::Builder;
use tokio::{fs, process::Command, time::sleep};

use crate::{
    analysis::{
        canary::{CANARY_VIDEO_NAME, CanaryConfig},
        provider::AnalysisProvider,
        request::AnalysisSettings,
    },
    db,
    http::{router::AppState, ui},
};

pub fn spawn_canary(state: AppState) {
    let config = match CanaryConfig::from_env() {
        Ok(config) if config.enabled => config,
        Ok(_) => return,
        Err(error) => {
            eprintln!(
                "{}",
                json!({
                    "level": "error",
                    "component": "canary",
                    "event": "config_failed",
                    "error": error.to_string(),
                })
            );
            return;
        }
    };

    tokio::spawn(async move {
        run_once(&state, &config).await;

        if let Some(interval) = config.interval() {
            loop {
                sleep(interval).await;
                run_once(&state, &config).await;
            }
        }
    });
}

async fn run_once(state: &AppState, config: &CanaryConfig) {
    let video = match ensure_canary_video(state.db()).await {
        Ok(video) => video,
        Err(error) => {
            state
                .metrics()
                .increment("leo_canary_runs_total", &[("result", "setup_failed")]);
            eprintln!(
                "{}",
                json!({
                    "level": "error",
                    "component": "canary",
                    "event": "setup_failed",
                    "error": error.to_string(),
                })
            );
            return;
        }
    };

    for provider in &config.providers {
        let provider = match provider.parse::<AnalysisProvider>() {
            Ok(provider) => provider,
            Err(error) => {
                eprintln!(
                    "{}",
                    json!({
                        "level": "error",
                        "component": "canary",
                        "event": "provider_parse_failed",
                        "provider": provider,
                        "error": error.to_string(),
                    })
                );
                continue;
            }
        };
        match db::analysis::Analysis::create_with_provider_and_settings(
            state.db(),
            provider,
            AnalysisSettings {
                frame_sample_rate_fps: 1.0,
            },
            config.prompt.clone(),
            vec![video.file.key().to_owned()],
        )
        .await
        {
            Ok(analysis) => {
                let _ = db::analysis::AnalysisEvent::record(
                    state.db(),
                    db::analysis::NewAnalysisEvent {
                        analysis_key: analysis.key(),
                        provider: analysis.provider.clone(),
                        stage: "queued".to_owned(),
                        level: "info".to_owned(),
                        message: "synthetic canary queued".to_owned(),
                        attempt: None,
                        attempts: None,
                        payload_bytes: None,
                        offset_bytes: None,
                        size_bytes: Some(video.size as i64),
                        duration_ms: None,
                    },
                )
                .await;
                state
                    .metrics()
                    .increment("leo_canary_runs_total", &[("result", "queued")]);
                eprintln!(
                    "{}",
                    json!({
                        "level": "info",
                        "component": "canary",
                        "event": "queued",
                        "analysis_id": analysis.key(),
                        "provider": analysis.provider,
                        "video_name": video.name,
                    })
                );
                ui::features::spawn_analysis_job(state.clone(), analysis);
            }
            Err(error) => {
                state
                    .metrics()
                    .increment("leo_canary_runs_total", &[("result", "queue_failed")]);
                eprintln!(
                    "{}",
                    json!({
                        "level": "error",
                        "component": "canary",
                        "event": "queue_failed",
                        "provider": provider.to_string(),
                        "error": error.to_string(),
                    })
                );
            }
        }
    }
}

async fn ensure_canary_video(db: &db::Database) -> Result<db::video::Video, CanaryError> {
    if let Some(asset) = db::video::Video::read_by_name(db, CANARY_VIDEO_NAME).await? {
        return Ok(asset.video);
    }

    let bytes = generate_canary_video().await?;
    Ok(db::video::Video::upload(db, CANARY_VIDEO_NAME, bytes).await?)
}

async fn generate_canary_video() -> Result<Vec<u8>, CanaryError> {
    let output = Builder::new()
        .prefix("leo-analysis-canary-")
        .suffix(".mp4")
        .tempfile()?;
    let path = output.path().to_owned();

    let command = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-y")
        .arg("-f")
        .arg("lavfi")
        .arg("-i")
        .arg("testsrc=size=320x180:rate=1:duration=1")
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg(&path)
        .output()
        .await?;

    if !command.status.success() {
        return Err(CanaryError::Ffmpeg(
            String::from_utf8_lossy(&command.stderr).trim().to_owned(),
        ));
    }

    Ok(fs::read(path).await?)
}

#[derive(Debug, thiserror::Error)]
enum CanaryError {
    #[error("ffmpeg canary generation failed: {0}")]
    Ffmpeg(String),
    #[error(transparent)]
    Video(#[from] db::video::VideoError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
