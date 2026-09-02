//! Discovers finalized camera segments and validates their timing with bounded FFprobe calls.

use std::{
    collections::HashSet,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use super::error::{Error, Result};

const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(50);
const DISCOVERY_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// One finalized local recording segment with exclusive UTC bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSegment {
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
    /// Direct finalized-file path discovered without following links.
    pub path: PathBuf,
}

/// FFprobe timing needed to align and bound one local recording segment.
#[derive(Debug)]
pub struct ProbedMedia {
    /// Container start time rounded down to milliseconds.
    pub start_time_ms: i64,
    /// Positive media span after subtracting the rounded container start.
    pub media_span_ms: i64,
}

#[derive(Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}

#[derive(Deserialize)]
struct ProbeStream {
    index: u32,
}

#[derive(Deserialize)]
struct ProbeFormat {
    start_time: String,
    duration: String,
}

/// Lists finalized MKV segments directly beneath the requested camera directories.
pub fn list_segments(recordings_root: &Path, camera_ids: &[u32]) -> Result<Vec<RecordingSegment>> {
    list_segments_with_ffprobe(
        recordings_root,
        camera_ids,
        &ffmpeg_sidecar::ffprobe::ffprobe_path(),
    )
}

fn list_segments_with_ffprobe(
    recordings_root: &Path,
    camera_ids: &[u32],
    ffprobe: &Path,
) -> Result<Vec<RecordingSegment>> {
    validate_camera_ids(camera_ids)?;
    if !fs::symlink_metadata(recordings_root).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return Err(Error::InvalidRecordingsRoot);
    }

    let shutdown = AtomicBool::new(false);
    let mut segments = Vec::new();
    for &camera_id in camera_ids {
        let camera_directory = recordings_root.join(format!("camera-{camera_id}"));
        if !fs::symlink_metadata(&camera_directory)
            .is_ok_and(|metadata| metadata.file_type().is_dir())
        {
            return Err(Error::InvalidCameraDirectory { camera_id });
        }

        for entry in fs::read_dir(&camera_directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                tracing::warn!(
                    camera_id,
                    path = %path.display(),
                    "invalid direct recording entry"
                );
                return Err(Error::InvalidSegmentEntry { camera_id });
            }
            if !metadata.file_type().is_file() || path.extension() != Some(OsStr::new("mkv")) {
                continue;
            }
            let Some(start_utc_ms) = path
                .file_stem()
                .and_then(OsStr::to_str)
                .and_then(|stem| stem.parse::<i64>().ok())
            else {
                continue;
            };

            let segment =
                probe_media(ffprobe, &path, DISCOVERY_PROBE_TIMEOUT, &shutdown).and_then(|probe| {
                    let end_utc_ms = start_utc_ms
                        .checked_add(probe.media_span_ms)
                        .ok_or(Error::TimestampOverflow)?;
                    Ok(RecordingSegment {
                        camera_id,
                        start_utc_ms,
                        end_utc_ms,
                        path: path.clone(),
                    })
                });
            match segment {
                Ok(segment) => segments.push(segment),
                Err(error) => {
                    tracing::warn!(
                        camera_id,
                        path = %path.display(),
                        "invalid finalized recording segment"
                    );
                    return Err(error);
                }
            }
        }
    }

    segments.sort_by_key(|segment| (segment.camera_id, segment.start_utc_ms));
    for pair in segments.windows(2) {
        if pair[0].camera_id == pair[1].camera_id && pair[1].start_utc_ms < pair[0].end_utc_ms {
            tracing::warn!(
                camera_id = pair[1].camera_id,
                path = %pair[1].path.display(),
                "invalid finalized recording segment"
            );
            return Err(Error::OverlappingSegments {
                camera_id: pair[1].camera_id,
            });
        }
    }

    tracing::info!(
        camera_count = camera_ids.len(),
        segment_count = segments.len(),
        "discovered finalized recording segments"
    );
    Ok(segments)
}

fn validate_camera_ids(camera_ids: &[u32]) -> Result<()> {
    if camera_ids.is_empty() {
        return Err(Error::EmptyCameraList);
    }
    let mut seen = HashSet::with_capacity(camera_ids.len());
    for &camera_id in camera_ids {
        if camera_id == 0 {
            return Err(Error::ZeroCameraId);
        }
        if !seen.insert(camera_id) {
            return Err(Error::DuplicateCamera { camera_id });
        }
    }
    Ok(())
}

