use std::env;

use crate::analysis::request::VideoFrame;

const SAFE_OPENAI_MAX_IMAGES_PER_REQUEST: usize = 450;
const SAFE_OPENAI_MAX_PAYLOAD_BYTES_PER_REQUEST: usize = 45 * 1024 * 1024;
const DEFAULT_OVERLAP_PERCENT: f64 = 10.0;

#[derive(Clone, Copy, Debug)]
pub struct ChunkingOptions {
    pub max_images_per_request: usize,
    pub max_payload_bytes_per_request: usize,
    pub overlap_percent: f64,
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
            SAFE_OPENAI_MAX_IMAGES_PER_REQUEST,
        )
        .min(SAFE_OPENAI_MAX_IMAGES_PER_REQUEST);
        let max_payload_bytes_per_request = env_usize(
            "ANALYSIS_MAX_IMAGE_BYTES_PER_REQUEST",
            SAFE_OPENAI_MAX_PAYLOAD_BYTES_PER_REQUEST,
        )
        .min(SAFE_OPENAI_MAX_PAYLOAD_BYTES_PER_REQUEST);
        let overlap_percent =
            env_f64("ANALYSIS_CHUNK_OVERLAP_PERCENT", DEFAULT_OVERLAP_PERCENT).clamp(0.0, 50.0);

        Self {
            max_images_per_request,
            max_payload_bytes_per_request,
            overlap_percent,
        }
    }
}

/// Groups sampled frames by global timeline rather than by source video.
///
/// Provider requests should see the selected videos in temporal order, so this
/// first sorts all frames by timestamp and then opens a new chunk before adding
/// an image would exceed either configured request budget.
pub fn chunk_frames(frames: Vec<VideoFrame>, options: ChunkingOptions) -> Vec<FrameChunk> {
    chunk_frames_by_payload(frames, options, |frame| frame.bytes.len())
}

pub fn chunk_frames_by_payload(
    mut frames: Vec<VideoFrame>,
    options: ChunkingOptions,
    payload_size: impl Fn(&VideoFrame) -> usize,
) -> Vec<FrameChunk> {
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
        let frame_payload_bytes = payload_size(&frame);
        let would_exceed_images = current.len() >= max_images;
        let would_exceed_payload =
            !current.is_empty() && current_payload_bytes + frame_payload_bytes > max_payload_bytes;

        if would_exceed_images || would_exceed_payload {
            let chunk = frame_chunk(current);
            current = overlap_frames(
                &chunk,
                options,
                &payload_size,
                max_images,
                max_payload_bytes,
            );
            current_payload_bytes = current.iter().map(&payload_size).sum();
            chunks.push(chunk);
        }

        current_payload_bytes += frame_payload_bytes;
        current.push(frame);
    }

    if !current.is_empty() {
        chunks.push(frame_chunk(current));
    }

    chunks
}

fn overlap_frames(
    chunk: &FrameChunk,
    options: ChunkingOptions,
    payload_size: &impl Fn(&VideoFrame) -> usize,
    max_images: usize,
    max_payload_bytes: usize,
) -> Vec<VideoFrame> {
    if options.overlap_percent <= 0.0 || max_images <= 1 || chunk.frames.len() <= 1 {
        return Vec::new();
    }

    let duration_secs = chunk.end_secs - chunk.start_secs;
    if duration_secs <= 0.0 {
        return Vec::new();
    }

    let overlap_secs = duration_secs * (options.overlap_percent / 100.0);
    let overlap_start_secs = chunk.end_secs - overlap_secs;
    let mut frames = chunk
        .frames
        .iter()
        .filter(|frame| frame.timestamp_secs >= overlap_start_secs)
        .cloned()
        .collect::<Vec<_>>();

    while frames.len() >= max_images {
        frames.remove(0);
    }

    while frames.iter().map(payload_size).sum::<usize>() >= max_payload_bytes {
        frames.remove(0);
    }

    frames
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

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
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
                overlap_percent: 0.0,
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
                overlap_percent: 0.0,
            },
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].frames.len(), 1);
        assert_eq!(chunks[1].frames.len(), 2);
    }

    #[test]
    fn chunk_frames_overlaps_next_chunk_by_time_percentage() {
        let frames = vec![
            frame("a.mp4", 0.0, 10),
            frame("a.mp4", 10.0, 10),
            frame("a.mp4", 20.0, 10),
            frame("a.mp4", 30.0, 10),
        ];

        let chunks = chunk_frames(
            frames,
            ChunkingOptions {
                max_images_per_request: 3,
                max_payload_bytes_per_request: 100,
                overlap_percent: 50.0,
            },
        );

        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[1]
                .frames
                .iter()
                .map(|frame| frame.timestamp_secs)
                .collect::<Vec<_>>(),
            vec![10.0, 20.0, 30.0]
        );
    }
}
