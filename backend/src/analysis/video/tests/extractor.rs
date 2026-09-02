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
