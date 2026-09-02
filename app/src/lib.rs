//! Leo's desktop entry point and internal application composition.

#[cfg(target_os = "windows")]
compile_error!("Leo does not support Windows.");

mod components;
mod desktop;
#[cfg(feature = "desktop-e2e")]
mod desktop_e2e;
mod logging;
mod operator;
#[cfg(all(test, feature = "paid-openai-test"))]
mod paid_openai_workflow;
mod preview;
mod route;
mod settings;
#[cfg(all(test, unix))]
mod test_support;
mod views;

use desktop::RuntimeAvailability;
pub use desktop::launch;
#[cfg(feature = "desktop-e2e")]
pub use desktop::launch_desktop_e2e;
use route::Route;

#[cfg(test)]
fn test_openai_config() -> backend::analysis::OpenAiConfig {
    backend::analysis::OpenAiConfig {
        api_key: "test-api-key".into(),
        model: "test-model".into(),
        base_url: Some("http://provider.example/v1".into()),
    }
}
