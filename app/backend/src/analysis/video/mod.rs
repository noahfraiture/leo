//! Local recording sampling plans, canonical frame sets, and JPEG extraction.

mod error;
mod extractor;
mod plan;

pub(super) use error::Error;
pub(super) use extractor::extract_jpeg;
pub use plan::AnalysisWarning;
#[cfg(test)]
pub use plan::Frame;
pub(super) use plan::{FrameSet, SampleSequence, SamplingSchedule, recording_gap_warnings};
