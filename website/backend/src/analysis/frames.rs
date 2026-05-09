use std::{env, io::Write, path::Path, time::Duration};

use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::{fs, process::Command};

use crate::{analysis::request::VideoFrame, db};

const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_secs(5);
const FRAME_OUTPUT_PATTERN: &str = "frame-%06d.jpg";

#[derive(Clone, Copy, Debug)]
pub struct FrameExtractionConfig {
    pub interval: Duration,
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
        let interval = env::var("ANALYSIS_FRAME_INTERVAL_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_FRAME_INTERVAL);

        Self { interval }
    }
}

pub async fn extract_video_frames(
    videos: &[db::video::VideoAsset],
    config: FrameExtractionConfig,
) -> Result<Vec<VideoFrame>, FrameExtractionError> {
    let mut frames = Vec::new();

    for video in videos {
        frames.extend(extract_single_video_frames(video, config).await?);
    }

    Ok(frames)
}

async fn extract_single_video_frames(
    video: &db::video::VideoAsset,
    config: FrameExtractionConfig,
) -> Result<Vec<VideoFrame>, FrameExtractionError> {
    let mut input = NamedTempFile::new()?;
    input.write_all(&video.bytes)?;

    let output_dir = tempfile::tempdir()?;
    let output_pattern = output_dir.path().join(FRAME_OUTPUT_PATTERN);
    let interval_secs = config.interval.as_secs_f64();

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
        .arg(format!("fps=1/{interval_secs}"))
        .arg("-q:v")
        .arg("4")
        .arg(&output_pattern)
        .output()
        .await?;

    if !output.status.success() {
        return Err(FrameExtractionError::CommandFailed {
            name: video.video.name.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    let files = frame_files(output_dir.path()).await?;
    if files.is_empty() {
        return Err(FrameExtractionError::Empty {
            name: video.video.name.clone(),
        });
    }

    let mut frames = Vec::with_capacity(files.len());
    for (index, path) in files.iter().enumerate() {
        frames.push(VideoFrame {
            video_name: video.video.name.clone(),
            timestamp_secs: index as f64 * interval_secs,
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
