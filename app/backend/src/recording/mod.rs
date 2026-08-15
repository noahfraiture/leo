//! Finalized local recording segment discovery and validation.

mod error;
mod recorder;
mod segment;

pub use error::Error;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use recorder::spawn_for_test;
pub use recorder::{
    RecorderEvent, RecorderHandle, RecorderRuntime, RecorderSettings, RecorderStatus,
    RecordingCamera,
};
pub use segment::{RecordingSegment, list_segments};
