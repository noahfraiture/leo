//! Candidate recording evidence, reference-aware sampling, and verified image materialization.
mod error;
mod gather;
mod materialize;
mod model;
mod plan;
mod playback;
#[cfg(test)]
mod tests;

pub use error::EvidenceError;
pub use gather::{gather_evidence,gather_evidence_with_budget};
pub use materialize::{materialize_frame, verify_sources};
pub use model::{
    CandidateFrame, EligiblePeriod, EvidenceInterval, EvidenceManifest, EvidenceWindow,
    RecordingSource, SentImage,
};
pub use plan::{build_evidence, plan_windows};
pub use playback::{PlaybackClip, prepare_playback};
