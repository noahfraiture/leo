//! Leo's desktop entry point and internal application composition.

#[cfg(target_os = "windows")]
compile_error!("Leo does not support Windows.");

mod components;
mod desktop;
#[cfg(feature = "desktop-e2e")]
mod e2e;
mod logging;
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

#[cfg(test)]
fn test_analysis_profile(max_images: usize, overlap: usize) -> backend::profiles::AnalysisProfile {
    let mut profile = settings::Settings::default().analysis_profiles.remove(0);
    profile.model = "test-model".into();
    profile.max_images_per_prompt = max_images;
    profile.max_prompt_span_ms = u64::MAX;
    profile.overlap_frame_sets = overlap;
    profile
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
        analysis_profiles: vec![test_analysis_profile(max_images, overlap)],
        openai: openai
            .map(|config| settings::OpenAiSettings {
                api_key: config.api_key,
                base_url: config.base_url,
            })
            .unwrap_or_else(|| settings::Settings::default().openai),
        ..settings::Settings::default()
    }
}
