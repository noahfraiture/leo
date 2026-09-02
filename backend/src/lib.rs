//! Reusable local session, recording, and video-analysis backend.
//!
//! The public API is organized by responsibility: [`recording`] owns capture processes,
//! [`session`] owns durable session storage, and [`analysis`] owns extraction and model analysis.
//! Implementation modules stay private behind those three entry points.

pub mod analysis;
pub mod recording;
pub mod session;
