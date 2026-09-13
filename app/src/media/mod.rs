//! Registered local media delivery to the desktop webview, with seekable HTTP ranges.
#[cfg(feature = "desktop-e2e")]
mod probe;
mod registry;
mod response;
#[cfg(feature = "desktop-e2e")]
pub use probe::{launch_clip_playback_probe, launch_playback_probe};

pub use registry::{MediaRegistry, use_media_registry};
