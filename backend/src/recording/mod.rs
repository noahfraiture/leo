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

/// Prevents fake recorder processes from exhausting the host scheduler during parallel tests.
#[cfg(all(test, unix))]
fn process_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static PROCESS_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
    PROCESS_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
