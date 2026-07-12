//! Server-rendered form and HTMX fragment for video cost estimates.

use std::collections::BTreeMap;

use async_trait::async_trait;
use axum_extra::extract::Form;
use hypertext::prelude::*;
use serde::Deserialize;

use crate::{
    analysis::cost::{
        AnalysisQuality, CostScenario, EstimateBand, ModelEstimate, ResponseProfile, SamplingRate,
        SourceResolution, estimate_all,
    },
    app::AppState,
    http::ui::{Public, Route, RouteContext, RouteError, RouteView, not_found_fragment},
};

const RESOLUTIONS: &[(&str, &str)] = &[
    ("360p", "360p"),
    ("480p", "480p"),
    ("720p", "720p"),
    ("1080p", "1080p"),
    ("2160p", "2160p / 4K"),
];
const QUALITIES: &[(&str, &str)] = &[
    ("current", "Current Leo"),
    ("low", "Low"),
    ("standard", "Standard"),
    ("high", "High"),
];
const SAMPLING: &[(&str, &str)] = &[
    ("0.1", "0.1 fps"),
    ("0.2", "0.2 fps"),
    ("0.5", "0.5 fps"),
    ("1", "1 fps"),
    ("2", "2 fps"),
    ("4", "4 fps"),
    ("8", "8 fps"),
];
const RESPONSES: &[(&str, &str)] = &[
    ("concise", "Concise"),
    ("standard", "Standard"),
    ("detailed", "Detailed"),
];

pub struct CostEstimateRoute;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct CostCalculatorInput {
    #[serde(default)]
    _cost: Option<String>,
    #[serde(default)]
    video_count: Option<String>,
    #[serde(default)]
    duration_minutes: Option<String>,
    #[serde(default)]
    source_resolution: Option<String>,
    #[serde(default)]
    audio_present: Option<String>,
    #[serde(default)]
    analysis_quality: Option<String>,
    #[serde(default)]
    sampling_fps: Option<String>,
    #[serde(default)]
    response_profile: Option<String>,
    #[serde(default)]
    prompt_tokens: Option<String>,
}

pub struct CostCalculatorView {
    form: FormState,
    estimates: Vec<ModelEstimate>,
}

struct FormState {
    video_count: String,
    duration_minutes: String,
    resolution: String,
    audio_present: bool,
    audio_raw: Option<String>,
    quality: String,
    sampling: String,
    response: String,
    prompt_tokens: String,
    errors: BTreeMap<&'static str, &'static str>,
}

impl CostCalculatorView {
    pub(super) fn from_input(input: CostCalculatorInput) -> Self {
        let defaults = CostScenario::default();
        let submitted = input._cost.is_some();
        let value = |raw: Option<String>, default: &str| {
            if submitted {
                raw.unwrap_or_else(|| default.to_owned())
            } else {
                default.to_owned()
            }
        };
        let video_count = value(input.video_count, &defaults.video_count.to_string());
        let duration_minutes = value(
            input.duration_minutes,
            &defaults.duration_minutes.to_string(),
        );
        let resolution = value(input.source_resolution, defaults.resolution.value());
        let quality = value(input.analysis_quality, defaults.quality.value());
        let sampling = value(input.sampling_fps, defaults.sampling.value());
        let response = value(input.response_profile, defaults.response.value());
        let prompt_tokens = value(input.prompt_tokens, &defaults.prompt_tokens.to_string());
        let audio_raw = submitted.then_some(input.audio_present).flatten();
        let mut errors = BTreeMap::new();
        let audio_present = if !submitted {
            true
        } else {
            match audio_raw.as_deref() {
                None | Some("0" | "false" | "off" | "no") => false,
                Some("1" | "true" | "on" | "yes") => true,
                Some(_) => {
                    errors.insert("audio_present", "Choose whether source audio is present.");
                    false
                }
            }
        };

        let videos = parse(
            &video_count,
            "video_count",
            "Enter a whole number of videos.",
            &mut errors,
        );
        let duration = parse(
            &duration_minutes,
            "duration_minutes",
            "Enter whole minutes per video.",
            &mut errors,
        );
        let prompt = parse(
            &prompt_tokens,
            "prompt_tokens",
            "Enter a whole number of prompt tokens.",
            &mut errors,
        );
        let parsed_resolution = parse_choice(
            SourceResolution::parse(&resolution),
            "source_resolution",
            "Choose a listed source resolution.",
            &mut errors,
        );
        let parsed_quality = parse_choice(
            AnalysisQuality::parse(&quality),
            "analysis_quality",
            "Choose a listed analysis quality.",
            &mut errors,
        );
        let parsed_sampling = parse_choice(
            SamplingRate::parse(&sampling),
            "sampling_fps",
            "Choose a listed frame sampling rate.",
            &mut errors,
        );
        let parsed_response = parse_choice(
            ResponseProfile::parse(&response),
            "response_profile",
            "Choose a listed response profile.",
            &mut errors,
        );

        let scenario = match (
            videos,
            duration,
            prompt,
            parsed_resolution,
            parsed_quality,
            parsed_sampling,
            parsed_response,
        ) {
            (
                Some(video_count),
                Some(duration_minutes),
                Some(prompt_tokens),
                Some(resolution),
                Some(quality),
                Some(sampling),
                Some(response),
            ) => {
                let scenario = CostScenario {
                    video_count,
                    duration_minutes,
                    resolution,
                    audio_present,
                    quality,
                    sampling,
                    response,
                    prompt_tokens,
                };
                for violation in scenario.violations() {
                    errors.entry(violation.field).or_insert(violation.message);
                }
                errors.is_empty().then_some(scenario)
            }
            _ => None,
        };
        let estimates = scenario.as_ref().map_or_else(Vec::new, estimate_all);
        Self {
            form: FormState {
                video_count,
                duration_minutes,
                resolution,
                audio_present,
                audio_raw,
                quality,
                sampling,
                response,
                prompt_tokens,
                errors,
            },
            estimates,
        }
    }

