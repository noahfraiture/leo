//! Home workspace page for upload, video playback, and analysis submission.

use async_trait::async_trait;
use axum::http::StatusCode;
use hypertext::prelude::*;

use crate::{
    app::AppState,
    db,
    http::ui::{
        NoInput, Public, Route, RouteContext, RouteError, RouteView, document, not_found_fragment,
    },
    upload::MAX_VIDEO_UPLOAD_SIZE_LABEL,
};

const RECENT_ANALYSIS_LIMIT: usize = 5;

/// Public workspace page mounted at `/`.
pub struct HomePage;

pub struct HomePageView {
    videos: Vec<db::video::Video>,
    recent_analyses: Vec<db::analysis::Analysis>,
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
        let recent_analyses =
            db::analysis::Analysis::list_recent(context.state().db(), RECENT_ANALYSIS_LIMIT)
                .await?;

        Ok(HomePageView {
            videos,
            recent_analyses,
        })
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

                        (video_workspace(&self.videos))

                        (recent_analyses(&self.recent_analyses, &self.videos))
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
            <div class="flex-none">
                <a class="btn btn-sm btn-ghost" href="/analyses">"History"</a>
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
                "x-data"="chunkedVideoUpload"
                "x-on:submit.prevent"="upload">
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
                        required="required"
                        "x-ref"="video" />
                    <span class="text-xs text-base-content/60">
                        (format!("Uploads are limited to {MAX_VIDEO_UPLOAD_SIZE_LABEL}."))
                    </span>
                </label>

                <div class="flex flex-wrap items-center justify-between gap-3">
                    <span class="text-sm text-base-content/60">
                        (format!("{} uploaded", videos.len()))
                    </span>
                    <div class="flex items-center gap-3">
                        <span
                            id="upload-indicator"
                            class="inline-flex items-center gap-2 text-sm text-base-content/70"
                            "x-show"="uploading"
                            "x-cloak"="x-cloak">
                            <span class="loading loading-spinner loading-sm"></span>
                            <span "x-text"="status">"Uploading"</span>
                        </span>
                        <span
                            class="text-sm text-base-content/70"
                            "x-show"="!uploading && status"
                            "x-text"="status"
                            "x-cloak"="x-cloak">
                        </span>
                        <span
                            class="text-sm text-error"
                            "x-show"="error"
                            "x-text"="error"
                            "x-cloak"="x-cloak">
                        </span>
                        <button
                            class="btn btn-primary"
                            type="submit"
                            "x-bind:disabled"="uploading">
                            "Upload"
                        </button>
                    </div>
                </div>
            </form>
        </section>
    }
}

pub(super) fn video_workspace(videos: &[db::video::Video]) -> impl Renderable {
    rsx! {
        <div id="video-workspace" class="space-y-6">
            (video_player(videos))
            (analysis_prompt(videos))
        </div>
    }
}

fn video_player(videos: &[db::video::Video]) -> impl Renderable {
    let selected_path = videos
        .first()
        .map(|video| video.path.as_str())
        .unwrap_or("");

    rsx! {
        <section
            class="space-y-4 rounded-box border border-base-300 bg-base-100 p-6 shadow-sm"
            "x-data"="videoPlayer"
            data-selected-video=(selected_path)>
            <div class="space-y-2">
                <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                    "Preview"
                </p>
                <h2 class="text-xl font-semibold text-base-content">"Preview video"</h2>
            </div>

            @if videos.is_empty() {
                <p class="rounded-box border border-dashed border-base-300 p-4 text-sm text-base-content/70">
                    "No videos have been uploaded yet."
                </p>
            } @else {
                <div class="flex flex-col gap-8">
                    <label class="flex flex-col gap-3">
                        <span class="text-sm font-medium text-base-content">"Video"</span>
                        <select
                            class="select select-bordered w-full"
                            "x-model"="selectedVideo">
                            @for video in videos.iter() {
                                <option value=(video.path.as_str())>
                                    (video.name.as_str())
                                </option>
                            }
                        </select>
                    </label>

                    <video
                        class="w-full rounded-box border border-base-300 bg-base-200"
                        controls="controls"
                        preload="metadata"
                        "x-bind:src"="selectedVideo">
                    </video>
                </div>
            }
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
                "x-data"="{ provider: 'gemini' }"
                hx-post="/analysis"
                hx-target="#analysis-result"
                hx-swap="outerHTML"
                hx-indicator="#analysis-indicator">
                (analysis_provider_selection())
                (frame_sampling_selection())
                (video_selection(videos))

                <label class="form-control space-y-2 gap-3">
                    <span class="text-sm font-medium text-base-content">"Prompt"</span>
                    <textarea
                        class="textarea textarea-bordered min-h-32 w-full"
                        name="prompt"
                        placeholder="Describe what the AI should look for in the selected videos."
                        required="required"></textarea>
                </label>

                <div id="analysis-result" class="empty:hidden"></div>

                <div class="flex flex-wrap items-center justify-end gap-3">
                    <span
                        id="analysis-indicator"
                        class="htmx-indicator inline-flex items-center gap-2 text-sm text-base-content/70">
                        <span class="loading loading-spinner loading-sm"></span>
                        "Analyzing"
                    </span>
                    <button class="btn btn-primary" type="submit">"Run analysis"</button>
                </div>
            </form>
        </section>
    }
}

