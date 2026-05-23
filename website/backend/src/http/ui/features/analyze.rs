use async_trait::async_trait;
use axum::extract::Path;
use axum_extra::extract::Form;
use hypertext::prelude::*;
use serde::Deserialize;
use std::time::Instant;

use crate::{
    analysis::{
        self as ai_analysis, error::AnalysisError as AiAnalysisError, gemini::GeminiError,
        openai::OpenAiError, request::AnalysisTelemetry,
    },
    db,
    http::{
        router::AppState,
        ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
    },
};

pub struct AnalyzeRoute;
pub struct AnalysisStatusRoute;

pub struct AnalyzeView {
    analysis: db::analysis::Analysis,
    events: Vec<db::analysis::AnalysisEvent>,
}

#[derive(Deserialize)]
pub struct AnalyzeInput {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    frame_sample_rate_fps: Option<f64>,
    #[serde(default)]
    video_keys: Vec<String>,
    #[serde(default)]
    prompt: String,
}

#[async_trait]
impl Route for AnalyzeRoute {
    type Input = Form<AnalyzeInput>;
    type Authz = Public;
    type View = AnalyzeView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        Form(input): Self::Input,
    ) -> Result<Self::View, RouteError> {
        if input.video_keys.is_empty() {
            return Err(RouteError::BadRequest(
                "select at least one video to analyze",
            ));
        }

        if input.video_keys.len() > 10 {
            return Err(RouteError::BadRequest(
                "select no more than 10 videos to analyze",
            ));
        }

        if input.prompt.trim().is_empty() {
            return Err(RouteError::BadRequest("analysis prompt cannot be empty"));
        }

        let provider =
            ai_analysis::provider_from_value(input.provider.as_deref().unwrap_or("gemini"))
                .map_err(|_| RouteError::BadRequest("unsupported analysis provider"))?;
        let frame_sample_rate_fps =
            validate_frame_sample_rate(input.frame_sample_rate_fps.unwrap_or(0.2))?;
        let settings = ai_analysis::request::AnalysisSettings {
            frame_sample_rate_fps,
        };

        for key in &input.video_keys {
            if db::video::Video::find_by_file_key(context.state().db(), key)
                .await?
                .is_none()
            {
                return Err(RouteError::BadRequest("selected video was not found"));
            }
        }

        let analysis = db::analysis::Analysis::create_with_provider_and_settings(
            context.state().db(),
            provider,
            settings,
            input.prompt.trim(),
            input.video_keys,
        )
        .await?;
        context.state().metrics().increment(
            "leo_analysis_submissions_total",
            &[("provider", &analysis.provider)],
        );
        record_analysis_event(
            context.state().db(),
            &analysis,
            "queued",
            "info",
            "analysis queued",
            EventNumbers::default(),
        )
        .await?;
        let events =
            db::analysis::AnalysisEvent::list_for_analysis(context.state().db(), &analysis.key())
                .await?;

        if context.state().runs_analysis_jobs() {
            spawn_analysis_job(context.state().clone(), analysis.clone());
        }

        Ok(AnalyzeView { analysis, events })
    }
}

#[async_trait]
impl Route for AnalysisStatusRoute {
    type Input = (Path<String>, NoInput);
    type Authz = Public;
    type View = AnalyzeView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        (Path(analysis_id), _): Self::Input,
    ) -> Result<Self::View, RouteError> {
        let Some(analysis) =
            db::analysis::Analysis::find(context.state().db(), &analysis_id).await?
        else {
            return Err(RouteError::NotFound("analysis was not found"));
        };

        let events =
            db::analysis::AnalysisEvent::list_for_analysis(context.state().db(), &analysis.key())
                .await?;

        Ok(AnalyzeView { analysis, events })
    }
}

impl RouteView for AnalyzeView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        let status_path = format!("/analysis/{}", self.analysis.key());

        rsx! {
            @if self.analysis.is_pending() {
                <div
                    id="analysis-result"
                    class="rounded-box border border-base-300 bg-base-200/60 p-4 text-sm leading-6 text-base-content/80 shadow-sm"
                    hx-get=(status_path)
                    hx-trigger="every 2s"
                    hx-swap="outerHTML">
                    <div class="flex items-center gap-4">
                        <span class="loading loading-spinner loading-sm text-primary"></span>
                        <span class="font-medium">
                            @if self.analysis.status == "queued" {
                                "Analysis queued"
                            } @else {
                                "Analysis running"
                            }
                        </span>
                    </div>
                    (analysis_events(&self.events))
                </div>
            } @else if self.analysis.status == "complete" {
                <div id="analysis-result" class="rounded-box border border-base-300 bg-base-200/60 p-4 shadow-sm">
                    <div class="mb-3 flex items-center justify-between gap-3 border-b border-base-300 pb-3">
                        <h3 class="text-sm font-semibold text-base-content">"Analysis result"</h3>
                        <span class="badge badge-success badge-outline">"Complete"</span>
                    </div>
                    <div class="whitespace-pre-wrap text-sm leading-7 text-base-content/80">
                        (self.analysis.response.as_deref().unwrap_or(""))
                    </div>
                    (analysis_events(&self.events))
                </div>
            } @else {
                <div id="analysis-result" class="rounded-box border border-error/30 bg-error/10 p-4 text-sm leading-6 text-error shadow-sm">
                    <div class="mb-2 font-semibold">"Analysis failed"</div>
                    <div class="whitespace-pre-wrap">
                        (self.analysis.error.as_deref().unwrap_or("Analysis failed"))
                    </div>
                    @if let Some(diagnostic) = &self.analysis.failure_diagnostic {
                        (failure_diagnostics(diagnostic))
                    }
                    (analysis_events(&self.events))
                </div>
            }
        }
    }
}

