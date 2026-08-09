//! Catalogue-backed sampling plans, canonical frame sets, and JPEG extraction.

mod error;
mod extractor;
mod video;

pub(super) use error::Error;
pub(super) use extractor::extract_jpeg;
pub(crate) use video::Video;
pub(super) use video::{Frame, FrameSet, SampleSequence, SamplingSchedule};
