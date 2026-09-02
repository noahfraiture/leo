#[cfg(target_os = "windows")]
compile_error!("Leo does not support Windows.");

mod analysis_task;
mod components;
mod desktop;
#[cfg(feature = "desktop-e2e")]
mod desktop_e2e;
mod logging;
#[cfg(all(test, feature = "paid-openai-test"))]
mod paid_openai_workflow;
mod preview;
mod route;
mod session_task;
mod settings;
#[cfg(all(test, unix))]
mod test_support;
mod views;
mod workflow;

#[cfg(feature = "desktop-e2e")]
pub use desktop::launch_desktop_e2e;
pub use desktop::{RuntimeAvailability, launch};
use route::Route;

#[cfg(test)]
fn test_openai_config() -> backend::analysis::OpenAiConfig {
    backend::analysis::OpenAiConfig {
        api_key: "test-api-key".into(),
        model: "test-model".into(),
        base_url: Some("http://provider.example/v1".into()),
    }
}