    pub(super) fn render(&self) -> impl Renderable {
        rsx! {
            <section id="cost-calculator" class="space-y-6 rounded-box border border-base-300 bg-base-100 p-6 shadow-sm">
                <header class="space-y-2">
                    <p class="text-sm font-semibold uppercase tracking-[0.22em] text-base-content/60">"Cost calculator"</p>
                    <h2 class="text-2xl font-semibold">"Compare video-analysis estimates"</h2>
                    <p class="max-w-3xl text-sm text-base-content/70">
                        "Model one scenario across Leo's five default models. Nothing is uploaded and no provider API is called."
                    </p>
                </header>
                <div class="grid gap-6 lg:grid-cols-[minmax(16rem,0.7fr)_minmax(0,1.3fr)]">
                    (self.render_form())
                    (self.render_results())
                </div>
                <p class="text-xs leading-5 text-base-content/60">
                    "Prices are a public-list-price snapshot checked 2026-07-09. Frame extraction, compression, overlap, prompt wrappers, and output are modeled; retries, discounts, and hidden output can change actual cost."
                </p>
            </section>
        }
    }

    fn render_form(&self) -> impl Renderable {
        let form = &self.form;
        rsx! {
            <form
                class="space-y-4"
                method="get"
                action="/#cost-calculator"
                novalidate="novalidate"
                hx-post="/cost/estimate"
                hx-target="#cost-calculator"
                hx-swap="outerHTML"
                hx-trigger="submit, input delay:500ms, change delay:200ms"
                hx-sync="this:replace">
                <input type="hidden" name="_cost" value="1" />
                <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
                    (number_field("cost-video-count", "Number of videos", "video_count", &form.video_count, "1", "10", form.error("video_count")))
                    (number_field("cost-duration", "Minutes per video", "duration_minutes", &form.duration_minutes, "1", "1440", form.error("duration_minutes")))
                </div>
                (select_field("cost-resolution", "Source resolution", "source_resolution", &form.resolution, RESOLUTIONS, form.error("source_resolution")))
                <div class="space-y-1">
                    <label for="cost-audio" class="flex cursor-pointer items-center gap-3 text-sm font-medium">
                        @if form.audio_present {
                            <input id="cost-audio" class="checkbox checkbox-primary" type="checkbox" name="audio_present" value="1" checked="checked" />
                        } @else {
                            <input id="cost-audio" class="checkbox checkbox-primary" type="checkbox" name="audio_present" value="1" />
                        }
                        <span>"Source audio is present"</span>
                    </label>
                    @if let Some(error) = form.error("audio_present") {
                        <p class="text-xs text-error" role="alert">(error)
                            @if let Some(raw) = form.audio_raw.as_deref() { " Submitted value: " (raw) }
                        </p>
                    }
                </div>
                (select_field("cost-quality", "Analysis quality", "analysis_quality", &form.quality, QUALITIES, form.error("analysis_quality")))
                (select_field("cost-sampling", "Frame sampling", "sampling_fps", &form.sampling, SAMPLING, form.error("sampling_fps")))
                (select_field("cost-response", "Response profile", "response_profile", &form.response, RESPONSES, form.error("response_profile")))
                @if form.error("prompt_tokens").is_some() {
                    <details class="rounded-box border border-error/40 p-3" open="open" data-advanced-assumptions="advanced">
                        <summary class="cursor-pointer text-sm font-semibold">"Advanced assumptions · Needs attention"</summary>
                        <div class="mt-3">(number_field("cost-prompt-tokens", "Prompt input tokens", "prompt_tokens", &form.prompt_tokens, "0", "1000000", form.error("prompt_tokens")))</div>
                    </details>
                } @else {
                    <details class="rounded-box border border-base-300 p-3" data-advanced-assumptions="advanced">
                        <summary class="cursor-pointer text-sm font-semibold">"Advanced assumptions"</summary>
                        <div class="mt-3">(number_field("cost-prompt-tokens", "Prompt input tokens", "prompt_tokens", &form.prompt_tokens, "0", "1000000", None))</div>
                    </details>
                }
                <button class="btn btn-primary" type="submit">"Calculate"</button>
            </form>
        }
    }

