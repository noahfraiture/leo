//! Deterministic cost estimates for the five models exposed by the UI.
//!
//! Prices and media assumptions are a checked-in snapshot. The calculator is
//! intentionally independent from provider clients: it never uploads media or
//! calls a remote API.

use std::cmp::Ordering;

pub const MAX_VIDEO_COUNT: u32 = 10;
pub const MAX_DURATION_MINUTES: u32 = 1_440;
pub const MAX_PROMPT_TOKENS: u64 = 1_000_000;
pub const MAX_UNIQUE_FRAMES: u64 = 250_000;
pub const LOCAL_COST_WARNING: &str =
    "No cataloged vendor fee; configured endpoint and operating costs not estimated.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Band<T> {
    pub low: T,
    pub typical: T,
    pub high: T,
}

impl<T> Band<T> {
    const fn new(low: T, typical: T, high: T) -> Self {
        Self { low, typical, high }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Money(u128);

impl Money {
    const ZERO: Self = Self(0);

    pub const fn as_nanos(self) -> u128 {
        self.0
    }

    fn units(units: u64, nano_usd_per_unit: u128) -> Self {
        Self(u128::from(units) * nano_usd_per_unit)
    }

    fn plus(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }

    fn times(self, multiplier: u64) -> Self {
        Self(self.0 * u128::from(multiplier))
    }

    pub fn per_video(self, videos: u32) -> Self {
        Self(self.0 / u128::from(videos.max(1)))
    }

    pub fn format_usd(self) -> String {
        if self.0 >= 1_000_000_000 {
            let cents = (self.0 + 5_000_000) / 10_000_000;
            format!("${}.{:02}", cents / 100, cents % 100)
        } else {
            let micros = (self.0 + 500) / 1_000;
            if micros >= 1_000_000 {
                "$1.00".to_owned()
            } else {
                format!("$0.{micros:06}")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceResolution {
    P360,
    P480,
    P720,
    P1080,
    P2160,
}

impl SourceResolution {
    pub const ALL: [Self; 5] = [Self::P360, Self::P480, Self::P720, Self::P1080, Self::P2160];

    pub const fn value(self) -> &'static str {
        match self {
            Self::P360 => "360p",
            Self::P480 => "480p",
            Self::P720 => "720p",
            Self::P1080 => "1080p",
            Self::P2160 => "2160p",
        }
    }

    pub const fn dimensions(self) -> (u64, u64) {
        match self {
            Self::P360 => (640, 360),
            Self::P480 => (854, 480),
            Self::P720 => (1_280, 720),
            Self::P1080 => (1_920, 1_080),
            Self::P2160 => (3_840, 2_160),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.value() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisQuality {
    Current,
    Low,
    Standard,
    High,
}

impl AnalysisQuality {
    pub const ALL: [(Self, &'static str); 4] = [
        (Self::Current, "Current Leo"),
        (Self::Low, "Low"),
        (Self::Standard, "Standard"),
        (Self::High, "High"),
    ];

    pub const fn value(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Low => "low",
            Self::Standard => "standard",
            Self::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .map(|(item, _)| item)
            .find(|item| item.value() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SamplingRate {
    Fps0_1,
    Fps0_2,
    Fps0_5,
    Fps1,
    Fps2,
    Fps4,
    Fps8,
}

impl SamplingRate {
    pub const ALL: [Self; 7] = [
        Self::Fps0_1,
        Self::Fps0_2,
        Self::Fps0_5,
        Self::Fps1,
        Self::Fps2,
        Self::Fps4,
        Self::Fps8,
    ];

    pub const fn value(self) -> &'static str {
        match self {
            Self::Fps0_1 => "0.1",
            Self::Fps0_2 => "0.2",
            Self::Fps0_5 => "0.5",
            Self::Fps1 => "1",
            Self::Fps2 => "2",
            Self::Fps4 => "4",
            Self::Fps8 => "8",
        }
    }

    pub const fn milli_fps(self) -> u64 {
        match self {
            Self::Fps0_1 => 100,
            Self::Fps0_2 => 200,
            Self::Fps0_5 => 500,
            Self::Fps1 => 1_000,
            Self::Fps2 => 2_000,
            Self::Fps4 => 4_000,
            Self::Fps8 => 8_000,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|item| item.value() == value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseProfile {
    Concise,
    Standard,
    Detailed,
}

impl ResponseProfile {
    pub const ALL: [(Self, &'static str); 3] = [
        (Self::Concise, "Concise"),
        (Self::Standard, "Standard"),
        (Self::Detailed, "Detailed"),
    ];

    pub const fn value(self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Standard => "standard",
            Self::Detailed => "detailed",
        }
    }

    fn output(self) -> Band<u64> {
        match self {
            Self::Concise => Band::new(128, 256, 512),
            Self::Standard => Band::new(256, 512, 1_024),
            Self::Detailed => Band::new(512, 1_024, 2_048),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .map(|(item, _)| item)
            .find(|item| item.value() == value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScenarioViolation {
    pub field: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CostScenario {
    pub video_count: u32,
    pub duration_minutes: u32,
    pub resolution: SourceResolution,
    pub audio_present: bool,
    pub quality: AnalysisQuality,
    pub sampling: SamplingRate,
    pub response: ResponseProfile,
    pub prompt_tokens: u64,
}

impl Default for CostScenario {
    fn default() -> Self {
        Self {
            video_count: 1,
            duration_minutes: 10,
            resolution: SourceResolution::P1080,
            audio_present: true,
            quality: AnalysisQuality::Current,
            sampling: SamplingRate::Fps0_2,
            response: ResponseProfile::Standard,
            prompt_tokens: 250,
        }
    }
}

impl CostScenario {
    pub fn duration_seconds(&self) -> u64 {
        u64::from(self.duration_minutes) * 60
    }

    pub fn frames_per_video(&self) -> u64 {
        (self.duration_seconds() * self.sampling.milli_fps()).div_ceil(1_000)
    }

    pub fn unique_frames(&self) -> u64 {
        self.frames_per_video()
            .saturating_mul(u64::from(self.video_count))
    }

    pub fn violations(&self) -> Vec<ScenarioViolation> {
        let mut errors = Vec::new();
        if !(1..=MAX_VIDEO_COUNT).contains(&self.video_count) {
            errors.push(ScenarioViolation {
                field: "video_count",
                message: "Use 1 to 10 videos.",
            });
        }
        if !(1..=MAX_DURATION_MINUTES).contains(&self.duration_minutes) {
            errors.push(ScenarioViolation {
                field: "duration_minutes",
                message: "Use 1 to 1,440 minutes.",
            });
        }
        if self.prompt_tokens > MAX_PROMPT_TOKENS {
            errors.push(ScenarioViolation {
                field: "prompt_tokens",
                message: "Use 0 to 1,000,000 prompt tokens.",
            });
        }
        if self.unique_frames() > MAX_UNIQUE_FRAMES {
            errors.push(ScenarioViolation {
                field: "sampling_fps",
                message: "This scenario exceeds 250,000 sampled frames; lower videos, duration, or fps.",
            });
        }
        errors
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub source_seconds: u64,
    pub unique_frames: Option<u64>,
    pub billed_frames: Option<u64>,
    pub requests: u64,
    pub text_units: u64,
    pub media_units: Option<u64>,
    pub audio_units: u64,
    pub output_units: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EstimateBand {
    pub total: Option<Money>,
    pub known_subtotal: Money,
    pub usage: Usage,
    pub limitation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelEstimate {
    pub model_id: &'static str,
    pub display_name: &'static str,
    pub provider: &'static str,
    pub local: bool,
    pub video_count: u32,
    pub bands: Band<EstimateBand>,
    pub warnings: Vec<&'static str>,
    pub source_label: &'static str,
    pub source_url: &'static str,
}

impl ModelEstimate {
    pub fn state_label(&self) -> &'static str {
        let available = [&self.bands.low, &self.bands.typical, &self.bands.high]
            .into_iter()
            .filter(|band| band.limitation.is_none())
            .count();
        let priced = [&self.bands.low, &self.bands.typical, &self.bands.high]
            .into_iter()
            .filter(|band| band.total.is_some())
            .count();
        match (available, priced) {
            (0, _) => "Unsupported",
            (_, 3) => "Estimated",
            (_, 1 | 2) => "Partial",
            _ => "Unpriced",
        }
    }
}

#[derive(Clone, Copy)]
enum Level {
    Low,
    Typical,
    High,
}

impl Level {
    fn pick(self, values: Band<u64>) -> u64 {
        match self {
            Self::Low => values.low,
            Self::Typical => values.typical,
            Self::High => values.high,
        }
    }
}

/// Returns the five fixed model estimates, with complete typical totals first.
pub fn estimate_all(scenario: &CostScenario) -> Vec<ModelEstimate> {
    debug_assert!(scenario.violations().is_empty());
    let mut estimates = vec![
        estimate_gemini(scenario),
        estimate_openai(scenario),
        estimate_local(
            scenario,
            "google/gemma-4-26b-a4b",
            "Gemma 4 26B A4B",
            "Gemma",
        ),
        estimate_local(scenario, "qwen/qwen3.6-35b-a3b", "Qwen 3.6 35B A3B", "Qwen"),
        estimate_mistral(scenario),
    ];
    estimates.sort_by(
        |left, right| match (left.bands.typical.total, right.bands.typical.total) {
            (Some(left), Some(right)) => left.cmp(&right),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
    );
    estimates
}

fn estimate_gemini(scenario: &CostScenario) -> ModelEstimate {
    let estimate = |level: Level| {
        let source_seconds = scenario.duration_seconds() * u64::from(scenario.video_count);
        let video_rate = if scenario.quality == AnalysisQuality::Low {
            68
        } else {
            268
        };
        let duration_limit = if scenario.quality == AnalysisQuality::Low {
            10_800
        } else {
            3_600
        };
        let text = scenario.prompt_tokens + level.pick(Band::new(8, 16, 32));
        let video = source_seconds * video_rate;
        let audio = if scenario.audio_present {
            source_seconds * 32
        } else {
            0
        };
        let output = level.pick(scenario.response.output());
        let usage = Usage {
            source_seconds,
            requests: 1,
            text_units: text,
            media_units: Some(video),
            audio_units: audio,
            output_units: output,
            ..Usage::default()
        };
        let limitation = if source_seconds > duration_limit {
            Some(format!(
                "Aggregate duration is {source_seconds}s; this quality is limited to {duration_limit}s."
            ))
        } else if text + video + audio + output > 1_048_576 {
            Some("Modeled input and output exceed Gemini's 1,048,576-token context.".to_owned())
        } else {
            None
        };
        let known = Money::units(text + video, 500)
            .plus(Money::units(audio, 1_000))
            .plus(Money::units(output, 3_000));
        EstimateBand {
            total: limitation.is_none().then_some(known),
            known_subtotal: known,
            usage,
            limitation,
        }
    };
    ModelEstimate {
        model_id: "gemini-3-flash-preview",
        display_name: "Gemini 3 Flash Preview",
        provider: "Gemini",
        local: false,
        video_count: scenario.video_count,
        bands: Band::new(
            estimate(Level::Low),
            estimate(Level::Typical),
            estimate(Level::High),
        ),
        warnings: vec![
            "Video units are inferred from the provider's rounded video documentation.",
            "Modeled output assumes no additional hidden output; actual hidden output can increase cost.",
        ],
        source_label: "Gemini pricing and video units",
        source_url: "https://ai.google.dev/gemini-api/docs/pricing",
    }
}

#[derive(Clone, Copy)]
struct ChunkPlan {
    capacity: u64,
    full_chunks: u64,
    last_frames: u64,
    chunks: u64,
    billed: u64,
}

fn chunk_plan(unique: u64, video_count: u32, payload_bytes: u64, max_images: u64) -> ChunkPlan {
    const MAX_PAYLOAD: u64 = 45 * 1_024 * 1_024;
    let capacity = max_images
        .min((MAX_PAYLOAD / payload_bytes.max(1)).max(1))
        .max(1);
    let video_count = u64::from(video_count);
    let (mut start, mut next, mut chunks, mut billed, mut last_frames): (u64, u64, u64, u64, u64) =
        (0, 0, 0, 0, 0);

    while next < unique {
        let retained = next - start;
        let added = (capacity - retained).min(unique - next);
        let end = next + added - 1;
        last_frames = retained + added;
        chunks += 1;
        billed += last_frames;
        next += added;
        if next == unique {
            break;
        }

        let start_group = start / video_count;
        let end_group = end / video_count;
        let overlap = if capacity <= 1 || end_group <= start_group {
            0
        } else {
            let first_group = (end_group * 100 - (end_group - start_group) * 10).div_ceil(100);
            (end - (first_group * video_count).max(start) + 1).min(capacity - 1)
        };
        start = next - overlap;
    }

    ChunkPlan {
        capacity,
        full_chunks: chunks.saturating_sub(1),
        last_frames,
        chunks,
        billed,
    }
}

fn payload_bytes(resolution: SourceResolution, level: Level) -> u64 {
    let (width, height) = resolution.dimensions();
    let jpeg = (width * height * level.pick(Band::new(4, 10, 25))).div_ceil(100);
    jpeg.div_ceil(3) * 4 + 215
}

fn image_units(resolution: SourceResolution, quality: AnalysisQuality) -> u64 {
    match quality {
        AnalysisQuality::Current | AnalysisQuality::Low => 256,
        AnalysisQuality::Standard => patch_count(resolution, 2_500, 2_048),
        AnalysisQuality::High => patch_count(resolution, 10_000, 6_000),
    }
}

fn patch_count(resolution: SourceResolution, budget: u64, max_dimension: u64) -> u64 {
    let (mut width, mut height) = resolution.dimensions();
    let long = width.max(height);
    if long > max_dimension {
        width = width * max_dimension / long;
        height = height * max_dimension / long;
    }
    let patches = |long_edge: u64| {
        let current = width.max(height);
        let scaled_width = (width * long_edge / current).max(1);
        let scaled_height = (height * long_edge / current).max(1);
        scaled_width.div_ceil(32) * scaled_height.div_ceil(32)
    };
    if patches(width.max(height)) <= budget {
        return patches(width.max(height));
    }
    let (mut low, mut high) = (1, width.max(height));
    while low < high {
        let midpoint = low + (high - low).div_ceil(2);
        if patches(midpoint) <= budget {
            low = midpoint
        } else {
            high = midpoint - 1
        }
    }
    patches(low)
}

fn sampled_usage(scenario: &CostScenario, level: Level, max_images: u64) -> (Usage, ChunkPlan) {
    let unique = scenario.unique_frames();
    let plan = chunk_plan(
        unique,
        scenario.video_count,
        payload_bytes(scenario.resolution, level),
        max_images,
    );
    let output = level.pick(scenario.response.output());
    let wrapper = level.pick(Band::new(64, 96, 128));
    let metadata = level.pick(Band::new(12, 16, 24));
    let summary_wrapper = level.pick(Band::new(48, 64, 96));
    let evidence_text = plan.chunks * (scenario.prompt_tokens + wrapper) + plan.billed * metadata;
    let summary_text = scenario.prompt_tokens + summary_wrapper + plan.chunks * output;
    (
        Usage {
            source_seconds: scenario.duration_seconds() * u64::from(scenario.video_count),
            unique_frames: Some(unique),
            billed_frames: Some(plan.billed),
            requests: plan.chunks + 1,
            text_units: evidence_text + summary_text,
            media_units: None,
            output_units: (plan.chunks + 1) * output,
            ..Usage::default()
        },
        plan,
    )
}

fn openai_request_cost(text: u64, media: u64, output: u64) -> (Money, bool) {
    let input = text + media;
    let tiered = input > 272_000;
    let input_cost = Money::units(input, if tiered { 10_000 } else { 5_000 });
    let output_cost = Money::units(output, if tiered { 45_000 } else { 30_000 });
    (input_cost.plus(output_cost), input + output <= 1_050_000)
}

fn estimate_openai(scenario: &CostScenario) -> ModelEstimate {
    let estimate = |level: Level| {
        let (mut usage, plan) = sampled_usage(scenario, level, 450);
        let output = level.pick(scenario.response.output());
        let wrapper = level.pick(Band::new(64, 96, 128));
        let metadata = level.pick(Band::new(12, 16, 24));
        let media_per_frame = image_units(scenario.resolution, scenario.quality);
        let request_cost = |frames| {
            openai_request_cost(
                scenario.prompt_tokens + wrapper + frames * metadata,
                frames * media_per_frame,
                output,
            )
        };
        let (full_cost, full_fits) = request_cost(plan.capacity);
        let (last_cost, last_fits) = request_cost(plan.last_frames);
        let summary_text =
            scenario.prompt_tokens + level.pick(Band::new(48, 64, 96)) + plan.chunks * output;
        let (summary_cost, summary_fits) = openai_request_cost(summary_text, 0, output);
        let known = full_cost
            .times(plan.full_chunks)
            .plus(last_cost)
            .plus(summary_cost);
        usage.media_units = Some(plan.billed * media_per_frame);
        let full_fits = plan.full_chunks == 0 || full_fits;
        let limitation = (!(full_fits && last_fits && summary_fits)).then(|| {
            "At least one modeled request exceeds GPT-5.5's 1,050,000-token context.".to_owned()
        });
        EstimateBand {
            total: limitation.is_none().then_some(known),
            known_subtotal: known,
            usage,
            limitation,
        }
    };
    let mut warnings = vec![
        "Frame count, JPEG size, payload splitting, and overlap are modeled estimates.",
        "Modeled output assumes no additional hidden output; actual hidden output can increase cost.",
    ];
    if scenario.audio_present {
        warnings.push("Source audio is ignored by this sampled-frame pipeline.");
    }
    ModelEstimate {
        model_id: "gpt-5.5",
        display_name: "GPT-5.5",
        provider: "OpenAI",
        local: false,
        video_count: scenario.video_count,
        bands: Band::new(
            estimate(Level::Low),
            estimate(Level::Typical),
            estimate(Level::High),
        ),
        warnings,
        source_label: "GPT-5.5 pricing and image tokens",
        source_url: "https://developers.openai.com/api/docs/models/gpt-5.5",
    }
}

fn estimate_local(
    scenario: &CostScenario,
    model_id: &'static str,
    display_name: &'static str,
    provider: &'static str,
) -> ModelEstimate {
    let estimate = |level: Level| {
        let (usage, _) = sampled_usage(scenario, level, 450);
        EstimateBand {
            usage,
            ..EstimateBand::default()
        }
    };
    let mut warnings = vec![
        LOCAL_COST_WARNING,
        "Frame count, JPEG size, payload splitting, and overlap are modeled estimates.",
    ];
    if scenario.audio_present {
        warnings.push("Source audio is ignored by this sampled-frame pipeline.");
    }
    ModelEstimate {
        model_id,
        display_name,
        provider,
        local: true,
        video_count: scenario.video_count,
        bands: Band::new(
            estimate(Level::Low),
            estimate(Level::Typical),
            estimate(Level::High),
        ),
        warnings,
        source_label: "LM Studio local server",
        source_url: "https://lmstudio.ai/docs/developer/core/server",
    }
}

fn estimate_mistral(scenario: &CostScenario) -> ModelEstimate {
    let estimate = |level: Level| {
        let (usage, plan) = sampled_usage(scenario, level, 8);
        let output = level.pick(scenario.response.output());
        let wrapper = level.pick(Band::new(64, 96, 128));
        let metadata = level.pick(Band::new(12, 16, 24));
        let summary =
            scenario.prompt_tokens + level.pick(Band::new(48, 64, 96)) + plan.chunks * output;
        let evidence_input = scenario.prompt_tokens + wrapper + plan.capacity * metadata;
        let known =
            Money::units(usage.text_units, 1_500).plus(Money::units(usage.output_units, 7_500));
        let limitation =
            (evidence_input + output > 256_000 || summary + output > 256_000).then(|| {
                "Known text and output alone exceed Mistral's 256,000-token context.".to_owned()
            });
        EstimateBand {
            total: None,
            known_subtotal: known,
            usage,
            limitation,
        }
    };
    let mut warnings = vec![
        "Image-token pricing is not published, so only known text and output subtotals are shown.",
        "Context compatibility cannot be confirmed without image-token usage.",
        "Analysis quality does not change Mistral image handling in this estimate.",
        "Frame count, JPEG size, payload splitting, and overlap are modeled estimates.",
        "Modeled output assumes no additional hidden output; actual hidden output can increase cost.",
    ];
    if scenario.audio_present {
        warnings.push("Source audio is ignored by this sampled-frame pipeline.");
    }
    ModelEstimate {
        model_id: "mistral-medium-latest",
        display_name: "Mistral Medium 3.5",
        provider: "Mistral",
        local: false,
        video_count: scenario.video_count,
        bands: Band::new(
            estimate(Level::Low),
            estimate(Level::Typical),
            estimate(Level::High),
        ),
        warnings,
        source_label: "Mistral API pricing",
        source_url: "https://mistral.ai/pricing/api/",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model<'a>(estimates: &'a [ModelEstimate], id: &str) -> &'a ModelEstimate {
        estimates
            .iter()
            .find(|estimate| estimate.model_id == id)
            .unwrap()
    }

    #[test]
    fn defaults_have_exact_hosted_totals_and_stable_ranking() {
        let estimates = estimate_all(&CostScenario::default());
        assert_eq!(estimates.len(), 5);
        assert_eq!(estimates[0].model_id, "gemini-3-flash-preview");
        assert_eq!(
            estimates[0].bands.typical.total.unwrap().as_nanos(),
            101_269_000
        );
        assert_eq!(estimates[1].model_id, "gpt-5.5");
        assert_eq!(
            estimates[1].bands.typical.total.unwrap().as_nanos(),
            199_780_000
        );
        assert_eq!(
            model(&estimates, "mistral-medium-latest")
                .bands
                .typical
                .usage
                .requests,
            18
        );
        assert!(
            model(&estimates, "mistral-medium-latest")
                .bands
                .typical
                .known_subtotal
                > Money::ZERO
        );
    }

    #[test]
    fn local_models_are_unpriced_and_explain_operating_costs() {
        for estimate in estimate_all(&CostScenario::default())
            .into_iter()
            .filter(|item| item.local)
        {
            assert_eq!(estimate.bands.typical.total, None);
            assert!(estimate.warnings.contains(&LOCAL_COST_WARNING));
        }
    }

    #[test]
    fn scenario_validation_covers_scalar_and_work_limits() {
        let scenario = CostScenario {
            video_count: 11,
            duration_minutes: 0,
            prompt_tokens: 1_000_001,
            sampling: SamplingRate::Fps8,
            ..CostScenario::default()
        };
        let fields = scenario
            .violations()
            .into_iter()
            .map(|item| item.field)
            .collect::<Vec<_>>();
        assert!(fields.contains(&"video_count"));
        assert!(fields.contains(&"duration_minutes"));
        assert!(fields.contains(&"prompt_tokens"));

        let too_many_frames = CostScenario {
            video_count: 10,
            duration_minutes: 60,
            sampling: SamplingRate::Fps8,
            ..CostScenario::default()
        };
        assert!(
            too_many_frames
                .violations()
                .iter()
                .any(|item| item.field == "sampling_fps")
        );
    }

    #[test]
    fn direct_video_duration_and_context_limits_are_visible() {
        let duration = CostScenario {
            video_count: 7,
            duration_minutes: 9,
            ..CostScenario::default()
        };
        assert_eq!(estimate_gemini(&duration).state_label(), "Unsupported");
        let context = CostScenario {
            duration_minutes: 60,
            quality: AnalysisQuality::Standard,
            ..CostScenario::default()
        };
        assert_eq!(estimate_gemini(&context).state_label(), "Unsupported");
    }

    #[test]
    fn openai_keeps_typical_price_when_only_high_band_exceeds_context() {
        let scenario = CostScenario {
            duration_minutes: 2,
            quality: AnalysisQuality::Standard,
            prompt_tokens: 1_000_000,
            ..CostScenario::default()
        };
        let estimate = estimate_openai(&scenario);
        assert!(estimate.bands.low.total.is_some());
        assert!(estimate.bands.typical.total.is_some());
        assert!(estimate.bands.high.total.is_none());
        assert_eq!(estimate.state_label(), "Partial");
    }

    #[test]
    fn default_chunk_models_match_frame_and_overlap_fixtures() {
        let scenario = CostScenario::default();
        assert_eq!(scenario.unique_frames(), 120);
        let openai = chunk_plan(
            120,
            1,
            payload_bytes(SourceResolution::P1080, Level::Typical),
            450,
        );
        assert_eq!((openai.chunks, openai.billed), (1, 120));
        let mistral = chunk_plan(
            120,
            1,
            payload_bytes(SourceResolution::P1080, Level::Typical),
            8,
        );
        assert_eq!((mistral.chunks, mistral.billed), (17, 136));

        let multi_video = chunk_plan(
            240,
            2,
            payload_bytes(SourceResolution::P1080, Level::Typical),
            8,
        );
        assert_eq!((multi_video.chunks, multi_video.billed), (40, 318));
        assert_eq!(
            image_units(SourceResolution::P1080, AnalysisQuality::Standard),
            2_040
        );
        assert_eq!(
            image_units(SourceResolution::P2160, AnalysisQuality::High),
            8_160
        );
    }

    #[test]
    fn money_formats_micro_dollars_and_regular_amounts() {
        assert_eq!(Money(101_269_000).format_usd(), "$0.101269");
        assert_eq!(Money(12_345_000_000).format_usd(), "$12.35");
    }
}
