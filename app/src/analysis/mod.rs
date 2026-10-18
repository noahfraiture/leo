//! Root-owned analysis runs, playback preparation, and operator correction tasks.

mod corrections;
mod execution;
mod playback;
mod review;
mod state;
mod storage;

pub use corrections::save_correction;
pub use execution::{recheck, resume, start};
pub use playback::{load_references, open_evidence};
pub use review::{mark_reviewed, save_assessment};
pub use state::{AnalysisContext, AnalysisPageState, AnalysisTab, use_analysis_context};
pub use storage::load_session;

#[cfg(test)]
mod test_fixture;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub use test_fixture::snapshot as test_snapshot;
