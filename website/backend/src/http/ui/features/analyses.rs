use std::collections::HashMap;

use async_trait::async_trait;
use axum::{
    extract::{Path, Query},
    http::StatusCode,
};
use hypertext::prelude::*;
use serde::Deserialize;

use crate::{
    db,
    http::{
        router::AppState,
        ui::{
            NoInput, Public, Route, RouteContext, RouteError, RouteView, document,
            not_found_fragment,
        },
    },
};

const PAGE_SIZE: usize = 20;

pub struct AnalysesPage;
pub struct ClearAnalysesRoute;
pub struct DeleteAnalysisRoute;

pub struct AnalysesPageView {
    analyses: Vec<db::analysis::Analysis>,
    videos: Vec<db::video::Video>,
    page: usize,
    has_next_page: bool,
}

#[derive(Deserialize)]
pub struct AnalysesQuery {
    #[serde(default = "default_page")]
    page: usize,
}

#[async_trait]
impl Route for AnalysesPage {
    type Input = (Query<AnalysesQuery>, NoInput);
    type Authz = Public;
    type View = AnalysesPageView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        (Query(input), _): Self::Input,
    ) -> Result<Self::View, RouteError> {
        load_analyses_page(context.state(), input.page).await
    }
}

#[async_trait]
impl Route for ClearAnalysesRoute {
    type Input = NoInput;
    type Authz = Public;
    type View = AnalysesPageView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        _input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        db::analysis::Analysis::clear_history(context.state().db()).await?;
        load_analyses_page(context.state(), 1).await
    }
}

pub struct DeleteAnalysisView;

#[async_trait]
impl Route for DeleteAnalysisRoute {
    type Input = (Path<String>, NoInput);
    type Authz = Public;
    type View = DeleteAnalysisView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        (Path(analysis_key), _): Self::Input,
    ) -> Result<Self::View, RouteError> {
        db::analysis::Analysis::hide_from_history(context.state().db(), &analysis_key).await?;
        Ok(DeleteAnalysisView)
    }
}

impl RouteView for AnalysesPageView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | History",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    (top_bar())

                    <section class="space-y-6">
                        (history_header(!self.analyses.is_empty()))

                        (analysis_history(&self.analyses, &self.videos))
                        (pagination(self.page, self.has_next_page))
                    </section>
                </main>
            },
        )
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment_status() -> StatusCode {
        StatusCode::NOT_FOUND
    }
}

impl RouteView for DeleteAnalysisView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        rsx! {}
    }
}

async fn load_analyses_page(state: &AppState, page: usize) -> Result<AnalysesPageView, RouteError> {
    let page = page.max(1);
    let offset = (page - 1) * PAGE_SIZE;
    let mut analyses = db::analysis::Analysis::list_page(state.db(), PAGE_SIZE + 1, offset).await?;
    let has_next_page = analyses.len() > PAGE_SIZE;
    analyses.truncate(PAGE_SIZE);
    let videos = db::video::Video::list(state.db()).await?;

    Ok(AnalysesPageView {
        analyses,
        videos,
        page,
        has_next_page,
    })
}

pub(super) fn analysis_history(
    analyses: &[db::analysis::Analysis],
    videos: &[db::video::Video],
) -> impl Renderable {
    let video_names = video_names_by_key(videos);

    rsx! {
        <div class="space-y-3">
            @if analyses.is_empty() {
                <p class="rounded-box border border-dashed border-base-300 p-4 text-sm text-base-content/70">
                    "No analyses have been run yet."
                </p>
            } @else {
                @for analysis in analyses.iter() {
                    (analysis_entry(analysis, &video_names))
                }
            }
        </div>
    }
}

fn top_bar() -> impl Renderable {
    rsx! {
        <header class="navbar rounded-box border border-base-300 bg-base-100 px-4 shadow-sm">
            <div class="flex-1">
                <a
                    class="btn btn-ghost px-0 text-xl font-semibold normal-case"
                    href="/">
                    "Video analysis"
                </a>
            </div>
            <div class="flex-none">
                <a class="btn btn-sm btn-ghost" href="/">"Workspace"</a>
            </div>
        </header>
    }
}

fn history_header(can_clear: bool) -> impl Renderable {
    rsx! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div class="space-y-2">
                <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                    "History"
                </p>
                <h1 class="text-3xl font-semibold text-base-content">"Analysis history"</h1>
                <p class="max-w-2xl text-sm leading-6 text-base-content/70">
                    "Review previous video selections, prompts, and saved responses."
                </p>
            </div>
            <form method="post" action="/analyses/clear">
                @if can_clear {
                    <button class="btn btn-sm btn-outline btn-error" type="submit">
                        "Clear history"
                    </button>
                } @else {
                    <button class="btn btn-sm btn-outline btn-error" type="submit" disabled="disabled">
                        "Clear history"
                    </button>
                }
            </form>
        </div>
    }
}

