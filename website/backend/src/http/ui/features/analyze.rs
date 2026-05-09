use async_trait::async_trait;
use axum::extract::Path;
use axum_extra::extract::Form;
use hypertext::prelude::*;
use serde::Deserialize;

use crate::{
    analysis as ai_analysis, db,
    http::{
        router::AppState,
        ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
    },
};

pub struct AnalyzeRoute;
pub struct AnalysisStatusRoute;

pub struct AnalyzeView {
    analysis: db::analysis::Analysis,
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

        if context.state().runs_analysis_jobs() {
            spawn_analysis_job(context.state().clone(), analysis.clone());
        }

        Ok(AnalyzeView { analysis })
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

        Ok(AnalyzeView { analysis })
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
                </div>
            } @else {
                <div id="analysis-result" class="rounded-box border border-error/30 bg-error/10 p-4 text-sm leading-6 text-error shadow-sm">
                    <div class="mb-2 font-semibold">"Analysis failed"</div>
                    <div class="whitespace-pre-wrap">
                        (self.analysis.error.as_deref().unwrap_or("Analysis failed"))
                    </div>
                </div>
            }
        }
    }
}

fn spawn_analysis_job(state: AppState, analysis: db::analysis::Analysis) {
    tokio::spawn(async move {
        let db = state.db().clone();

        if let Err(error) = run_analysis_job(db.clone(), &analysis).await {
            eprintln!("analysis job failure: {error}");
            if let Err(update_error) = analysis.fail(&db, error.to_string()).await {
                eprintln!("analysis failure update failed: {update_error}");
            }
        }
    });
}

async fn run_analysis_job(
    db: db::Database,
    analysis: &db::analysis::Analysis,
) -> Result<(), RouteError> {
    analysis.mark_running(&db).await?;

    let mut videos = Vec::with_capacity(analysis.video_keys.len());
    for key in &analysis.video_keys {
        let Some(video) = db::video::Video::read_by_file_key(&db, key).await? else {
            return Err(RouteError::BadRequest("selected video was not found"));
        };
        videos.push(video);
    }

    let provider = ai_analysis::provider_from_value(&analysis.provider)?;
    let response = ai_analysis::analyze_videos(
        provider,
        videos,
        analysis.prompt.clone(),
        ai_analysis::request::AnalysisSettings {
            frame_sample_rate_fps: analysis.frame_sample_rate_fps,
        },
    )
    .await?;
    analysis.complete(&db, response).await?;

    Ok(())
}

fn validate_frame_sample_rate(value: f64) -> Result<f64, RouteError> {
    if value.is_finite() && (0.1..=8.0).contains(&value) {
        Ok(value)
    } else {
        Err(RouteError::BadRequest("unsupported frame sampling rate"))
    }
}
