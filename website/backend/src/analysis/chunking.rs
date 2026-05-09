use std::env;

use crate::analysis::request::VideoFrame;

const DEFAULT_MAX_IMAGES_PER_REQUEST: usize = 200;
const DEFAULT_MAX_PAYLOAD_BYTES_PER_REQUEST: usize = 40 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct ChunkingOptions {
    pub max_images_per_request: usize,
    pub max_payload_bytes_per_request: usize,
}

#[derive(Debug, PartialEq)]
pub struct FrameChunk {
    pub start_secs: f64,
    pub end_secs: f64,
    pub frames: Vec<VideoFrame>,
}

impl ChunkingOptions {
    pub fn from_env() -> Self {
        let max_images_per_request = env_usize(
            "ANALYSIS_MAX_IMAGES_PER_REQUEST",
            DEFAULT_MAX_IMAGES_PER_REQUEST,
        );
        let max_payload_bytes_per_request = env_usize(
            "ANALYSIS_MAX_IMAGE_BYTES_PER_REQUEST",
            DEFAULT_MAX_PAYLOAD_BYTES_PER_REQUEST,
        );

        Self {
            max_images_per_request,
            max_payload_bytes_per_request,
        }
    }
}

/// Groups sampled frames by global timeline rather than by source video.
///
/// Provider requests should see the selected videos in temporal order, so this
/// first sorts all frames by timestamp and then opens a new chunk before adding
/// an image would exceed either configured request budget.
pub fn chunk_frames(mut frames: Vec<VideoFrame>, options: ChunkingOptions) -> Vec<FrameChunk> {
    frames.sort_by(|left, right| {
        left.timestamp_secs
            .total_cmp(&right.timestamp_secs)
            .then_with(|| left.video_name.cmp(&right.video_name))
    });

    let max_images = options.max_images_per_request.max(1);
    let max_payload_bytes = options.max_payload_bytes_per_request.max(1);
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    let mut current_payload_bytes = 0usize;

    for frame in frames {
        let frame_bytes = frame.bytes.len();
        let would_exceed_images = current.len() >= max_images;
        let would_exceed_payload =
            !current.is_empty() && current_payload_bytes + frame_bytes > max_payload_bytes;

        if would_exceed_images || would_exceed_payload {
            chunks.push(frame_chunk(current));
            current = Vec::new();
            current_payload_bytes = 0;
        }

        current_payload_bytes += frame_bytes;
        current.push(frame);
    }

    if !current.is_empty() {
        chunks.push(frame_chunk(current));
    }

    chunks
}

fn frame_chunk(frames: Vec<VideoFrame>) -> FrameChunk {
    let start_secs = frames
        .first()
        .map(|frame| frame.timestamp_secs)
        .unwrap_or(0.0);
    let end_secs = frames
        .last()
        .map(|frame| frame.timestamp_secs)
        .unwrap_or(start_secs);

    FrameChunk {
        start_secs,
        end_secs,
        frames,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{ChunkingOptions, chunk_frames};
    use crate::analysis::request::VideoFrame;

    fn frame(video_name: &str, timestamp_secs: f64, bytes: usize) -> VideoFrame {
        VideoFrame {
            video_name: video_name.to_owned(),
            timestamp_secs,
            mime_type: "image/jpeg",
            bytes: vec![b'x'; bytes],
        }
    }

    #[test]
    fn chunk_frames_keeps_all_videos_in_timeline_order() {
        let frames = vec![
            frame("b.mp4", 10.0, 10),
            frame("a.mp4", 0.0, 10),
            frame("b.mp4", 0.0, 10),
            frame("a.mp4", 10.0, 10),
        ];

        let chunks = chunk_frames(
            frames,
            ChunkingOptions {
                max_images_per_request: 3,
                max_payload_bytes_per_request: 100,
            },
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_secs, 0.0);
        assert_eq!(chunks[0].end_secs, 10.0);
        assert_eq!(
            chunks[0]
                .frames
                .iter()
                .map(|frame| (frame.video_name.as_str(), frame.timestamp_secs))
                .collect::<Vec<_>>(),
            vec![("a.mp4", 0.0), ("b.mp4", 0.0), ("a.mp4", 10.0)]
        );
        assert_eq!(
            chunks[1]
                .frames
                .iter()
                .map(|frame| (frame.video_name.as_str(), frame.timestamp_secs))
                .collect::<Vec<_>>(),
            vec![("b.mp4", 10.0)]
        );
    }

    #[test]
    fn chunk_frames_starts_new_chunk_before_payload_limit_is_exceeded() {
        let frames = vec![
            frame("a.mp4", 0.0, 60),
            frame("a.mp4", 5.0, 60),
            frame("a.mp4", 10.0, 20),
        ];

        let chunks = chunk_frames(
            frames,
            ChunkingOptions {
                max_images_per_request: 10,
                max_payload_bytes_per_request: 100,
            },
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].frames.len(), 1);
        assert_eq!(chunks[1].frames.len(), 2);
    }
}
