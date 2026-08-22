use std::{fs, io, path::Path, time::Duration};

use ffmpeg_sidecar::command::FfmpegCommand;
use tempfile::TempDir;

use super::error::{Error, Result};

/// Extracts one video frame at a recording-relative offset as JPEG bytes.
pub(in crate::analysis) fn extract_jpeg(input: &Path, offset: Duration) -> Result<Vec<u8>> {
    let directory = TempDir::new().map_err(|source| Error::CreateFrameTempDir { source })?;
    let output = directory.path().join("frame.jpg");
    let offset_ms = offset.as_millis();

    let status = FfmpegCommand::new()
        .no_overwrite()
        .seek(format!("{offset_ms}ms"))
        .arg("-i")
        .arg(input)
        .map("0:V:0")
        .frames(1)
        .codec_video("mjpeg")
        .format("image2")
        .arg(&output)
        .spawn()
        .map_err(|source| Error::FfmpegSpawn { source })?
        .wait()
        .map_err(|source| Error::FfmpegWait { source })?;

    if !status.success() {
        return Err(Error::FfmpegExit { status });
    }

    read_jpeg(&output)
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
mod tests {
    use std::{fs, path::Path, time::Duration};

    use tempfile::TempDir;

    use super::{extract_jpeg, read_jpeg};
    use crate::analysis::video::error::Error;

    #[test]
    fn reports_missing_frame_output_without_ffmpeg() {
        let directory = TempDir::new().expect("temporary directory should be created");

        let error = read_jpeg(&directory.path().join("frame.jpg"))
            .expect_err("a missing frame should be reported");

        assert!(matches!(error, Error::MissingFrameOutput));
    }

    #[test]
    fn rejects_output_without_jpeg_markers() {
        let directory = TempDir::new().expect("temporary directory should be created");
        let output = directory.path().join("frame.jpg");
        fs::write(&output, [0xff, 0xd8, 0xff, 0x00]).expect("invalid fixture should be written");

        let error = read_jpeg(&output).expect_err("invalid JPEG markers should be rejected");

        assert!(matches!(error, Error::InvalidJpeg));
    }

    #[test]
    #[ignore = "requires FFmpeg on PATH"]
    fn extracts_fixture_frame_as_jpeg() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../camera/fixtures/default.mp4");

        let jpeg = extract_jpeg(&fixture, Duration::from_millis(1_000))
            .expect("fixture frame should be extracted");

        assert!(!jpeg.is_empty());
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(jpeg.ends_with(&[0xff, 0xd9]));
    }
}
