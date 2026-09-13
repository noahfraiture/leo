//! Leo's desktop entry point and internal application composition.

#[cfg(target_os = "windows")]
compile_error!("Leo does not support Windows.");

mod analysis;
mod components;
mod desktop;
#[cfg(feature = "desktop-e2e")]
mod e2e;
mod exercises;
mod inference;
mod logging;
mod media;
#[cfg(all(test, feature = "paid-openai-evaluations"))]
#[path = "evaluations/openai.rs"]
mod openai_evaluations;
mod operator;
mod preview;
mod route;
mod settings;
#[cfg(all(test, unix))]
#[path = "tests/support.rs"]
mod test_support;
mod views;

use desktop::RuntimeAvailability;
pub use desktop::launch;
#[cfg(feature = "desktop-e2e")]
pub use e2e::launch as launch_desktop_e2e;
#[cfg(feature = "desktop-e2e")]
pub use media::{launch_clip_playback_probe, launch_playback_probe};
use route::Route;

#[cfg(test)]
fn test_openai_config() -> backend::analysis::OpenAiConfig {
    backend::analysis::OpenAiConfig {
        api_key: "test-api-key".into(),
        base_url: Some("http://provider.example/v1".into()),
    }
}

#[cfg(test)]
fn test_monitoring_profiles() -> Vec<backend::profiles::MonitoringProfile> {
    (1..=3)
        .map(|id| backend::profiles::MonitoringProfile {
            id,
            name: format!("Profile {id}"),
            sample_every_ms: u64::from(id) * 1000,
        })
        .collect()
}

#[cfg(all(test, feature = "paid-openai-evaluations"))]
fn test_analysis_profile(max_images: usize, overlap: usize) -> backend::profiles::AnalysisProfile {
    backend::profiles::AnalysisProfile {
        id: 1,
        name: "Fixture".into(),
        model: "test-model".into(),
        max_images_per_prompt: max_images,
        max_prompt_span_ms: u64::MAX,
        overlap_frame_sets: overlap,
        image_size: backend::profiles::ImageSizePolicy::Original,
        image_detail: backend::profiles::ImageDetailPolicy::ProviderDefault,
        max_output_tokens: None,
    }
}

#[cfg(test)]
fn test_workflow(
    openai: Option<backend::analysis::OpenAiConfig>,
    max_images: usize,
    overlap: usize,
) -> settings::WorkflowSettings {
    use backend::model::{ModelProfile, ModelSet, ProviderConnection, Transport};
    let Some(openai) = openai else {
        return settings::WorkflowSettings::default();
    };
    settings::WorkflowSettings {
        providers: vec![ProviderConnection {
            id: "fixture".into(),
            name: "Fixture connection".into(),
            api_key: openai.api_key,
            base_url: openai
                .base_url
                .unwrap_or_else(|| "https://api.openai.com/v1".into()),
            transport: Transport::Responses,
        }],
        models: vec![ModelProfile {
            id: "fixture".into(),
            name: "Fixture model".into(),
            provider_id: "fixture".into(),
            model: "test-model".into(),
            memory_cost_mib: 0,
            image_limit: max_images,
            max_output_tokens: None,
            reasoning_effort: None,
        }],
        model_sets: vec![ModelSet {
            id: "fixture".into(),
            name: "Fixture set".into(),
            reader_ids: vec!["fixture".into()],
            reviewer_id: "fixture".into(),
            formatter_id: "fixture".into(),
        }],
        evidence: settings::EvidenceSettings {
            overlap_frame_sets: overlap,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
fn test_settings(
    cameras: Vec<settings::CameraSettings>,
    openai: Option<backend::analysis::OpenAiConfig>,
    max_images: usize,
    overlap: usize,
) -> settings::Settings {
    settings::Settings {
        next_camera_id: cameras.iter().map(|camera| camera.id).max().unwrap_or(0) + 1,
        cameras,
        monitoring_profiles: test_monitoring_profiles(),
        next_monitoring_profile_id: 4,
        workflow: test_workflow(openai, max_images, overlap),
        ..settings::Settings::default()
    }
}
