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

#[cfg(all(test, unix))]
static PROCESS_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Prevents fake recorder processes from exhausting the host scheduler during parallel tests.
#[cfg(all(test, unix))]
async fn process_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    PROCESS_TEST.lock().await
}

#[cfg(all(test, unix))]
fn blocking_process_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    PROCESS_TEST.blocking_lock()
}
