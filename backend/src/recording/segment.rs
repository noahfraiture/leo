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
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use serde_json::json;
    use tempfile::TempDir;

    use crate::recording::Error;

    use super::{RecordingSegment, list_segments_with_ffprobe, probe_media};

    fn write_script(directory: &TempDir, body: &str) -> PathBuf {
        let path = directory.path().join("ffprobe");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn fake_ffprobe(
        directory: &TempDir,
        stdout: &str,
        exit_code: i32,
        expected_paths: &[&Path],
    ) -> PathBuf {
        fs::write(directory.path().join("ffprobe.stdout"), stdout).unwrap();
        fs::write(
            directory.path().join("ffprobe.expected"),
            expected_paths
                .iter()
                .map(|path| path.to_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        write_script(
            directory,
            &format!(
                r#"if [ "$#" -ne 9 ] || [ "$1" != "-v" ] || [ "$2" != "error" ] || [ "$3" != "-select_streams" ] || [ "$4" != "v" ] || [ "$5" != "-show_entries" ] || [ "$6" != "stream=index:format=start_time,duration" ] || [ "$7" != "-of" ] || [ "$8" != "json" ]; then
    exit 97
fi
grep -Fqx -e "$9" "$0.expected" || exit 98
cat "$0.stdout"
exit {exit_code}"#
            ),
        )
    }

    fn valid_probe(start_time: &str, duration: &str) -> String {
        json!({
            "streams": [{"index": 0}],
            "format": {"start_time": start_time, "duration": duration}
        })
        .to_string()
    }

    fn input_path(directory: &TempDir) -> PathBuf {
        let path = directory.path().join("segment.mkv");
        fs::write(&path, b"media").unwrap();
        path
    }

    fn process_exists(pid: &str) -> bool {
        Command::new("ps")
            .args(["-p", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    fn wait_for_file(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !path.exists() {
            assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn camera_directory(root: &Path, camera_id: u32) -> PathBuf {
        let path = root.join(format!("camera-{camera_id}"));
        fs::create_dir(&path).unwrap();
        path
    }

    fn write_segment(camera: &Path, name: &str) -> PathBuf {
        let path = camera.join(name);
        fs::write(&path, b"media").unwrap();
        path
    }

    #[test]
    fn probe_rounds_start_down_and_duration_up() {
        let directory = tempfile::tempdir().unwrap();
        let output = json!({
            "streams": [{"index": 4}],
            "format": {"start_time": "0.0679", "duration": "2.0001"},
            "programs": [{"padding": "x".repeat(128 * 1024)}],
            "stream_groups": []
        })
        .to_string();
        let input = input_path(&directory);
        let ffprobe = fake_ffprobe(&directory, &output, 0, &[&input]);

        let media = probe_media(
            &ffprobe,
            &input,
            Duration::from_secs(2),
            &AtomicBool::new(false),
        )
        .unwrap();

        assert_eq!(media.start_time_ms, 67);
        assert_eq!(media.media_span_ms, 1_934);
    }

    #[test]
    fn probe_rejects_non_finite_non_positive_and_multiple_video_streams() {
        let directory = tempfile::tempdir().unwrap();
        let input = input_path(&directory);
        let shutdown = AtomicBool::new(false);

        for output in [
            valid_probe("NaN", "2"),
            valid_probe("-0.001", "2"),
            valid_probe("0", "inf"),
            valid_probe("0", "0"),
            valid_probe("2", "1"),
            valid_probe("0", "9223372036854776"),
        ] {
            let ffprobe = fake_ffprobe(&directory, &output, 0, &[&input]);
            assert!(matches!(
                probe_media(&ffprobe, &input, Duration::from_secs(2), &shutdown),
                Err(Error::InvalidMediaDuration)
            ));
        }

        for output in [
            json!({
                "streams": [],
                "format": {"start_time": "0", "duration": "1"}
            })
            .to_string(),
            json!({
                "streams": [{"index": 0}, {"index": 1}],
                "format": {"start_time": "0", "duration": "1"}
            })
            .to_string(),
        ] {
            let ffprobe = fake_ffprobe(&directory, &output, 0, &[&input]);
            assert!(matches!(
                probe_media(&ffprobe, &input, Duration::from_secs(2), &shutdown),
                Err(Error::InvalidMedia)
            ));
        }
    }

    #[test]
    fn successful_malformed_probe_json_is_fatal() {
        let directory = tempfile::tempdir().unwrap();
        let input = input_path(&directory);
        let ffprobe = fake_ffprobe(&directory, "{", 0, &[&input]);

        let error = probe_media(
            &ffprobe,
            &input,
            Duration::from_secs(2),
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(matches!(error, Error::ProbeJson(_)));
    }

    #[test]
    fn unsuccessful_probe_is_invalid_media() {
        let directory = tempfile::tempdir().unwrap();
        let input = input_path(&directory);
        let ffprobe = fake_ffprobe(&directory, &valid_probe("0", "1"), 23, &[&input]);

        let error = probe_media(
            &ffprobe,
            &input,
            Duration::from_secs(2),
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(matches!(error, Error::InvalidMedia));
    }

    #[test]
    fn hanging_probe_is_killed_reaped_and_times_out() {
        let directory = tempfile::tempdir().unwrap();
        let ffprobe = write_script(
            &directory,
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let started = Instant::now();

        let error = probe_media(
            &ffprobe,
            &input_path(&directory),
            Duration::from_millis(500),
            &AtomicBool::new(false),
        )
        .unwrap_err();

        assert!(matches!(error, Error::ProbeTimeout));
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = fs::read_to_string(directory.path().join("ffprobe.pid")).unwrap();
        assert!(
            !process_exists(&pid),
            "probe process {pid:?} was not reaped"
        );
    }

    #[test]
    fn shutdown_kills_and_reaps_an_active_probe() {
        let directory = tempfile::tempdir().unwrap();
        let ffprobe = write_script(
            &directory,
            r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
        );
        let input = input_path(&directory);
        let pid_path = directory.path().join("ffprobe.pid");
        let shutdown = Arc::new(AtomicBool::new(false));

        let error = thread::scope(|scope| {
            let probe_shutdown = Arc::clone(&shutdown);
            let probe = scope.spawn(move || {
                probe_media(&ffprobe, &input, Duration::from_secs(5), &probe_shutdown)
            });
            wait_for_file(&pid_path);
            let pid = fs::read_to_string(&pid_path).unwrap();
            assert!(process_exists(&pid), "probe was not active before shutdown");
            let shutdown_started = Instant::now();
            shutdown.store(true, Ordering::Relaxed);
            let error = probe.join().unwrap().unwrap_err();
            assert!(shutdown_started.elapsed() < Duration::from_secs(2));
            assert!(
                !process_exists(&pid),
                "probe process {pid:?} was not reaped"
            );
            error
        });

        assert!(matches!(error, Error::Shutdown));
    }

    #[test]
    fn list_segments_ignores_partial_and_unrelated_files() {
        let recordings = tempfile::tempdir().unwrap();
        let camera = camera_directory(recordings.path(), 1);
        let accepted = write_segment(&camera, "1000.mkv");
        write_segment(&camera, "1001.partial.mkv");
        write_segment(&camera, "not-a-timestamp.mkv");
        write_segment(&camera, "1002.mp4");
        write_segment(&camera, "1003.MKV");
        let nested = camera.join("nested");
        fs::create_dir(&nested).unwrap();
        write_segment(&nested, "1004.mkv");
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "2"), 0, &[&accepted]);

        let segments = list_segments_with_ffprobe(recordings.path(), &[1], &ffprobe).unwrap();

        assert_eq!(
            segments,
            vec![RecordingSegment {
                camera_id: 1,
                start_utc_ms: 1_000,
                end_utc_ms: 3_000,
                path: accepted,
            }]
        );
    }

    #[test]
    fn list_segments_accepts_an_existing_empty_camera_directory() {
        let recordings = tempfile::tempdir().unwrap();
        camera_directory(recordings.path(), 1);
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

        let segments = list_segments_with_ffprobe(recordings.path(), &[1], &ffprobe).unwrap();

        assert!(segments.is_empty());
    }

    #[test]
    fn list_segments_rejects_missing_camera_directory() {
        let recordings = tempfile::tempdir().unwrap();
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

        let error = list_segments_with_ffprobe(recordings.path(), &[7], &ffprobe).unwrap_err();

        assert!(matches!(
            error,
            Error::InvalidCameraDirectory { camera_id: 7 }
        ));
    }

    #[test]
    fn list_segments_rejects_symlinked_roots_directories_and_entries() {
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

        let root_case = tempfile::tempdir().unwrap();
        let root_target = root_case.path().join("target");
        fs::create_dir(&root_target).unwrap();
        camera_directory(&root_target, 1);
        let root_link = root_case.path().join("recordings");
        symlink(&root_target, &root_link).unwrap();
        assert!(matches!(
            list_segments_with_ffprobe(&root_link, &[1], &ffprobe),
            Err(Error::InvalidRecordingsRoot)
        ));

        let directory_case = tempfile::tempdir().unwrap();
        let camera_target = directory_case.path().join("actual-camera");
        fs::create_dir(&camera_target).unwrap();
        symlink(&camera_target, directory_case.path().join("camera-1")).unwrap();
        assert!(matches!(
            list_segments_with_ffprobe(directory_case.path(), &[1], &ffprobe),
            Err(Error::InvalidCameraDirectory { camera_id: 1 })
        ));

        let entry_case = tempfile::tempdir().unwrap();
        let camera = camera_directory(entry_case.path(), 1);
        let target = entry_case.path().join("target.mkv");
        fs::write(&target, b"media").unwrap();
        symlink(&target, camera.join("1000.mkv")).unwrap();
        assert!(matches!(
            list_segments_with_ffprobe(entry_case.path(), &[1], &ffprobe),
            Err(Error::InvalidSegmentEntry { camera_id: 1 })
        ));
    }

    #[test]
    fn list_segments_rejects_duplicate_camera_ids() {
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

        let error =
            list_segments_with_ffprobe(Path::new("missing"), &[3, 3], &ffprobe).unwrap_err();

        assert!(matches!(error, Error::DuplicateCamera { camera_id: 3 }));
    }

    #[test]
    fn list_segments_rejects_empty_and_zero_camera_ids() {
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

        assert!(matches!(
            list_segments_with_ffprobe(Path::new("missing"), &[], &ffprobe),
            Err(Error::EmptyCameraList)
        ));
        assert!(matches!(
            list_segments_with_ffprobe(Path::new("missing"), &[0], &ffprobe),
            Err(Error::ZeroCameraId)
        ));
    }

    #[test]
    fn list_segments_rejects_overlapping_intervals() {
        let overlapping = tempfile::tempdir().unwrap();
        let camera = camera_directory(overlapping.path(), 1);
        let overlapping_first = write_segment(&camera, "1000.mkv");
        let overlapping_second = write_segment(&camera, "1500.mkv");

        let adjacent = tempfile::tempdir().unwrap();
        let camera = camera_directory(adjacent.path(), 1);
        let adjacent_first = write_segment(&camera, "1000.mkv");
        let adjacent_second = write_segment(&camera, "2000.mkv");
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(
            &probe_directory,
            &valid_probe("0", "1"),
            0,
            &[
                &overlapping_first,
                &overlapping_second,
                &adjacent_first,
                &adjacent_second,
            ],
        );

        let error = list_segments_with_ffprobe(overlapping.path(), &[1], &ffprobe).unwrap_err();

        assert!(matches!(error, Error::OverlappingSegments { camera_id: 1 }));

        assert_eq!(
            list_segments_with_ffprobe(adjacent.path(), &[1], &ffprobe)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn list_segments_rejects_duplicate_parsed_starts() {
        let recordings = tempfile::tempdir().unwrap();
        let camera = camera_directory(recordings.path(), 1);
        let first = write_segment(&camera, "1000.mkv");
        let duplicate = write_segment(&camera, "01000.mkv");
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(
            &probe_directory,
            &valid_probe("0", "1"),
            0,
            &[&first, &duplicate],
        );

        let error = list_segments_with_ffprobe(recordings.path(), &[1], &ffprobe).unwrap_err();

        assert!(matches!(error, Error::OverlappingSegments { camera_id: 1 }));
    }

    #[test]
    fn list_segments_sorts_by_camera_and_start() {
        let recordings = tempfile::tempdir().unwrap();
        let camera_2 = camera_directory(recordings.path(), 2);
        let camera_1 = camera_directory(recordings.path(), 1);
        let camera_1_later = write_segment(&camera_1, "3000.mkv");
        let camera_1_earlier = write_segment(&camera_1, "1000.mkv");
        let camera_2_segment = write_segment(&camera_2, "2000.mkv");
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(
            &probe_directory,
            &valid_probe("0", "0.5"),
            0,
            &[&camera_1_later, &camera_1_earlier, &camera_2_segment],
        );

        let segments = list_segments_with_ffprobe(recordings.path(), &[2, 1], &ffprobe).unwrap();

        assert_eq!(
            segments,
            vec![
                RecordingSegment {
                    camera_id: 1,
                    start_utc_ms: 1_000,
                    end_utc_ms: 1_500,
                    path: camera_1_earlier,
                },
                RecordingSegment {
                    camera_id: 1,
                    start_utc_ms: 3_000,
                    end_utc_ms: 3_500,
                    path: camera_1_later,
                },
                RecordingSegment {
                    camera_id: 2,
                    start_utc_ms: 2_000,
                    end_utc_ms: 2_500,
                    path: camera_2_segment,
                },
            ]
        );
    }

    #[test]
    fn list_segments_rejects_timestamp_overflow() {
        let recordings = tempfile::tempdir().unwrap();
        let camera = camera_directory(recordings.path(), 1);
        let segment = write_segment(&camera, &format!("{}.mkv", i64::MAX));
        let probe_directory = tempfile::tempdir().unwrap();
        let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "0.001"), 0, &[&segment]);

        let error = list_segments_with_ffprobe(recordings.path(), &[1], &ffprobe).unwrap_err();

        assert!(matches!(error, Error::TimestampOverflow));
    }
}
