use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
fn list_segments_ignores_partial_and_unrelated_files() {
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
    let recordings = tempfile::tempdir().unwrap();
    camera_directory(recordings.path(), 1);
    let probe_directory = tempfile::tempdir().unwrap();
    let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

    let segments = list_segments_with_ffprobe(recordings.path(), &[1], &ffprobe).unwrap();

    assert!(segments.is_empty());
}

#[test]
fn list_segments_rejects_missing_camera_directory() {
    let _process_test = crate::recording::blocking_process_test_guard();
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
fn list_segments_rejects_duplicate_camera_ids() {
    let _process_test = crate::recording::blocking_process_test_guard();
    let probe_directory = tempfile::tempdir().unwrap();
    let ffprobe = fake_ffprobe(&probe_directory, &valid_probe("0", "1"), 0, &[]);

    let error = list_segments_with_ffprobe(Path::new("missing"), &[3, 3], &ffprobe).unwrap_err();

    assert!(matches!(error, Error::DuplicateCamera { camera_id: 3 }));
}

#[test]
fn list_segments_rejects_empty_and_zero_camera_ids() {
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
    let _process_test = crate::recording::blocking_process_test_guard();
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
