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
        model: "test-model".into(),
        base_url: Some("http://provider.example/v1".into()),
    }
}
