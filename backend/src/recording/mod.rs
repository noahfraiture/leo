//! Host recording supervision plus finalized local segment discovery and validation.

mod error;
mod recorder;
mod segment;

#[cfg(feature = "test-support")]
/// Construction hooks for cross-crate recording tests.
pub mod test_support {
    pub use super::recorder::spawn_for_test as spawn;
}

pub use error::Error;
pub use recorder::{
    RecorderEvent, RecorderHandle, RecorderRuntime, RecorderSettings, RecorderStatus,
    RecordingCamera,
};
pub use segment::{RecordingSegment, list_segments};
