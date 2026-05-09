use crate::db;

/// Provider-agnostic analysis input built by the background job.
///
/// Providers decide whether to upload the original video bytes directly or to
/// convert them into sampled frames before calling their model API.
pub struct AnalysisRequest {
    pub videos: Vec<db::video::VideoAsset>,
    pub prompt: String,
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
