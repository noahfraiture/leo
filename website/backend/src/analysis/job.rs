use std::time::Instant;

use serde_json::json;
use thiserror::Error;

use crate::{
    analysis::{
        self as ai_analysis,
        error::AnalysisError as AiAnalysisError,
        gemini::GeminiError,
        openai::OpenAiError,
        request::{AnalysisTelemetry, AnalysisVideo},
    },
    db,
};

pub struct AnalysisSubmission {
    pub provider: Option<String>,
    pub frame_sample_rate_fps: Option<f64>,
    pub video_keys: Vec<String>,
    pub prompt: String,
}

pub struct AnalysisSnapshot {
    pub analysis: db::analysis::Analysis,
    pub events: Vec<db::analysis::AnalysisEvent>,
}

#[derive(Debug, Error)]
pub enum AnalysisJobError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error(transparent)]
    Video(#[from] db::video::VideoError),
    #[error(transparent)]
    Analysis(#[from] db::analysis::AnalysisError),
    #[error(transparent)]
    AiAnalysis(#[from] AiAnalysisError),
}

pub async fn queue_analysis(
    db: &db::Database,
    submission: AnalysisSubmission,
) -> Result<AnalysisSnapshot, AnalysisJobError> {
    let video_keys = validate_selected_videos(submission.video_keys)?;
    let prompt = validate_prompt(submission.prompt)?;
    let provider =
        ai_analysis::provider_from_value(submission.provider.as_deref().unwrap_or("gemini"))
            .map_err(|_| AnalysisJobError::BadRequest("unsupported analysis provider"))?;
    let frame_sample_rate_fps =
        validate_frame_sample_rate(submission.frame_sample_rate_fps.unwrap_or(0.2))?;
    let settings = ai_analysis::request::AnalysisSettings {
        frame_sample_rate_fps,
    };

    for key in &video_keys {
        if db::video::Video::find_by_file_key(db, key).await?.is_none() {
            return Err(AnalysisJobError::BadRequest("selected video was not found"));
        }
    }

    let analysis = db::analysis::Analysis::create_with_provider_and_settings(
        db, provider, settings, prompt, video_keys,
    )
    .await?;
    record_analysis_event(
        db,
        &analysis,
        "queued",
        "info",
        "analysis queued",
        EventNumbers::default(),
    )
    .await?;

    Ok(AnalysisSnapshot {
        events: db::analysis::AnalysisEvent::list_for_analysis(db, &analysis.key()).await?,
        analysis,
    })
}

pub async fn load_analysis_snapshot(
    db: &db::Database,
    analysis_id: &str,
) -> Result<Option<AnalysisSnapshot>, AnalysisJobError> {
    let Some(analysis) = db::analysis::Analysis::find(db, analysis_id).await? else {
        return Ok(None);
    };
    let events = db::analysis::AnalysisEvent::list_for_analysis(db, &analysis.key()).await?;

    Ok(Some(AnalysisSnapshot { analysis, events }))
}

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

pub async fn record_analysis_failure(
    db: &db::Database,
    analysis: &db::analysis::Analysis,
    error: &AnalysisJobError,
) -> Result<db::analysis::AnalysisFailureDiagnostic, AnalysisJobError> {
    let diagnostic = failure_diagnostic(&analysis.provider, error);
    eprintln!(
        "{}",
        AnalysisTelemetry::new(analysis.key(), analysis.provider.clone()).event_json(
            "error",
            "analysis_job",
            "failed",
            [
                ("stage", json!(&diagnostic.stage)),
                ("kind", json!(&diagnostic.kind)),
                ("message", json!(&diagnostic.message)),
            ],
        )
    );
    analysis
        .fail_with_diagnostic(db, diagnostic.clone())
        .await?;
    record_analysis_event(
        db,
        analysis,
        &diagnostic.stage,
        "error",
        &diagnostic.message,
        EventNumbers {
            attempt: diagnostic.attempt,
            attempts: diagnostic.attempts,
            payload_bytes: diagnostic.payload_bytes,
            offset_bytes: None,
            size_bytes: None,
            duration_ms: None,
        },
    )
    .await?;

    Ok(diagnostic)
}

fn validate_selected_videos(video_keys: Vec<String>) -> Result<Vec<String>, AnalysisJobError> {
    if video_keys.is_empty() {
        return Err(AnalysisJobError::BadRequest(
            "select at least one video to analyze",
        ));
    }

    if video_keys.len() > 10 {
        return Err(AnalysisJobError::BadRequest(
            "select no more than 10 videos to analyze",
        ));
    }

    Ok(video_keys)
}

fn validate_prompt(prompt: String) -> Result<String, AnalysisJobError> {
    let prompt = prompt.trim().to_owned();
    if prompt.is_empty() {
        return Err(AnalysisJobError::BadRequest(
            "analysis prompt cannot be empty",
        ));
    }

    Ok(prompt)
}

fn validate_frame_sample_rate(value: f64) -> Result<f64, AnalysisJobError> {
    if value.is_finite() && (0.1..=8.0).contains(&value) {
        Ok(value)
    } else {
        Err(AnalysisJobError::BadRequest(
            "unsupported frame sampling rate",
        ))
    }
}

#[derive(Clone, Copy, Default)]
struct EventNumbers {
    attempt: Option<i64>,
    attempts: Option<i64>,
    payload_bytes: Option<i64>,
    offset_bytes: Option<i64>,
    size_bytes: Option<i64>,
    duration_ms: Option<i64>,
}

async fn record_analysis_event(
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

fn failure_diagnostic(
    provider: &str,
    error: &AnalysisJobError,
) -> db::analysis::AnalysisFailureDiagnostic {
    match error {
        AnalysisJobError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Request {
            stage,
            attempt,
            attempts,
            payload_bytes,
            timeout,
            connect,
            body,
            request,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: format!("openai.{stage}"),
            kind: request_failure_kind(*timeout, *connect, *body, *request).to_owned(),
            retryable: true,
            attempt: Some(*attempt as i64),
            attempts: Some(*attempts as i64),
            payload_bytes: Some(*payload_bytes as i64),
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Api {
            status, ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "openai.api".to_owned(),
            kind: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::UploadRequest {
            offset,
            bytes,
            timeout,
            connect,
            body,
            ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.upload".to_owned(),
            kind: request_failure_kind(*timeout, *connect, *body, false).to_owned(),
            retryable: true,
            attempt: None,
            attempts: None,
            payload_bytes: Some(*bytes as i64),
            message: format!("{error} offset={offset}"),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(
            GeminiError::UploadFinalizationUnknown { attempts, .. },
        )) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.upload_finalize".to_owned(),
            kind: "lost_final_response".to_owned(),
            retryable: true,
            attempt: Some(*attempts as i64),
            attempts: Some(*attempts as i64),
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::Api {
            status, ..
        })) => db::analysis::AnalysisFailureDiagnostic {
            stage: "gemini.api".to_owned(),
            kind: format!("http_{}", status.as_u16()),
            retryable: status.is_server_error(),
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        AnalysisJobError::AiAnalysis(AiAnalysisError::FrameExtraction(_)) => {
            db::analysis::AnalysisFailureDiagnostic {
                stage: "frame_extraction".to_owned(),
                kind: "ffmpeg".to_owned(),
                retryable: false,
                attempt: None,
                attempts: None,
                payload_bytes: None,
                message: error.to_string(),
            }
        }
        AnalysisJobError::BadRequest(_) => db::analysis::AnalysisFailureDiagnostic {
            stage: "input_validation".to_owned(),
            kind: "bad_request".to_owned(),
            retryable: false,
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
        _ => db::analysis::AnalysisFailureDiagnostic {
            stage: provider.to_owned(),
            kind: "internal".to_owned(),
            retryable: false,
            attempt: None,
            attempts: None,
            payload_bytes: None,
            message: error.to_string(),
        },
    }
}

fn request_failure_kind(timeout: bool, connect: bool, body: bool, request: bool) -> &'static str {
    if timeout {
        "timeout"
    } else if connect {
        "connect"
    } else if body {
        "body"
    } else if request {
        "request"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::{AnalysisJobError, validate_frame_sample_rate, validate_prompt};

    #[test]
    fn prompt_validation_trims_user_prompt_before_persistence() {
        assert_eq!(
            validate_prompt("  Summarize this clip.  ".to_owned()).expect("prompt should validate"),
            "Summarize this clip."
        );
    }

    #[test]
    fn frame_sample_rate_validation_rejects_values_outside_supported_range() {
        assert!(matches!(
            validate_frame_sample_rate(20.0),
            Err(AnalysisJobError::BadRequest(
                "unsupported frame sampling rate"
            ))
        ));
    }
}
