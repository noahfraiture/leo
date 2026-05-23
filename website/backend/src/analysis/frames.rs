use std::{env, io::Write, path::Path};

use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{fs, process::Command};

use crate::analysis::request::{AnalysisVideo, VideoFrame};

const DEFAULT_FRAME_SAMPLE_RATE_FPS: f64 = 0.2;
const FRAME_OUTPUT_PATTERN: &str = "frame-%06d.jpg";

#[derive(Clone, Copy, Debug)]
pub struct FrameExtractionConfig {
    pub sample_rate_fps: f64,
}

#[derive(Debug, Error)]
pub enum FrameExtractionError {
    #[error("ffmpeg did not extract any frames from {name}")]
    Empty { name: String },
    #[error("ffmpeg failed for {name}: {stderr}")]
    CommandFailed { name: String, stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl FrameExtractionConfig {
    pub fn from_env() -> Self {
        let sample_rate_fps = env::var("ANALYSIS_FRAME_SAMPLE_RATE_FPS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(DEFAULT_FRAME_SAMPLE_RATE_FPS);

        Self::from_sample_rate_fps(sample_rate_fps)
    }

    pub fn from_sample_rate_fps(sample_rate_fps: f64) -> Self {
        Self {
            sample_rate_fps: normalize_sample_rate(sample_rate_fps),
        }
    }

    pub fn seconds_per_frame(self) -> f64 {
        1.0 / self.sample_rate_fps
    }

    pub fn ffmpeg_fps_filter(self) -> String {
        format!("fps={}", trim_float(self.sample_rate_fps))
    }
}

pub async fn extract_video_frames(
    videos: &[AnalysisVideo],
    config: FrameExtractionConfig,
) -> Result<Vec<VideoFrame>, FrameExtractionError> {
    let mut frames = Vec::new();

    for video in videos {
        frames.extend(extract_single_video_frames(video, config).await?);
    }

    Ok(frames)
}

async fn extract_single_video_frames(
    video: &AnalysisVideo,
    config: FrameExtractionConfig,
) -> Result<Vec<VideoFrame>, FrameExtractionError> {
    let mut input = NamedTempFile::new()?;
    input.write_all(&video.bytes)?;

    let output_dir = tempfile::tempdir()?;
    let output_pattern = output_dir.path().join(FRAME_OUTPUT_PATTERN);
    let seconds_per_frame = config.seconds_per_frame();

    // ffmpeg stays at the process boundary. It handles the wide video codec
    // surface better than pure Rust crates, while this module keeps the rest of
    // the analysis pipeline working with simple JPEG bytes.
    let output = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input.path())
        .arg("-vf")
        .arg(config.ffmpeg_fps_filter())
        .arg("-q:v")
        .arg("4")
        .arg(&output_pattern)
        .output()
        .await?;

    if !output.status.success() {
        return Err(FrameExtractionError::CommandFailed {
            name: video.name.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let files = frame_files(output_dir.path()).await?;
    if files.is_empty() {
        return Err(FrameExtractionError::Empty {
            name: video.name.clone(),
        });
    }

    let mut frames = Vec::with_capacity(files.len());
    for (index, path) in files.iter().enumerate() {
        frames.push(VideoFrame {
            video_name: video.name.clone(),
            timestamp_secs: index as f64 * seconds_per_frame,
            mime_type: "image/jpeg",
            bytes: fs::read(path).await?,
        });
    }

    Ok(frames)
}

async fn frame_files(path: &Path) -> Result<Vec<std::path::PathBuf>, FrameExtractionError> {
    let mut files = Vec::new();
    let mut entries = fs::read_dir(path).await?;

    while let Some(entry) = entries.next_entry().await? {
        files.push(entry.path());
    }

    files.sort();
    Ok(files)
}

fn normalize_sample_rate(sample_rate_fps: f64) -> f64 {
    if sample_rate_fps.is_finite() && sample_rate_fps > 0.0 {
        sample_rate_fps.clamp(0.1, 8.0)
    } else {
        DEFAULT_FRAME_SAMPLE_RATE_FPS
    }
}

fn trim_float(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::FrameExtractionConfig;

    #[test]
    fn frame_extraction_config_supports_dense_sampling_rates() {
        let config = FrameExtractionConfig::from_sample_rate_fps(2.0);

        assert_eq!(config.sample_rate_fps, 2.0);
        assert_eq!(config.seconds_per_frame(), 0.5);
        assert_eq!(config.ffmpeg_fps_filter(), "fps=2");
    }
}