/// Probes one media file with bounded process and stdout-reader lifetimes.
pub fn probe_media(
    ffprobe: &Path,
    path: &Path,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<ProbedMedia> {
    if shutdown.load(Ordering::Relaxed) {
        return Err(Error::Shutdown);
    }

    let started = Instant::now();
    let mut child = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v",
            "-show_entries",
            "stream=index:format=start_time,duration",
            "-of",
            "json",
        ])
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        let mut first_error = Some(Error::Io(std::io::Error::other(
            "FFprobe stdout was not piped",
        )));
        kill_and_wait(&mut child, &mut first_error);
        return Err(first_error.expect("missing stdout was recorded"));
    };
    let reader = match thread::Builder::new()
        .name("ffprobe-stdout".into())
        .spawn(move || {
            let mut output = Vec::new();
            let mut stdout = stdout;
            stdout.read_to_end(&mut output)?;
            Ok::<_, std::io::Error>(output)
        }) {
        Ok(reader) => reader,
        Err(error) => {
            let mut first_error = Some(Error::Io(error));
            kill_and_wait(&mut child, &mut first_error);
            return Err(first_error.expect("reader spawn failure was recorded"));
        }
    };

    let mut status = None;
    let mut first_error = None;
    loop {
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                status = Some(exit_status);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                first_error = Some(Error::Io(error));
                break;
            }
        }
        if shutdown.load(Ordering::Relaxed) {
            first_error = Some(Error::Shutdown);
            break;
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            first_error = Some(Error::ProbeTimeout);
            break;
        }
        thread::sleep(PROBE_POLL_INTERVAL.min(timeout.saturating_sub(elapsed)));
    }

    if status.is_some() {
        record_cleanup_result(child.wait(), &mut status, &mut first_error);
    } else {
        kill_and_wait_with_status(&mut child, &mut status, &mut first_error);
    }
    if first_error.is_none() && status.is_some_and(|status| !status.success()) {
        first_error = Some(Error::InvalidMedia);
    }

    let output = match reader.join() {
        Ok(Ok(output)) => Some(output),
        Ok(Err(error)) => {
            record_first_error(&mut first_error, Error::Io(error));
            None
        }
        Err(_) => {
            record_first_error(
                &mut first_error,
                Error::Io(std::io::Error::other("FFprobe stdout reader panicked")),
            );
            None
        }
    };
    if let Some(error) = first_error {
        return Err(error);
    }

    parse_probe_output(&output.expect("successful stdout read was retained"))
}

fn kill_and_wait(child: &mut Child, first_error: &mut Option<Error>) {
    let mut status = None;
    kill_and_wait_with_status(child, &mut status, first_error);
}

fn kill_and_wait_with_status(
    child: &mut Child,
    status: &mut Option<ExitStatus>,
    first_error: &mut Option<Error>,
) {
    if let Err(error) = child.kill() {
        record_first_error(first_error, Error::Io(error));
    }
    record_cleanup_result(child.wait(), status, first_error);
}

fn record_cleanup_result(
    result: std::io::Result<ExitStatus>,
    status: &mut Option<ExitStatus>,
    first_error: &mut Option<Error>,
) {
    match result {
        Ok(exit_status) => *status = Some(exit_status),
        Err(error) => record_first_error(first_error, Error::Io(error)),
    }
}

fn record_first_error(first_error: &mut Option<Error>, error: Error) {
    if first_error.is_none() {
        *first_error = Some(error);
    }
}

fn parse_probe_output(output: &[u8]) -> Result<ProbedMedia> {
    let output: ProbeOutput = serde_json::from_slice(output).map_err(Error::ProbeJson)?;
    let [stream] = output.streams.as_slice() else {
        return Err(Error::InvalidMedia);
    };
    let _ = stream.index;
    let start = output
        .format
        .start_time
        .parse::<f64>()
        .map_err(|_| Error::InvalidMediaDuration)?;
    let duration = output
        .format
        .duration
        .parse::<f64>()
        .map_err(|_| Error::InvalidMediaDuration)?;
    if !start.is_finite() || start < 0.0 || !duration.is_finite() || duration <= 0.0 {
        return Err(Error::InvalidMediaDuration);
    }

    let start_time_ms = checked_millis(start, f64::floor)?;
    let duration_ms = checked_millis(duration, f64::ceil)?;
    let media_span_ms = duration_ms
        .checked_sub(start_time_ms)
        .filter(|span| *span > 0)
        .ok_or(Error::InvalidMediaDuration)?;
    Ok(ProbedMedia {
        start_time_ms,
        media_span_ms,
    })
}

fn checked_millis(seconds: f64, round: fn(f64) -> f64) -> Result<i64> {
    let milliseconds = round(seconds * 1_000.0);
    if !milliseconds.is_finite()
        || milliseconds < i64::MIN as f64
        || milliseconds >= i64::MAX as f64
    {
        return Err(Error::InvalidMediaDuration);
    }
    Ok(milliseconds as i64)
}

#[cfg(all(test, unix))]
#[path = "tests/segment.rs"]
mod tests;
