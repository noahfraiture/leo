//! Desktop startup, native resource ownership, and the root Dioxus shell.

mod bootstrap;
mod launch;
mod shell;

#[cfg(all(test, unix))]
mod render_tests;
#[cfg(all(test, unix))]
mod tests;

pub use launch::launch;
#[cfg(feature = "desktop-e2e")]
pub use launch::launch_desktop_e2e;
pub use shell::RuntimeAvailability;