fn analysis_entry(
    analysis: &db::analysis::Analysis,
    video_names: &HashMap<String, String>,
) -> impl Renderable {
    let delete_path = format!("/analyses/{}/delete", analysis.key());
    let delete_label = "Delete analysis";

    rsx! {
        <article class="space-y-5 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm">
            <div class="flex flex-wrap items-start justify-between gap-3">
                <div class="min-w-0 space-y-1">
                    <h2 class="truncate text-base font-semibold text-base-content">
                        (analysis.prompt.as_str())
                    </h2>
                    <p class="truncate text-sm text-base-content/60">
                        (selected_video_names(&analysis.video_keys, video_names))
                    </p>
                    <time
                        class="block text-xs text-base-content/50"
                        datetime=(analysis.created_at.to_string())>
                        (analysis_started_at_label(analysis))
                    </time>
                </div>
                <div class="flex flex-wrap items-center gap-2">
                    <span class="badge badge-outline">
                        (analysis.provider.as_str())
                    </span>
                    (status_badge(analysis.status.as_str()))
                </div>
            </div>

            <section class="space-y-2">
                <h3 class="text-sm font-semibold text-base-content">"Videos"</h3>
                <ul class="list-disc space-y-1 pl-5 text-sm text-base-content/70">
                    @for name in selected_video_names_vec(&analysis.video_keys, video_names) {
                        <li>(name)</li>
                    }
                </ul>
            </section>

            <section class="space-y-2">
                <h3 class="text-sm font-semibold text-base-content">"Prompt"</h3>
                <p class="whitespace-pre-wrap text-sm leading-6 text-base-content/80">
                    (analysis.prompt.as_str())
                </p>
            </section>

            <section class="space-y-2">
                <h3 class="text-sm font-semibold text-base-content">"Response"</h3>
                (analysis_body(analysis))
            </section>

            <button
                class="btn btn-error w-full"
                type="button"
                aria-label=(delete_label)
                hx-post=(delete_path)
                hx-target="closest article"
                hx-swap="delete">
                "Delete analysis"
            </button>
        </article>
    }
}

fn status_badge(status: &str) -> impl Renderable {
    let class = match status {
        "complete" => "badge badge-success badge-outline",
        "failed" => "badge badge-error badge-outline",
        "running" => "badge badge-info badge-outline",
        _ => "badge badge-warning badge-outline",
    };

    rsx! {
        <span class=(class)>(status)</span>
    }
}

fn analysis_body(analysis: &db::analysis::Analysis) -> impl Renderable {
    rsx! {
        @if analysis.status == "complete" {
            <p class="whitespace-pre-wrap text-sm leading-6 text-base-content/80">
                (analysis.response.as_deref().unwrap_or(""))
            </p>
        } @else if analysis.status == "failed" {
            <p class="whitespace-pre-wrap text-sm leading-6 text-error">
                (analysis.error.as_deref().unwrap_or("Analysis failed"))
            </p>
        } @else {
            <p class="text-sm leading-6 text-base-content/70">
                "Analysis has not finished yet."
            </p>
        }
    }
}

fn pagination(page: usize, has_next_page: bool) -> impl Renderable {
    let previous_page = page.saturating_sub(1);
    let next_page = page + 1;
    let previous_path = format!("/analyses?page={previous_page}");
    let next_path = format!("/analyses?page={next_page}");

    rsx! {
        <div class="join flex justify-end">
            @if page > 1 {
                <a class="btn join-item" href=(previous_path)>"Previous"</a>
            } @else {
                <button class="btn join-item" type="button" disabled="disabled">"Previous"</button>
            }
            <button class="btn join-item" type="button">("Page ")(page)</button>
            @if has_next_page {
                <a class="btn join-item" href=(next_path)>"Next"</a>
            } @else {
                <button class="btn join-item" type="button" disabled="disabled">"Next"</button>
            }
        </div>
    }
}

fn video_names_by_key(videos: &[db::video::Video]) -> HashMap<String, String> {
    videos
        .iter()
        .map(|video| (video.file.key().to_owned(), video.name.clone()))
        .collect()
}

fn analysis_started_at_label(analysis: &db::analysis::Analysis) -> String {
    format!(
        "Started {}",
        analysis.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    )
}

fn selected_video_names(video_keys: &[String], video_names: &HashMap<String, String>) -> String {
    selected_video_names_vec(video_keys, video_names).join(", ")
}

fn selected_video_names_vec(
    video_keys: &[String],
    video_names: &HashMap<String, String>,
) -> Vec<String> {
    video_keys
        .iter()
        .map(|key| video_names.get(key).cloned().unwrap_or_else(|| key.clone()))
        .collect()
}

fn default_page() -> usize {
    1
}
