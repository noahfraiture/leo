use std::{fs, io, path::Path, time::Duration};

use ffmpeg_sidecar::command::FfmpegCommand;
use tempfile::TempDir;

use super::error::{Error, Result};

/// Extracts one video frame at a recording-relative offset as JPEG bytes.
pub(in crate::analysis) fn extract_jpeg(input: &Path, offset: Duration) -> Result<Vec<u8>> {
    let directory = TempDir::new().map_err(|source| Error::CreateFrameTempDir { source })?;
    let frame = directory.path().join("frame.jpg");
    let offset_ms = offset.as_millis();

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
        .filter("trim=end_pts=1,setpts=PTS-STARTPTS")
        .args(["-update", "1"])
        .codec_video("mjpeg")
        .format("image2")
        .arg(&frame);
    let output = command
        .as_inner_mut()
        .output()
        .map_err(|source| Error::FfmpegRun { source })?;

    if !output.status.success() {
        tracing::error!(
            path = %input.display(),
            offset_ms = %offset_ms,
            status = %output.status,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            "FFmpeg frame extraction failed"
        );
        return Err(Error::FfmpegExit {
            status: output.status,
        });
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

        let earlier = extract_jpeg(&fixture, Duration::from_millis(4_800))
            .expect("earlier fixture frame should be extracted");
        let final_start = extract_jpeg(&fixture, Duration::from_millis(4_934))
            .expect("final fixture frame should be extracted");
        let final_end = extract_jpeg(&fixture, Duration::from_millis(4_999))
            .expect("final visible fixture frame should be extracted");

        assert!(
            earlier != final_end,
            "different timestamps need different frames"
        );
        assert!(
            final_start == final_end,
            "timestamps within the final frame need the same image"
        );
        assert!(final_end.starts_with(&[0xff, 0xd8, 0xff]));
        assert!(final_end.ends_with(&[0xff, 0xd9]));
    }
}