    fn render_results(&self) -> impl Renderable {
        rsx! {
            <div class="space-y-4" aria-live="polite">
                <div>
                    <h3 class="text-lg font-semibold">"Model comparison"</h3>
                    <p class="text-sm text-base-content/60">"Complete typical totals are ranked first."</p>
                </div>
                @if self.estimates.is_empty() {
                    <p class="rounded-box border border-error/40 bg-error/5 p-4 text-sm text-error" role="alert">
                        "Fix the highlighted assumptions to calculate model results."
                    </p>
                } @else {
                    <div class="space-y-3">
                        @for estimate in self.estimates.iter() { (estimate_card(estimate)) }
                    </div>
                }
            </div>
        }
    }
}

impl FormState {
    fn error(&self, field: &'static str) -> Option<&'static str> {
        self.errors.get(field).copied()
    }
}

#[async_trait]
impl Route for CostEstimateRoute {
    type Input = Form<CostCalculatorInput>;
    type Authz = Public;
    type View = CostCalculatorView;

    async fn handle(
        _context: &RouteContext,
        _granted: (),
        Form(input): Self::Input,
    ) -> Result<Self::View, RouteError> {
        Ok(CostCalculatorView::from_input(input))
    }
}

impl RouteView for CostCalculatorView {
    fn document(&self, _state: &AppState) -> impl Renderable {
        not_found_fragment()
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        self.render()
    }
}

fn parse<T: std::str::FromStr>(
    raw: &str,
    field: &'static str,
    message: &'static str,
    errors: &mut BTreeMap<&'static str, &'static str>,
) -> Option<T> {
    raw.parse().ok().or_else(|| {
        errors.insert(field, message);
        None
    })
}

fn parse_choice<T>(
    value: Option<T>,
    field: &'static str,
    message: &'static str,
    errors: &mut BTreeMap<&'static str, &'static str>,
) -> Option<T> {
    value.or_else(|| {
        errors.insert(field, message);
        None
    })
}

fn number_field<'a>(
    id: &'a str,
    label: &'a str,
    name: &'a str,
    value: &'a str,
    min: &'a str,
    max: &'a str,
    error: Option<&'a str>,
) -> impl Renderable + 'a {
    rsx! {
        <div class="space-y-1">
            <label for=(id) class="text-sm font-medium">(label)</label>
            <input id=(id) class="input input-bordered w-full" type="number" name=(name) min=(min) max=(max) step="1" value=(value) aria-invalid=(if error.is_some() { "true" } else { "false" }) />
            @if let Some(error) = error { <p class="text-xs text-error" role="alert">(error)</p> }
        </div>
    }
}

fn select_field<'a>(
    id: &'a str,
    label: &'a str,
    name: &'a str,
    selected: &'a str,
    options: &'a [(&'a str, &'a str)],
    error: Option<&'a str>,
) -> impl Renderable + 'a {
    rsx! {
        <div class="space-y-1">
            <label for=(id) class="text-sm font-medium">(label)</label>
            <select id=(id) class="select select-bordered w-full" name=(name) aria-invalid=(if error.is_some() { "true" } else { "false" })>
                @if !options.iter().any(|(value, _)| *value == selected) {
                    <option value=(selected) selected="selected">("Submitted value: ")(selected)</option>
                }
                @for (value, option_label) in options.iter().copied() { (select_option(value, option_label, value == selected)) }
            </select>
            @if let Some(error) = error { <p class="text-xs text-error" role="alert">(error)</p> }
        </div>
    }
}

