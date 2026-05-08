use async_trait::async_trait;
use axum::http::StatusCode;
use hypertext::prelude::*;

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

/// Public workspace page mounted at `/`.
pub struct HomePage;

pub struct HomePageView {
    videos: Vec<db::video::Video>,
}

#[async_trait]
impl Route for HomePage {
    type Input = NoInput;
    type Authz = Public;
    type View = HomePageView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        _input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        let videos = db::video::Video::list(context.state().db()).await?;

        Ok(HomePageView { videos })
    }
}

impl RouteView for HomePageView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | Home",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    (top_bar())

                    <section class="space-y-6">
                        (intro())

                        (video_intake(&self.videos))

                        (analysis_prompt(&self.videos))
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
        </header>
    }
}

fn intro() -> impl Renderable {
    rsx! {
        <div class="space-y-2">
            <h1 class="text-3xl font-semibold text-base-content">"Upload videos"</h1>
            <p class="max-w-2xl text-base-content/70">
                "A server-rendered workspace for video uploads and AI analysis with OpenAI and Gemini."
            </p>
        </div>
    }
}

fn video_intake(videos: &[db::video::Video]) -> impl Renderable {
    rsx! {
        <section class="rounded-box border border-base-300 bg-base-100 p-6 shadow-sm">
            <form
                class="space-y-4"
                method="post"
                action="/videos"
                enctype="multipart/form-data"
                hx-post="/videos"
                hx-encoding="multipart/form-data"
                hx-target="#video-selection"
                hx-swap="outerHTML"
                hx-indicator="#upload-indicator">
                <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                    "Upload"
                </p>
                <h2 class="text-xl font-semibold text-base-content">"Video intake"</h2>

                <label class="form-control space-y-2">
                    <span class="text-sm font-medium text-base-content">"Video file"</span>
                    <input
                        class="file-input file-input-bordered w-full"
                        type="file"
                        name="video"
                        accept="video/*"
                        required="required" />
                </label>

                <div class="flex flex-wrap items-center justify-between gap-3">
                    <span class="text-sm text-base-content/60">
                        (format!("{} uploaded", videos.len()))
                    </span>
                    <div class="flex items-center gap-3">
                        <span
                            id="upload-indicator"
                            class="htmx-indicator inline-flex items-center gap-2 text-sm text-base-content/70">
                            <span class="loading loading-spinner loading-sm"></span>
                            "Uploading"
                        </span>
                        <button class="btn btn-primary" type="submit">"Upload"</button>
                    </div>
                </div>
            </form>
        </section>
    }
}

fn analysis_prompt(videos: &[db::video::Video]) -> impl Renderable {
    rsx! {
        <section class="space-y-6">
            <div class="space-y-2">
                <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                    "Analyze"
                </p>
                <h2 class="text-2xl font-semibold text-base-content">"Prompt videos"</h2>
                <p class="max-w-2xl text-sm leading-6 text-base-content/70">
                    "Choose uploaded videos, add the analysis prompt, and submit it for processing."
                </p>
            </div>

            <form
                class="space-y-5 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm"
                method="post"
                action="/analysis"
                hx-post="/analysis"
                hx-target="#analysis-result"
                hx-swap="outerHTML"
                hx-indicator="#analysis-indicator">
                (video_selection(videos))

                <label class="form-control space-y-2">
                    <span class="text-sm font-medium text-base-content">"Prompt"</span>
                    <textarea
                        class="textarea textarea-bordered min-h-32 w-full"
                        name="prompt"
                        placeholder="Describe what the AI should look for in the selected videos."
                        required="required"></textarea>
                </label>

                <div class="flex flex-wrap items-center justify-between gap-3">
                    <p id="analysis-result" class="text-sm text-base-content/70"></p>
                    <div class="flex items-center gap-3">
                        <span
                            id="analysis-indicator"
                            class="htmx-indicator inline-flex items-center gap-2 text-sm text-base-content/70">
                            <span class="loading loading-spinner loading-sm"></span>
                            "Analyzing"
                        </span>
                        <button class="btn btn-primary" type="submit">"Run analysis"</button>
                    </div>
                </div>
            </form>
        </section>
    }
}

pub(super) fn video_selection(videos: &[db::video::Video]) -> impl Renderable {
    rsx! {
        <fieldset id="video-selection" class="space-y-3">
            <legend class="text-sm font-medium text-base-content">"Videos"</legend>

            @if videos.is_empty() {
                <p class="rounded-box border border-dashed border-base-300 p-4 text-sm text-base-content/70">
                    "No videos have been uploaded yet."
                </p>
            } @else {
                <div class="space-y-2">
                    @for video in videos.iter() {
                        (video_option(video))
                    }
                </div>
            }
        </fieldset>
    }
}

pub(super) fn video_option(video: &db::video::Video) -> impl Renderable {
    let delete_path = format!("/videos/{}", video.file.key().trim_start_matches('/'));
    let delete_label = format!("Delete {}", video.name);
    let delete_confirm = format!("Delete {}?", video.name);

    rsx! {
        <div class="flex items-center gap-2 rounded-box border border-base-300 hover:bg-base-200">
            <label class="flex min-w-0 flex-1 cursor-pointer items-center gap-3 p-3">
                <input
                    class="checkbox checkbox-primary"
                    type="checkbox"
                    name="video_keys"
                    value=(video.file.key()) />
                <span class="min-w-0 flex-1">
                    <span class="block truncate text-sm font-medium text-base-content">
                        (video.name.as_str())
                    </span>
                    <span class="block text-xs text-base-content/60">
                        (format!("{} bytes", video.size))
                    </span>
                </span>
            </label>

            <button
                class="btn btn-ghost btn-sm mr-2 text-error hover:bg-error hover:text-error-content"
                type="button"
                aria-label=(delete_label)
                hx-delete=(delete_path)
                hx-target="#video-selection"
                hx-swap="outerHTML"
                hx-confirm=(delete_confirm)>
                "Delete"
            </button>
        </div>
    }
}

pub async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}
