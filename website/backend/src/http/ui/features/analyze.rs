//! Analysis submission and status fragment routes.

use async_trait::async_trait;
use axum::extract::Path;
use axum_extra::extract::Form;
use hypertext::prelude::*;
use serde::Deserialize;

use crate::{
    analysis::job::{self as analysis_job, AnalysisSnapshot, AnalysisSubmission},
    app::{self, AppState},
    db,
    http::ui::{NoInput, Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
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
        let snapshot = analysis_job::queue_analysis(
            context.state().db(),
            AnalysisSubmission {
                provider: input.provider,
                frame_sample_rate_fps: input.frame_sample_rate_fps,
                video_keys: input.video_keys,
                prompt: input.prompt,
            },
        )
        .await?;
        let analysis = snapshot.analysis.clone();
        context.state().metrics().increment(
            "leo_analysis_submissions_total",
            &[("provider", &analysis.provider)],
        );

        if context.state().runs_analysis_jobs() {
            app::spawn_analysis_job(context.state().clone(), analysis);
        }

        Ok(snapshot.into())
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
        let Some(snapshot) =
            analysis_job::load_analysis_snapshot(context.state().db(), &analysis_id).await?
        else {
            return Err(RouteError::NotFound("analysis was not found"));
        };

        Ok(snapshot.into())
    }
}

impl From<AnalysisSnapshot> for AnalyzeView {
    fn from(snapshot: AnalysisSnapshot) -> Self {
        Self {
            analysis: snapshot.analysis,
            events: snapshot.events,
        }
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
