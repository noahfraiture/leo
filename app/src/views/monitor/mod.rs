#[allow(clippy::module_inception)]
mod monitor;
#[cfg(all(test, unix))]
#[path = "tests/render.rs"]
mod render_tests;
mod sidebar;

pub use monitor::Monitor;
pub use sidebar::Sidebar;
