//! Desktop startup, native resource ownership, and the root Dioxus shell.

mod bootstrap;
mod launch;
mod shell;

#[cfg(all(test, unix))]
#[path = "tests/render.rs"]
mod render_tests;

pub use launch::{launch, launch_with_store};
pub use shell::RuntimeAvailability;
