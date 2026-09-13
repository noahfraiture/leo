#[allow(clippy::module_inception)]
mod analyze;
mod assessment;
mod citations;
mod correction;
mod evaluation;
mod evidence;
#[cfg(test)]
#[path = "tests/findings.rs"]
mod findings_tests;
mod labels;
mod progress;
#[cfg(all(test, unix))]
#[path = "tests/render.rs"]
mod render_tests;
mod setup;
mod sidebar;

pub use analyze::Analyze;
pub use sidebar::Sidebar;