fn select_option<'a>(value: &'a str, label: &'a str, selected: bool) -> impl Renderable + 'a {
    rsx! {
        @if selected { <option value=(value) selected="selected">(label)</option> }
        @else { <option value=(value)>(label)</option> }
    }
}

fn estimate_card(estimate: &ModelEstimate) -> impl Renderable {
    let typical = &estimate.bands.typical;
    rsx! {
        <article class="space-y-3 rounded-box border border-base-300 p-4" data-model-result=(estimate.model_id)>
            <div class="flex flex-wrap items-start justify-between gap-2">
                <div>
                    <h4 class="font-semibold">(estimate.display_name)</h4>
                    <p class="break-all font-mono text-xs text-base-content/60">(estimate.model_id)</p>
                    <p class="text-xs text-base-content/60">(estimate.provider) @if estimate.local { " · local" } @else { " · hosted" }</p>
                </div>
                <span class="badge badge-outline">(estimate.state_label())</span>
            </div>
            <div class="grid grid-cols-3 gap-2 text-center">
                (band_price("Low", &estimate.bands.low, false))
                (band_price("Typical", typical, true))
                (band_price("High", &estimate.bands.high, false))
            </div>
            @if estimate.local {
                <p class="rounded-box bg-base-200 p-3 text-sm">(crate::analysis::cost::LOCAL_COST_WARNING)</p>
            } @else if typical.total.is_none() && typical.known_subtotal.as_nanos() > 0 {
                <p class="rounded-box bg-warning/10 p-3 text-sm">
                    "Known priced subtotal (typical): " <strong>(typical.known_subtotal.format_usd())</strong>
                    ". Complete total unavailable because image-token pricing is not published."
                </p>
            }
            <dl class="grid grid-cols-[minmax(0,1fr)_auto] gap-x-4 gap-y-1 rounded-box bg-base-200/50 p-3 text-xs text-base-content/70" data-model-detail=(estimate.model_id)>
                <dt>"Typical source video"</dt><dd>(typical.usage.source_seconds)(" seconds")</dd>
                @if let Some(unique) = typical.usage.unique_frames {
                    <dt>"Modeled frames"</dt><dd>(unique)(" unique · ")(typical.usage.billed_frames.unwrap_or(unique))(" billed")</dd>
                }
                <dt>"Modeled requests"</dt><dd>(typical.usage.requests)</dd>
                <dt>"Text input units"</dt><dd>(typical.usage.text_units)</dd>
                <dt>"Image / video units"</dt><dd>@if let Some(media) = typical.usage.media_units { (media) } @else { "Not cataloged" }</dd>
                @if typical.usage.audio_units > 0 { <dt>"Audio units"</dt><dd>(typical.usage.audio_units)</dd> }
                <dt>"Output units"</dt><dd>(typical.usage.output_units)</dd>
            </dl>
            @if let Some(total) = typical.total {
                <p class="text-sm text-base-content/70">"Typical cost per video: " <strong>(total.per_video(estimate.video_count).format_usd())</strong></p>
            }
            <ul class="list-disc space-y-1 pl-5 text-xs leading-5 text-base-content/60">
                @for warning in estimate.warnings.iter() { <li>(warning)</li> }
            </ul>
            <a class="link link-primary text-xs" href=(estimate.source_url) target="_blank" rel="noreferrer">(estimate.source_label)</a>
        </article>
    }
}

fn band_price(label: &'static str, band: &EstimateBand, emphasized: bool) -> impl Renderable {
    let class = if emphasized {
        "rounded-box border border-primary/40 bg-primary/10 p-2"
    } else {
        "rounded-box bg-base-200 p-2"
    };
    rsx! {
        <div class=(class)>
            <p class="text-xs text-base-content/60">(label)</p>
            @if let Some(total) = band.total { <p class="font-semibold">(total.format_usd())</p> }
            @else if let Some(reason) = band.limitation.as_deref() {
                <p class="text-xs font-semibold text-warning">"Unavailable"</p>
                <p class="mt-1 text-left text-[0.7rem] leading-4 text-warning">(reason)</p>
            }
            @else if band.known_subtotal.as_nanos() > 0 { <p class="text-xs font-semibold">("Known ")(band.known_subtotal.format_usd())</p> }
            @else { <p class="text-xs text-warning">"Not priced"</p> }
        </div>
    }
}
