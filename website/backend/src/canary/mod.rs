//! Synthetic production canary configuration and scheduler.

mod config;
mod runner;

pub use config::{CANARY_VIDEO_NAME, CanaryConfig, DEFAULT_CANARY_PROMPT};
pub use runner::spawn_canary;