pub fn spawn_analysis_job(state: AppState, analysis: db::analysis::Analysis) {
    tokio::spawn(async move {
        let provider = analysis.provider.clone();

        if let Err(error) = run_analysis_job(state.clone(), &analysis).await {
            state.metrics().increment(
                "leo_analysis_jobs_total",
                &[("provider", &provider), ("result", "failed")],
            );
            let diagnostic = failure_diagnostic(&provider, &error);
            eprintln!(
                "{}",
                AnalysisTelemetry::new(analysis.key(), provider.clone()).event_json(
                    "error",
                    "analysis_job",
                    "failed",
                    [
                        ("stage", serde_json::json!(diagnostic.stage)),
                        ("kind", serde_json::json!(diagnostic.kind)),
                        ("message", serde_json::json!(diagnostic.message)),
                    ],
                )
            );
            if let Err(update_error) = analysis
                .fail_with_diagnostic(state.db(), diagnostic.clone())
                .await
            {
                eprintln!("analysis failure update failed: {update_error}");
            }
            if let Err(event_error) = record_analysis_event(
                state.db(),
                &analysis,
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
            .await
            {
                eprintln!("analysis failure event update failed: {event_error}");
            }
        }
    });
}

async fn run_analysis_job(
    state: AppState,
    analysis: &db::analysis::Analysis,
) -> Result<(), RouteError> {
    let db = state.db().clone();
    let started_at = Instant::now();
    let telemetry = AnalysisTelemetry::new(analysis.key(), analysis.provider.clone())
        .with_canary(analysis.is_canary);
    telemetry.log(
        "info",
        "analysis_job",
        "started",
        [("provider", serde_json::json!(analysis.provider))],
    );
    analysis.mark_running(&db).await?;
    record_analysis_event(
        &db,
        analysis,
        "running",
        "info",
        "analysis started",
        EventNumbers::default(),
    )
    .await?;

    let mut videos = Vec::with_capacity(analysis.video_keys.len());
    for key in &analysis.video_keys {
        let Some(video) = db::video::Video::read_by_file_key(&db, key).await? else {
            return Err(RouteError::BadRequest("selected video was not found"));
        };
        videos.push(video);
    }
    let total_video_bytes = videos.iter().map(|asset| asset.bytes.len() as i64).sum();
    record_analysis_event(
        &db,
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
    analysis.complete(&db, response).await?;
    state.metrics().increment(
        "leo_analysis_jobs_total",
        &[("provider", &analysis.provider), ("result", "completed")],
    );
    record_analysis_event(
        &db,
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
) -> Result<(), RouteError> {
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
    error: &RouteError,
) -> db::analysis::AnalysisFailureDiagnostic {
    match error {
        RouteError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Request {
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
        RouteError::AiAnalysis(AiAnalysisError::OpenAi(OpenAiError::Api { status, .. })) => {
            db::analysis::AnalysisFailureDiagnostic {
                stage: "openai.api".to_owned(),
                kind: format!("http_{}", status.as_u16()),
                retryable: status.is_server_error(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                message: error.to_string(),
            }
        }
        RouteError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::UploadRequest {
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
        RouteError::AiAnalysis(AiAnalysisError::Gemini(
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
        RouteError::AiAnalysis(AiAnalysisError::Gemini(GeminiError::Api { status, .. })) => {
            db::analysis::AnalysisFailureDiagnostic {
                stage: "gemini.api".to_owned(),
                kind: format!("http_{}", status.as_u16()),
                retryable: status.is_server_error(),
                attempt: None,
                attempts: None,
                payload_bytes: None,
                message: error.to_string(),
            }
        }
        RouteError::AiAnalysis(AiAnalysisError::FrameExtraction(_)) => {
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
        RouteError::BadRequest(_) => db::analysis::AnalysisFailureDiagnostic {
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

fn failure_diagnostics(diagnostic: &db::analysis::AnalysisFailureDiagnostic) -> impl Renderable {
    rsx! {
        <div class="space-y-2 border-b border-base-300 pb-3">
            <div class="font-semibold">"Failure diagnostics"</div>
            <div class="text-xs text-base-content/70">
                "stage="(diagnostic.stage.as_str())
                " kind="(diagnostic.kind.as_str())
                " retryable="(diagnostic.retryable)
                @if let Some(attempt) = diagnostic.attempt {
                    " attempt="(attempt)
                }
                @if let Some(attempts) = diagnostic.attempts {
                    "/"(attempts)
                }
                @if let Some(payload_bytes) = diagnostic.payload_bytes {
                    " payload_bytes="(payload_bytes)
                }
            </div>
        </div>
    }
}

fn analysis_events(events: &[db::analysis::AnalysisEvent]) -> impl Renderable {
    rsx! {
        @if !events.is_empty() {
            <div class="space-y-2 border-b border-base-300 pb-3">
                <div class="font-semibold">"Event history"</div>
                <ul class="list-disc space-y-1 pl-5 text-xs text-base-content/70">
                    @for event in events {
                        <li>
                            (event.stage.as_str())": "(event.message.as_str())
                            @if let Some(duration_ms) = event.duration_ms {
                                " duration_ms="(duration_ms)
                            }
                            @if let Some(payload_bytes) = event.payload_bytes {
                                " payload_bytes="(payload_bytes)
                            }
                        </li>
                    }
                </ul>
            </div>
        }
    }
}

fn validate_frame_sample_rate(value: f64) -> Result<f64, RouteError> {
    if value.is_finite() && (0.1..=8.0).contains(&value) {
        Ok(value)
    } else {
        Err(RouteError::BadRequest("unsupported frame sampling rate"))
    }
}