fn frame_sampling_selection() -> impl Renderable {
    rsx! {
        <fieldset class="space-y-3" "x-show"="provider === 'openai'" "x-cloak"="">
            <legend class="text-sm font-medium text-base-content">"Frame sampling"</legend>
            <div class="join flex-wrap">
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="0.1"
                    aria-label="1 / 10s" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="0.2"
                    aria-label="1 / 5s"
                    checked="checked" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="0.5"
                    aria-label="1 / 2s" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="1"
                    aria-label="1 fps" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="2"
                    aria-label="2 fps" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="frame_sample_rate_fps"
                    value="4"
                    aria-label="4 fps" />
            </div>
        </fieldset>
    }
}

fn analysis_provider_selection() -> impl Renderable {
    rsx! {
        <fieldset class="space-y-3">
            <legend class="text-sm font-medium text-base-content">"Provider"</legend>
            <div id="provider-switch" class="join">
                <input
                    class="btn join-item"
                    type="radio"
                    name="provider"
                    value="gemini"
                    "x-model"="provider"
                    aria-label="Gemini"
                    checked="checked" />
                <input
                    class="btn join-item"
                    type="radio"
                    name="provider"
                    value="openai"
                    "x-model"="provider"
                    aria-label="OpenAI" />
            </div>
        </fieldset>
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
    let delete_path = format!(
        "/videos/{}/delete",
        video.file.key().trim_start_matches('/')
    );
    let delete_label = format!("Delete {}", video.name);
    let delete_confirm = format!("Delete {}?", video.name);
    let size_label = megabyte_label(video.size);

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
                        (size_label)
                    </span>
                </span>
            </label>

            <button
                class="btn btn-ghost btn-sm mr-2 text-error hover:bg-error hover:text-error-content"
                type="button"
                aria-label=(delete_label)
                hx-post=(delete_path)
                hx-target="#video-workspace"
                hx-swap="outerHTML"
                hx-confirm=(delete_confirm)>
                "Delete"
            </button>
        </div>
    }
}

fn megabyte_label(bytes: u64) -> String {
    format!("{:.2} MB", bytes as f64 / 1_000_000.0)
}

fn recent_analyses(
    analyses: &[db::analysis::Analysis],
    videos: &[db::video::Video],
) -> impl Renderable {
    rsx! {
        <section class="space-y-4">
            <div class="flex flex-wrap items-end justify-between gap-3">
                <div class="space-y-2">
                    <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">
                        "History"
                    </p>
                    <h2 class="text-2xl font-semibold text-base-content">"Recent analyses"</h2>
                </div>
                <a class="btn btn-sm btn-outline" href="/analyses">"View all"</a>
            </div>

            (super::analyses::analysis_history(analyses, videos))
        </section>
    }
}

pub async fn healthz() -> (StatusCode, &'static str) {
    (StatusCode::OK, "OK")
}
