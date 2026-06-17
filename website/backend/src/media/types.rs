//! Shared in-memory media payload types used by analysis providers.

#[derive(Clone, Debug, PartialEq)]
pub struct AnalysisVideo {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// A sampled video frame ready to send to a vision model.
///
/// Frames keep their source video and timestamp so chunk prompts can preserve
/// temporal context even when multiple videos are analyzed together.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    pub video_name: String,
    pub timestamp_secs: f64,
    pub mime_type: &'static str,
    pub bytes: Vec<u8>,
}
