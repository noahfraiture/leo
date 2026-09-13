//! Reusable local session, recording, and video-analysis backend.
//!
//! The public API separates capture and session storage from exercise configuration,
//! model requests, and analysis. Implementation modules remain private behind these entry points.

pub mod analysis;
pub mod exercise;
pub mod model;
pub mod profiles;
pub mod recording;
pub mod session;

#[cfg(test)]
mod tests;
