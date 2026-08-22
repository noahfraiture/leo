//! Offline session-video planning, materialization, model analysis, and resumable results.

mod agent;
mod analyzer;
mod error;
mod session;
mod video;

pub use agent::{AnalysisResponse, ChecklistProgress, Observation};
pub use analyzer::{
    ANALYSIS_SCHEMA_VERSION, AnalysisCheckpoint, AnalysisIdentity, AnalysisWarning,
};
pub use error::Error;
pub use session::{AnalyzeSession, analyze_session};

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn extract_jpeg_for_test(
    input: &std::path::Path,
    offset: std::time::Duration,
) -> Result<Vec<u8>, Error> {
    video::extract_jpeg(input, offset).map_err(Error::from)
}
