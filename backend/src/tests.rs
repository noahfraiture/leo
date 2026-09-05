//! Shared explicit profiles for local session and analyzer fixtures.

use crate::profiles::{AnalysisProfile, ImageDetailPolicy, ImageSizePolicy, MonitoringProfile};

pub fn monitoring_profiles() -> Vec<MonitoringProfile> {
    [
        1, 100, 250, 500, 750, 1000, 1500, 2000, 2500, 3000, 4000, 5000, 10000,
    ]
    .into_iter()
    .map(|id| MonitoringProfile {
        id,
        name: format!("Every {id} ms"),
        sample_every_ms: u64::from(id),
    })
    .collect()
}

pub fn analysis_profile(max_images: usize, overlap: usize) -> AnalysisProfile {
    AnalysisProfile {
        id: 1,
        name: "Fixture".into(),
        model: "test-model".into(),
        max_images_per_prompt: max_images,
        max_prompt_span_ms: u64::MAX,
        overlap_frame_sets: overlap,
        image_size: ImageSizePolicy::Original,
        image_detail: ImageDetailPolicy::ProviderDefault,
        max_output_tokens: None,
    }
}
