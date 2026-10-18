use std::{fs, io, path::Path, time::Duration};

use ffmpeg_sidecar::command::FfmpegCommand;
use tempfile::TempDir;

use crate::profiles::ImageSizePolicy;

use super::error::{Error, Result};

const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Extracts one video frame at a recording-relative offset as JPEG bytes.
pub fn extract_jpeg(input: &Path, offset: Duration, size: ImageSizePolicy) -> Result<Vec<u8>> {
    let directory = TempDir::new().map_err(|source| Error::CreateFrameTempDir { source })?;
    let frame = directory.path().join("frame.jpg");
    let offset_ms = offset.as_millis();

    let filter = match size {
        ImageSizePolicy::Original => "trim=end_pts=1,setpts=PTS-STARTPTS".to_owned(),
        ImageSizePolicy::MaximumLongEdge(edge) => format!(
            "trim=end_pts=1,setpts=PTS-STARTPTS,scale=w='min(iw,{edge})':h='min(ih,{edge})':force_original_aspect_ratio=decrease"
        ),
    };
    let mut command = FfmpegCommand::new();
    command
        .hide_banner()
        .args(["-loglevel", "error"])
        .no_overwrite()
        .arg("-noaccurate_seek")
        .seek(format!("{offset_ms}ms"))
        .arg("-i")
        .arg(input)
        .map("0:V:0")
        // Stream through the frame visible at the timestamp, leaving the latest in place.
        .filter(filter)
        .args(["-update", "1"])
        .codec_video("mjpeg")
        .format("image2")
        .arg(&frame);
    let status = crate::analysis::process::run(command.as_inner_mut(), EXTRACTION_TIMEOUT)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::TimedOut {
                Error::FfmpegTimeout {
                    timeout: EXTRACTION_TIMEOUT,
                }
            } else {
                Error::FfmpegRun { source }
            }
        })?;

    if !status.success() {
        tracing::error!(
            path = %input.display(),
            offset_ms = %offset_ms,
            status = %status,
            "FFmpeg frame extraction failed"
        );
        return Err(Error::FfmpegExit { status });
    }

    read_jpeg(&frame)
}

fn read_jpeg(output: &Path) -> Result<Vec<u8>> {
    let jpeg = match fs::read(output) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(Error::MissingFrameOutput);
        }
        result => result.map_err(|source| Error::ReadFrameOutput { source })?,
    };

    if !jpeg.starts_with(&[0xff, 0xd8, 0xff]) || !jpeg.ends_with(&[0xff, 0xd9]) {
        return Err(Error::InvalidJpeg);
    }

    Ok(jpeg)
}

#[cfg(test)]
#[path = "tests/extractor.rs"]
mod tests;
