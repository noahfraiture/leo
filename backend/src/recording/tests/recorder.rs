use std::{
    cell::Cell,
    fs,
    future::Future,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use tempfile::TempDir;

use crate::recording::{Error, RecordingSegment};

use super::{
    AttemptConfig, AttemptEvent, AttemptResult, CameraSupervisor, RecorderEvent, RecorderSet,
    RecorderSettings, RecorderStatus, RecordingCamera, StartupState, cleanup_failed_start,
    finalize_attempt, parse_progress_time, run_attempt_with_clock, spawn_with_executables,
};

const PROGRESS_ONE_SECOND: &str = "[info] frame=    1 fps=1.0 q=-1.0 size=       1kB time=00:00:01.000 bitrate=   8.0kbits/s speed=1x";

fn write_script(directory: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn attempt_config<'a>(
    ffmpeg: &'a Path,
    rtsp_url: &'a str,
    partial_path: &'a Path,
) -> AttemptConfig<'a> {
    AttemptConfig {
        ffmpeg,
        rtsp_url,
        partial_path,
        io_timeout: Duration::from_millis(750),
        stop_timeout: Duration::from_secs(1),
    }
}

fn run<F>(
    config: AttemptConfig<'_>,
    stop: &AtomicBool,
    now_utc_ms: F,
) -> (Result<AttemptResult, Error>, Vec<AttemptEvent>)
where
    F: FnMut() -> Result<i64, Error>,
{
    let shutdown = AtomicBool::new(false);
    run_with_tokens(config, stop, &shutdown, now_utc_ms)
}

fn run_with_tokens<F>(
    config: AttemptConfig<'_>,
    stop: &AtomicBool,
    shutdown: &AtomicBool,
    now_utc_ms: F,
) -> (Result<AttemptResult, Error>, Vec<AttemptEvent>)
where
    F: FnMut() -> Result<i64, Error>,
{
    let (events_tx, events_rx) = mpsc::channel();
    let result = run_attempt_with_clock(config, stop, shutdown, &events_tx, now_utc_ms);
    drop(events_tx);
    (result, events_rx.try_iter().collect())
}

fn args_path(ffmpeg: &Path) -> PathBuf {
    ffmpeg.with_extension("args")
}

fn marker_path(ffmpeg: &Path, extension: &str) -> PathBuf {
    ffmpeg.with_extension(extension)
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {path:?}");
        thread::sleep(Duration::from_millis(10));
    }
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

fn fake_ffprobe(directory: &TempDir, name: &str, output: &str, exit_code: i32) -> PathBuf {
    let ffprobe = directory.path().join(name);
    fs::write(ffprobe.with_extension("stdout"), output).unwrap();
    let body = format!(
        r#"if [ "$#" -ne 9 ] || [ "$1" != "-v" ] || [ "$2" != "error" ] || [ "$3" != "-select_streams" ] || [ "$4" != "v" ] || [ "$5" != "-show_entries" ] || [ "$6" != "stream=index:format=start_time,duration" ] || [ "$7" != "-of" ] || [ "$8" != "json" ]; then
    exit 97
fi
cat "$0.stdout"
exit {exit_code}"#
    );
    write_script(directory, name, &body);
    ffprobe
}

fn valid_probe(start_time: &str, duration: &str) -> String {
    json!({
        "streams": [{"index": 0}],
        "format": {"start_time": start_time, "duration": duration}
    })
    .to_string()
}

fn recorder_settings() -> RecorderSettings {
    RecorderSettings {
        io_timeout: Duration::from_secs(2),
        retry_delay: Duration::from_millis(100),
        stop_timeout: Duration::from_secs(1),
    }
}

fn preflight_executable(directory: &TempDir, name: &str, exit_code: i32) -> PathBuf {
    write_script(
        directory,
        name,
        &format!(
            r#"if [ "$#" -ne 1 ] || [ "$1" != "-version" ]; then
    printf invoked > "$0.recording-invoked"
    exit 97
fi
exit {exit_code}"#
        ),
    )
}

fn valid_recordings_root(directory: &TempDir, camera_ids: &[u32]) -> PathBuf {
    let root = directory.path().join("recordings");
    fs::create_dir(&root).unwrap();
    for camera_id in camera_ids {
        fs::create_dir(root.join(format!("camera-{camera_id}"))).unwrap();
    }
    root
}

fn startup_ffmpeg(directory: &TempDir, name: &str) -> PathBuf {
    write_script(
        directory,
        name,
        &format!(
            r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
    fi
    previous="$argument"
    output_path="$argument"
done
scenario="${{source_url##*/}}"
printf '%s\n' "$$" > "$0.$scenario.pid"
if [ "$scenario" = "slow" ]; then
    while [ ! -f "$0.release" ]; do sleep 0.01; done
fi
if [ "$scenario" = "fail" ]; then
    while [ ! -f "$0.fail" ]; do sleep 0.01; done
    exit 23
fi
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
if [ "$scenario" = "ready-fail" ]; then
    while [ ! -f "$0.ready-fail-exit" ]; do sleep 0.01; done
    : > "$0.ready-fail-exited"
    exit 23
fi
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.$scenario.quit""#
        ),
    )
}

fn scenario_marker(ffmpeg: &Path, scenario: &str, suffix: &str) -> PathBuf {
    ffmpeg.with_extension(format!("{scenario}.{suffix}"))
}

fn wait_for_status(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
    camera_id: u32,
    status: RecorderStatus,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match events.try_recv() {
            Ok(RecorderEvent::Status {
                camera_id: actual_camera,
                status: actual_status,
                ..
            }) if actual_camera == camera_id && actual_status == status => return,
            Ok(RecorderEvent::Faulted { message, .. }) => {
                panic!("unexpected recorder fault: {message}")
            }
            Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("recorder event channel disconnected")
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {status:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn supervision_ffmpeg(directory: &TempDir, name: &str) -> PathBuf {
    write_script(
        directory,
        name,
        &format!(
            r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
    fi
    previous="$argument"
    output_path="$argument"
done
scenario="${{source_url##*/}}"
count_file="$0.$scenario.count"
count=1
if [ -f "$count_file" ]; then count=$(( $(cat "$count_file") + 1 )); fi
printf '%s' "$count" > "$count_file"
printf '%s\n' "$output_path" >> "$0.paths"
printf '%s\n' "$$" > "$0.$scenario.$count.pid"
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
if [ "$scenario" = "reconnect" ] && [ "$count" -eq 1 ]; then
    while [ ! -f "$0.reconnect-exit" ]; do sleep 0.01; done
    exit 0
fi
if [ "$scenario" = "storage" ]; then
    while [ ! -f "$0.exit" ]; do sleep 0.01; done
    exit 0
fi
case "$scenario" in
    fatal-*) while [ ! -f "$0.fatal" ]; do sleep 0.01; done; exit 0 ;;
esac
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.$scenario.$count.quit""#
        ),
    )
}

fn runtime_ffprobe(directory: &TempDir, name: &str, probe_body: &str) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(path.with_extension("stdout"), valid_probe("0", "1")).unwrap();
    write_script(
        directory,
        name,
        &format!(
            r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
if [ "$#" -ne 9 ] || [ "$1" != "-v" ] || [ "$2" != "error" ] || [ "$3" != "-select_streams" ] || [ "$4" != "v" ] || [ "$5" != "-show_entries" ] || [ "$6" != "stream=index:format=start_time,duration" ] || [ "$7" != "-of" ] || [ "$8" != "json" ]; then
    exit 97
fi
{probe_body}"#
        ),
    )
}

fn valid_runtime_ffprobe(directory: &TempDir, name: &str) -> PathBuf {
    runtime_ffprobe(directory, name, r#"cat "$0.stdout""#)
}

fn wait_for_fault(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
) -> (Option<u32>, String) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match events.try_recv() {
            Ok(RecorderEvent::Faulted { camera_id, message }) => return (camera_id, message),
            Ok(_) | Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                panic!("recorder event channel disconnected")
            }
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for recorder fault"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn spawn_rejects_missing_or_failing_ffmpeg_and_ffprobe() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let success = preflight_executable(&directory, "preflight-success", 0);
    let failure = preflight_executable(&directory, "preflight-failure", 23);
    let missing = directory.path().join("missing");

    for (ffmpeg, ffprobe) in [
        (&missing, &success),
        (&failure, &success),
        (&success, &missing),
        (&success, &failure),
    ] {
        assert!(
            spawn_with_executables(
                recorder_settings(),
                ffmpeg.to_path_buf(),
                ffprobe.to_path_buf(),
            )
            .is_err()
        );
    }
}

#[test]
fn hanging_preflight_is_killed_reaped_and_times_out() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let hanging = write_script(
        &directory,
        "preflight-hanging",
        r#"if [ "$#" -ne 1 ] || [ "$1" != "-version" ]; then
    exit 97
fi
printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
    );
    let success = preflight_executable(&directory, "preflight-success", 0);
    let mut settings = recorder_settings();
    settings.io_timeout = Duration::from_secs(1);
    let started = Instant::now();

    assert!(spawn_with_executables(settings, hanging.clone(), success).is_err());

    assert!(started.elapsed() < Duration::from_secs(2));
    let pid = fs::read_to_string(marker_path(&hanging, "pid")).unwrap();
    assert!(!process_exists(&pid), "preflight process {pid:?} leaked");
}

#[test]
fn spawn_rejects_zero_or_unrepresentable_settings() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let executable = preflight_executable(&directory, "preflight-settings", 0);
    let valid = recorder_settings();
    let invalid = [
        RecorderSettings {
            io_timeout: Duration::ZERO,
            ..valid
        },
        RecorderSettings {
            retry_delay: Duration::ZERO,
            ..valid
        },
        RecorderSettings {
            stop_timeout: Duration::ZERO,
            ..valid
        },
        RecorderSettings {
            io_timeout: Duration::MAX,
            ..valid
        },
        RecorderSettings {
            retry_delay: Duration::MAX,
            ..valid
        },
        RecorderSettings {
            stop_timeout: Duration::MAX,
            ..valid
        },
    ];

    for settings in invalid {
        assert!(spawn_with_executables(settings, executable.clone(), executable.clone()).is_err());
    }
    assert!(!marker_path(&executable, "recording-invoked").exists());
}

#[tokio::test]
async fn start_rejects_empty_duplicate_zero_and_non_rtsp_cameras() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let executable = preflight_executable(&directory, "preflight-cameras", 0);
    let (runtime, handle, _events) =
        spawn_with_executables(recorder_settings(), executable.clone(), executable.clone())
            .unwrap();
    let root = valid_recordings_root(&directory, &[0, 1]);

    assert!(handle.start(Vec::new(), root.clone()).await.is_err());
    assert!(
        handle
            .start(
                vec![RecordingCamera {
                    id: 0,
                    rtsp_url: "rtsp://camera.invalid/stream".into(),
                }],
                root.clone(),
            )
            .await
            .is_err()
    );
    assert!(
        handle
            .start(
                vec![
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/one".into(),
                    },
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/two".into(),
                    },
                ],
                root.clone(),
            )
            .await
            .is_err()
    );
    let secret_url = "https://student:secret@camera.invalid/private";
    let error = handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: secret_url.into(),
            }],
            root,
        )
        .await
        .unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(secret_url));
    assert!(!rendered.contains("student:secret"));
    assert!(!marker_path(&executable, "recording-invoked").exists());
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn start_rejects_missing_symlinked_and_non_directory_output_paths() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let executable = preflight_executable(&directory, "preflight-paths", 0);
    let (runtime, handle, _events) =
        spawn_with_executables(recorder_settings(), executable.clone(), executable.clone())
            .unwrap();
    let camera = || {
        vec![RecordingCamera {
            id: 1,
            rtsp_url: "rtsp://camera.invalid/stream".into(),
        }]
    };

    assert!(
        handle
            .start(camera(), directory.path().join("missing"))
            .await
            .is_err()
    );

    let root_file = directory.path().join("root-file");
    fs::write(&root_file, b"not a directory").unwrap();
    assert!(handle.start(camera(), root_file).await.is_err());

    let root_target = directory.path().join("root-target");
    fs::create_dir(&root_target).unwrap();
    fs::create_dir(root_target.join("camera-1")).unwrap();
    let root_link = directory.path().join("root-link");
    symlink(&root_target, &root_link).unwrap();
    assert!(handle.start(camera(), root_link).await.is_err());

    let missing_camera_root = directory.path().join("missing-camera-root");
    fs::create_dir(&missing_camera_root).unwrap();
    assert!(handle.start(camera(), missing_camera_root).await.is_err());

    let camera_file_root = directory.path().join("camera-file-root");
    fs::create_dir(&camera_file_root).unwrap();
    fs::write(camera_file_root.join("camera-1"), b"not a directory").unwrap();
    assert!(handle.start(camera(), camera_file_root).await.is_err());

    let camera_link_root = directory.path().join("camera-link-root");
    fs::create_dir(&camera_link_root).unwrap();
    let camera_target = directory.path().join("camera-target");
    fs::create_dir(&camera_target).unwrap();
    symlink(&camera_target, camera_link_root.join("camera-1")).unwrap();
    assert!(handle.start(camera(), camera_link_root).await.is_err());

    assert!(!marker_path(&executable, "recording-invoked").exists());
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn start_waits_for_every_camera() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = startup_ffmpeg(&directory, "startup-all");
    let ffprobe = preflight_executable(&directory, "startup-all-probe", 0);
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1, 2]);
    let start_handle = handle.clone();
    let start = tokio::spawn(async move {
        start_handle
            .start(
                vec![
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/ready".into(),
                    },
                    RecordingCamera {
                        id: 2,
                        rtsp_url: "rtsp://camera.invalid/slow".into(),
                    },
                ],
                root,
            )
            .await
    });
    tokio::task::yield_now().await;

    wait_for_status(&mut events, 1, RecorderStatus::Recording);
    wait_for_file(&scenario_marker(&ffmpeg, "slow", "pid"));
    assert!(
        !start.is_finished(),
        "Start completed before every camera was ready"
    );
    fs::write(marker_path(&ffmpeg, "release"), []).unwrap();

    start.await.unwrap().unwrap();
    handle.stop().await.unwrap();
    for scenario in ["ready", "slow"] {
        let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
        assert!(!process_exists(&pid), "startup process {pid:?} leaked");
    }
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn one_startup_failure_stops_and_reaps_ready_cameras() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = startup_ffmpeg(&directory, "startup-rollback");
    let ffprobe = preflight_executable(&directory, "startup-rollback-probe", 0);
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1, 2]);
    let start_handle = handle.clone();
    let start = tokio::spawn(async move {
        start_handle
            .start(
                vec![
                    RecordingCamera {
                        id: 1,
                        rtsp_url: "rtsp://camera.invalid/ready".into(),
                    },
                    RecordingCamera {
                        id: 2,
                        rtsp_url: "rtsp://camera.invalid/fail".into(),
                    },
                ],
                root,
            )
            .await
    });
    tokio::task::yield_now().await;

    wait_for_status(&mut events, 1, RecorderStatus::Recording);
    wait_for_file(&scenario_marker(&ffmpeg, "fail", "pid"));
    fs::write(marker_path(&ffmpeg, "fail"), []).unwrap();

    assert!(matches!(
        start.await.unwrap(),
        Err(Error::RecorderStartupFailed)
    ));
    for scenario in ["ready", "fail"] {
        let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "pid")).unwrap();
        assert!(!process_exists(&pid), "startup process {pid:?} leaked");
    }
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, RecorderEvent::Faulted { .. }));
    }
    runtime.shutdown().unwrap();
}

#[test]
fn startup_cleanup_failure_is_reported_distinctly() {
    let _process_test = crate::recording::process_test_guard();
    let shutdown = Arc::new(AtomicBool::new(false));
    let (events, _receiver) = tokio::sync::mpsc::unbounded_channel();
    let recorders = RecorderSet {
        stop: Arc::new(AtomicBool::new(false)),
        shutdown,
        startup: Arc::new(Mutex::new(StartupState::Pending)),
        fault_emitted: Arc::new(AtomicBool::new(false)),
        events,
        supervisors: vec![
            CameraSupervisor {
                thread: thread::spawn(|| {
                    Err(Error::RecorderCleanupFailed {
                        source: Box::new(Error::FfmpegQuit),
                    })
                }),
            },
            CameraSupervisor {
                thread: thread::spawn(|| Err(Error::RecorderStartupFailed)),
            },
        ],
    };

    let error = cleanup_failed_start(recorders, Error::RecorderStartupFailed);

    assert!(matches!(error, Error::RecorderStartupCleanupFailed));
}

#[tokio::test]
async fn failure_after_successful_start_reply_is_faulted() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = startup_ffmpeg(&directory, "startup-published-failure");
    let ffprobe = runtime_ffprobe(&directory, "startup-published-failure-probe", "printf '{'");
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);

    handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/ready-fail".into(),
            }],
            root,
        )
        .await
        .unwrap();
    fs::write(marker_path(&ffmpeg, "ready-fail-exit"), []).unwrap();

    let (camera_id, _) = wait_for_fault(&mut events);
    assert_eq!(camera_id, Some(1));
    assert!(handle.stop().await.is_err());
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, RecorderEvent::Faulted { .. }));
    }
    let pid = fs::read_to_string(scenario_marker(&ffmpeg, "ready-fail", "pid")).unwrap();
    assert!(!process_exists(&pid), "post-start process {pid:?} leaked");
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn duplicate_start_and_stop_commands_are_rejected() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = startup_ffmpeg(&directory, "startup-duplicates");
    let ffprobe = preflight_executable(&directory, "startup-duplicates-probe", 0);
    let (runtime, handle, _events) =
        spawn_with_executables(recorder_settings(), ffmpeg, ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    let cameras = vec![RecordingCamera {
        id: 1,
        rtsp_url: "rtsp://camera.invalid/ready".into(),
    }];

    handle.start(cameras.clone(), root.clone()).await.unwrap();
    assert!(handle.start(cameras, root).await.is_err());
    handle.stop().await.unwrap();
    assert!(handle.stop().await.is_err());
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn ordinary_exit_finalizes_and_emits_reconnecting() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "supervision-ordinary");
    let ffprobe = valid_runtime_ffprobe(&directory, "supervision-ordinary-probe");
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);

    handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/reconnect".into(),
            }],
            root,
        )
        .await
        .unwrap();
    fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
    wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);

    let segments = handle.stop().await.unwrap();
    assert_eq!(segments.len(), 1);
    assert!(segments[0].path.exists());
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn retry_uses_a_new_partial_path_and_returns_to_recording() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "supervision-retry");
    let ffprobe = valid_runtime_ffprobe(&directory, "supervision-retry-probe");
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);

    handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/reconnect".into(),
            }],
            root,
        )
        .await
        .unwrap();
    fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
    wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);
    wait_for_status(&mut events, 1, RecorderStatus::Recording);

    let paths = fs::read_to_string(marker_path(&ffmpeg, "paths")).unwrap();
    let paths = paths.lines().collect::<Vec<_>>();
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1]);
    assert!(
        paths
            .iter()
            .all(|path| path.contains(".attempt-") && path.ends_with(".partial.mkv"))
    );

    let segments = handle.stop().await.unwrap();
    assert_eq!(segments.len(), 2);
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn storage_probe_failure_emits_faulted() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "supervision-storage");
    let ffprobe = valid_runtime_ffprobe(&directory, "supervision-storage-probe");
    let (runtime, handle, mut events) =
        spawn_with_executables(recorder_settings(), ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    let camera_directory = root.join("camera-1");

    handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://student:secret@camera.invalid/storage".into(),
            }],
            root,
        )
        .await
        .unwrap();
    wait_for_status(&mut events, 1, RecorderStatus::Recording);
    let partial = fs::read_to_string(marker_path(&ffmpeg, "paths")).unwrap();
    fs::remove_file(partial.trim()).unwrap();
    fs::remove_dir(camera_directory).unwrap();
    fs::write(marker_path(&ffmpeg, "exit"), []).unwrap();

    let (camera_id, message) = wait_for_fault(&mut events);
    assert_eq!(camera_id, Some(1));
    assert!(!message.contains("student:secret"));
    assert!(!message.contains("rtsp://"));
    thread::sleep(Duration::from_millis(100));
    while let Ok(event) = events.try_recv() {
        assert!(!matches!(event, RecorderEvent::Faulted { .. }));
    }
    assert!(handle.stop().await.is_err());
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn stop_finalizes_and_reaps_every_camera_concurrently() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "supervision-concurrent");
    let ffprobe = runtime_ffprobe(
        &directory,
        "supervision-concurrent-probe",
        r#"case "$9" in
    */camera-1/*) marker="$0.camera-1" ;;
    */camera-2/*) marker="$0.camera-2" ;;
    *) exit 98 ;;
esac
: > "$marker"
while [ ! -f "$0.camera-1" ] || [ ! -f "$0.camera-2" ]; do sleep 0.01; done
cat "$0.stdout""#,
    );
    let mut settings = recorder_settings();
    settings.io_timeout = Duration::from_secs(2);
    let (runtime, handle, _events) =
        spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1, 2]);

    handle
        .start(
            vec![
                RecordingCamera {
                    id: 1,
                    rtsp_url: "rtsp://camera.invalid/camera-1".into(),
                },
                RecordingCamera {
                    id: 2,
                    rtsp_url: "rtsp://camera.invalid/camera-2".into(),
                },
            ],
            root,
        )
        .await
        .unwrap();
    let started = Instant::now();

    let segments = handle.stop().await.unwrap();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        directory
            .path()
            .join("supervision-concurrent-probe.camera-1")
            .exists(),
        "camera 1 FFprobe was not invoked"
    );
    assert!(
        directory
            .path()
            .join("supervision-concurrent-probe.camera-2")
            .exists(),
        "camera 2 FFprobe was not invoked"
    );
    assert_eq!(
        segments
            .iter()
            .map(|segment| segment.camera_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    for scenario in ["camera-1", "camera-2"] {
        let pid = fs::read_to_string(scenario_marker(&ffmpeg, scenario, "1.pid")).unwrap();
        assert!(!process_exists(&pid), "recorder process {pid:?} leaked");
    }
    runtime.shutdown().unwrap();
}

#[tokio::test]
async fn hanging_stop_probe_times_out_without_leaking_a_process() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "supervision-hanging-probe");
    let ffprobe = runtime_ffprobe(
        &directory,
        "supervision-hanging-probe-ffprobe",
        r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
    );
    let mut settings = recorder_settings();
    settings.io_timeout = Duration::from_secs(1);
    let (runtime, handle, mut events) =
        spawn_with_executables(settings, ffmpeg.clone(), ffprobe.clone()).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    handle
        .start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/hold".into(),
            }],
            root,
        )
        .await
        .unwrap();
    let started = Instant::now();

    assert!(handle.stop().await.is_err());

    assert!(started.elapsed() < Duration::from_secs(2));
    let pid = fs::read_to_string(marker_path(&ffprobe, "pid")).unwrap();
    assert!(!process_exists(&pid), "FFprobe process {pid:?} leaked");
    let (camera_id, _) = wait_for_fault(&mut events);
    assert_eq!(camera_id, Some(1));
    let ffmpeg_pid = fs::read_to_string(scenario_marker(&ffmpeg, "hold", "1.pid")).unwrap();
    assert!(
        !process_exists(&ffmpeg_pid),
        "FFmpeg process {ffmpeg_pid:?} leaked"
    );
    runtime.shutdown().unwrap();
}

#[test]
fn shutdown_interrupts_initial_readiness_and_reaps_children() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "shutdown-startup",
        r#"if [ "$#" -eq 1 ] && [ "$1" = "-version" ]; then
    exit 0
fi
printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
    );
    let ffprobe = valid_runtime_ffprobe(&directory, "shutdown-startup-probe");
    let mut settings = recorder_settings();
    settings.io_timeout = Duration::from_secs(5);
    settings.stop_timeout = Duration::from_millis(100);
    let (runtime, handle, _events) =
        spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    let start_handle = handle.clone();
    let start = thread::spawn(move || {
        block_on(start_handle.start(
            vec![RecordingCamera {
                id: 1,
                rtsp_url: "rtsp://camera.invalid/never-ready".into(),
            }],
            root,
        ))
    });
    wait_for_file(&marker_path(&ffmpeg, "pid"));
    let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
    let started = Instant::now();

    runtime.shutdown().unwrap();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(start.join().unwrap().is_err());
    assert!(!process_exists(&pid), "startup process {pid:?} leaked");
    assert!(block_on(handle.stop()).is_err());
}

#[test]
fn shutdown_interrupts_reconnect_delay() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "shutdown-reconnect-delay");
    let ffprobe = valid_runtime_ffprobe(&directory, "shutdown-reconnect-delay-probe");
    let mut settings = recorder_settings();
    settings.retry_delay = Duration::from_secs(5);
    let (runtime, handle, mut events) =
        spawn_with_executables(settings, ffmpeg.clone(), ffprobe).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    block_on(handle.start(
        vec![RecordingCamera {
            id: 1,
            rtsp_url: "rtsp://camera.invalid/reconnect".into(),
        }],
        root,
    ))
    .unwrap();
    fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
    wait_for_status(&mut events, 1, RecorderStatus::Reconnecting);
    let pid = fs::read_to_string(scenario_marker(&ffmpeg, "reconnect", "1.pid")).unwrap();
    let started = Instant::now();

    drop(runtime);

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!process_exists(&pid), "reconnect process {pid:?} leaked");
    assert_eq!(
        fs::read_to_string(marker_path(&ffmpeg, "paths"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert!(block_on(handle.stop()).is_err());
}

#[test]
fn shutdown_interrupts_ffprobe_and_reaps_it() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = supervision_ffmpeg(&directory, "shutdown-active-probe");
    let ffprobe = runtime_ffprobe(
        &directory,
        "shutdown-active-probe-ffprobe",
        r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
    );
    let mut settings = recorder_settings();
    settings.io_timeout = Duration::from_secs(5);
    let (runtime, handle, _events) =
        spawn_with_executables(settings, ffmpeg.clone(), ffprobe.clone()).unwrap();
    let root = valid_recordings_root(&directory, &[1]);
    block_on(handle.start(
        vec![RecordingCamera {
            id: 1,
            rtsp_url: "rtsp://camera.invalid/reconnect".into(),
        }],
        root,
    ))
    .unwrap();
    fs::write(marker_path(&ffmpeg, "reconnect-exit"), []).unwrap();
    wait_for_file(&marker_path(&ffprobe, "pid"));
    let pid = fs::read_to_string(marker_path(&ffprobe, "pid")).unwrap();
    assert!(process_exists(&pid));
    let started = Instant::now();

    runtime.shutdown().unwrap();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(!process_exists(&pid), "FFprobe process {pid:?} leaked");
    assert!(block_on(handle.stop()).is_err());
}

#[test]
fn ffmpeg_command_uses_tcp_timeout_video_copy_and_matroska() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-command",
        &format!(
            r#"FAKE_FFMPEG_ARGS="$0.args"
printf '%s\n' "$@" > "$FAKE_FFMPEG_ARGS"
for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.quit""#
        ),
    );
    let partial_path = directory.path().join(".attempt-command.partial.mkv");
    let command_args = args_path(&ffmpeg);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let stopper = thread::spawn(move || {
        wait_for_file(&command_args);
        stop_signal.store(true, Ordering::Relaxed);
    });

    run(
        attempt_config(
            &ffmpeg,
            "rtsp://camera.invalid/axis-media/media.amp",
            &partial_path,
        ),
        &stop,
        || Ok(10_000),
    )
    .0
    .unwrap();
    stopper.join().unwrap();

    let args = fs::read_to_string(args_path(&ffmpeg)).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        vec![
            "-loglevel",
            "level+info",
            "-hide_banner",
            "-n",
            "-rtsp_transport",
            "tcp",
            "-timeout",
            "750000",
            "-i",
            "rtsp://camera.invalid/axis-media/media.amp",
            "-map",
            "0:v:0",
            "-an",
            "-c:v",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
            "-f",
            "matroska",
            partial_path.to_str().unwrap(),
        ]
    );
}

#[test]
fn timeout_microseconds_are_checked_before_command_creation() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-timeout",
        r#"printf invoked > "$0.invoked""#,
    );
    let partial_path = directory.path().join(".attempt-timeout.partial.mkv");
    let stop = AtomicBool::new(true);
    let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
    config.io_timeout = Duration::MAX;

    let error = run(config, &stop, || Ok(10_000)).0.unwrap_err();

    assert!(!format!("{error:?} {error}").contains("camera.invalid"));
    assert!(!marker_path(&ffmpeg, "invoked").exists());
    assert!(!args_path(&ffmpeg).exists());
}

#[test]
fn recorder_errors_and_events_never_expose_rtsp_credentials() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-secret",
        r#"previous=
for argument in "$@"
do
    if [ "$previous" = "-i" ]; then
        source_url="$argument"
        break
    fi
    previous="$argument"
done
printf '%s\n' "$$" > "$0.pid"
printf '[info] Stream #0:0: Video: h264, yuv420p, 16x16, 1 fps, source=%s\n' "$source_url" >&2
exec sleep 30"#,
    );
    let partial_path = directory.path().join(".attempt-secret.partial.mkv");
    let secret_url = "rtsp://student:secret@camera.invalid/private";
    let stop = AtomicBool::new(false);
    let (result, events) = run(
        attempt_config(&ffmpeg, secret_url, &partial_path),
        &stop,
        || Ok(10_000),
    );

    let error = result.unwrap_err();
    let rendered = format!("{error:?} {error} {events:?}");
    assert!(!rendered.contains(secret_url));
    assert!(!rendered.contains("student:secret"));
    let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
    assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
}

#[test]
fn first_qualifying_progress_freezes_media_timeline_zero() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-first-progress",
        r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '[info] frame=    1 fps=1.0 q=-1.0 size=       1kB time=00:00:02.500 bitrate=   3.2kbits/s speed=1x' >&2"#,
    );
    let partial_path = directory.path().join(".attempt-first.partial.mkv");
    let stop = AtomicBool::new(false);

    let (result, events) = run(
        attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
        &stop,
        || Ok(10_000),
    );

    assert_eq!(result.unwrap().media_zero_utc_ms, Some(7_500));
    assert_eq!(
        events,
        vec![AttemptEvent::Ready {
            media_zero_utc_ms: 7_500
        }]
    );
}

#[test]
fn later_progress_does_not_move_media_timeline_zero() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-later-progress",
        &format!(
            r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
printf '%s\r' '[info] frame=    2 fps=1.0 q=-1.0 size=       2kB time=00:00:05.000 bitrate=   3.2kbits/s speed=1x' >&2"#
        ),
    );
    let partial_path = directory.path().join(".attempt-later.partial.mkv");
    let stop = AtomicBool::new(false);
    let clock_calls = Cell::new(0);

    let (result, events) = run(
        attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
        &stop,
        || {
            clock_calls.set(clock_calls.get() + 1);
            Ok(if clock_calls.get() == 1 {
                10_000
            } else {
                100_000
            })
        },
    );

    assert_eq!(clock_calls.get(), 1);
    assert_eq!(result.unwrap().media_zero_utc_ms, Some(9_000));
    assert_eq!(
        events,
        vec![AttemptEvent::Ready {
            media_zero_utc_ms: 9_000
        }]
    );
}

#[test]
fn progress_time_rejects_negative_and_non_finite_values() {
    let _process_test = crate::recording::process_test_guard();
    for value in ["-00:00:00.000", "-0.0000001", "01:-01:00", "NaN", "inf"] {
        assert!(parse_progress_time(value).is_none(), "accepted {value}");
    }
    assert_eq!(
        parse_progress_time("00:00:01.250").unwrap().as_micros(),
        1_250_000
    );
}

#[test]
fn readiness_requires_a_frame_and_nonempty_regular_output() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        (
            "ffmpeg-zero-frame",
            format!(
                r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{}' >&2"#,
                PROGRESS_ONE_SECOND.replace("frame=    1", "frame=    0")
            ),
            false,
        ),
        (
            "ffmpeg-empty-output",
            format!(
                r#"for output_path
do
    :
done
: > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
            ),
            false,
        ),
        (
            "ffmpeg-directory-output",
            format!(
                r#"for output_path
do
    :
done
mkdir "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
            ),
            false,
        ),
        (
            "ffmpeg-ready",
            format!(
                r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2"#
            ),
            true,
        ),
    ];

    for (name, body, expected_ready) in cases {
        let ffmpeg = write_script(&directory, name, &body);
        let partial_path = directory.path().join(format!(".{name}.partial.mkv"));
        let stop = AtomicBool::new(false);
        let (result, events) = run(
            attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
            &stop,
            || Ok(10_000),
        );

        assert_eq!(
            result.unwrap().media_zero_utc_ms.is_some(),
            expected_ready,
            "unexpected readiness for {name}"
        );
        assert_eq!(
            !events.is_empty(),
            expected_ready,
            "unexpected readiness event for {name}"
        );
    }
}

#[test]
fn graceful_stop_sends_q_and_reaps() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-graceful",
        &format!(
            r#"for output_path
do
    :
done
printf media > "$output_path"
printf '%s\r' '{PROGRESS_ONE_SECOND}' >&2
printf '%s\n' "$$" > "$0.pid"
quit=$(dd bs=1 count=1 2>/dev/null)
printf '%s' "$quit" > "$0.quit""#
        ),
    );
    let partial_path = directory.path().join(".attempt-graceful.partial.mkv");
    let pid_path = marker_path(&ffmpeg, "pid");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let stopper = thread::spawn(move || {
        wait_for_file(&pid_path);
        stop_signal.store(true, Ordering::Relaxed);
    });

    let result = run(
        attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path),
        &stop,
        || Ok(10_000),
    )
    .0
    .unwrap();
    stopper.join().unwrap();

    assert!(result.stopped);
    assert_eq!(
        fs::read_to_string(marker_path(&ffmpeg, "quit")).unwrap(),
        "q"
    );
    let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
    assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
}

#[test]
fn stop_timeout_kills_and_reaps() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffmpeg = write_script(
        &directory,
        "ffmpeg-forced",
        r#"printf '%s\n' "$$" > "$0.pid"
exec sleep 30"#,
    );
    let partial_path = directory.path().join(".attempt-forced.partial.mkv");
    let pid_path = marker_path(&ffmpeg, "pid");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);
    let stopper = thread::spawn(move || {
        wait_for_file(&pid_path);
        stop_signal.store(true, Ordering::Relaxed);
    });
    let mut config = attempt_config(&ffmpeg, "rtsp://camera.invalid/stream", &partial_path);
    config.stop_timeout = Duration::from_millis(100);
    let started = Instant::now();

    let result = run(config, &stop, || Ok(10_000)).0.unwrap();
    stopper.join().unwrap();

    assert!(result.stopped);
    assert!(started.elapsed() < Duration::from_secs(2));
    let pid = fs::read_to_string(marker_path(&ffmpeg, "pid")).unwrap();
    assert!(!process_exists(&pid), "fake FFmpeg process {pid:?} leaked");
}

#[test]
fn positive_container_start_adjusts_segment_bounds() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let partial_path = directory.path().join(".attempt-positive.partial.mkv");
    fs::write(&partial_path, b"media").unwrap();
    let ffprobe = fake_ffprobe(
        &directory,
        "ffprobe-positive",
        &valid_probe("0.0679", "2.0001"),
        0,
    );

    let segment = finalize_attempt(
        7,
        &partial_path,
        &ffprobe,
        Duration::from_secs(1),
        &AtomicBool::new(false),
        Some(10_000),
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        segment,
        RecordingSegment {
            camera_id: 7,
            start_utc_ms: 10_067,
            end_utc_ms: 12_001,
            path: directory.path().join("10067.mkv"),
        }
    );
    assert!(!partial_path.exists());
}

#[test]
fn reconnect_start_is_clamped_to_previous_end() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let partial_path = directory.path().join(".attempt-reconnect.partial.mkv");
    fs::write(&partial_path, b"media").unwrap();
    let ffprobe = fake_ffprobe(&directory, "ffprobe-reconnect", &valid_probe("0", "1"), 0);

    let segment = finalize_attempt(
        3,
        &partial_path,
        &ffprobe,
        Duration::from_secs(1),
        &AtomicBool::new(false),
        Some(9_500),
        Some(10_000),
    )
    .unwrap()
    .unwrap();

    assert_eq!(segment.start_utc_ms, 10_000);
    assert_eq!(segment.end_utc_ms, 11_000);
    assert_eq!(segment.path, directory.path().join("10000.mkv"));
}

#[test]
fn valid_attempt_is_promoted_without_overwrite() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let ffprobe = fake_ffprobe(&directory, "ffprobe-promote", &valid_probe("0", "1"), 0);
    let shutdown = AtomicBool::new(false);
    let first_partial = directory.path().join(".attempt-promote-first.partial.mkv");
    fs::write(&first_partial, b"first media").unwrap();

    let segment = finalize_attempt(
        1,
        &first_partial,
        &ffprobe,
        Duration::from_secs(1),
        &shutdown,
        Some(5_000),
        None,
    )
    .unwrap()
    .unwrap();

    assert_eq!(segment.path, directory.path().join("5000.mkv"));
    assert_eq!(fs::read(&segment.path).unwrap(), b"first media");
    assert!(!first_partial.exists());

    let second_partial = directory.path().join(".attempt-promote-second.partial.mkv");
    fs::write(&second_partial, b"second media").unwrap();
    finalize_attempt(
        1,
        &second_partial,
        &ffprobe,
        Duration::from_secs(1),
        &shutdown,
        Some(5_000),
        None,
    )
    .unwrap_err();

    assert_eq!(fs::read(&segment.path).unwrap(), b"first media");
    assert_eq!(fs::read(&second_partial).unwrap(), b"second media");
}

#[test]
fn empty_attempt_is_removed() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let partial_path = directory.path().join(".attempt-empty.partial.mkv");
    fs::write(&partial_path, []).unwrap();
    let ffprobe = write_script(
        &directory,
        "ffprobe-empty",
        r#"printf invoked > "$0.invoked"
exit 99"#,
    );

    let segment = finalize_attempt(
        1,
        &partial_path,
        &ffprobe,
        Duration::from_secs(1),
        &AtomicBool::new(false),
        Some(5_000),
        None,
    )
    .unwrap();

    assert!(segment.is_none());
    assert!(!partial_path.exists());
    assert!(!marker_path(&ffprobe, "invoked").exists());
}

#[test]
fn invalid_nonempty_attempt_is_retained() {
    let _process_test = crate::recording::process_test_guard();
    let directory = tempfile::tempdir().unwrap();
    let partial_path = directory.path().join(".attempt-invalid.partial.mkv");
    fs::write(&partial_path, b"invalid media").unwrap();
    let ffprobe = fake_ffprobe(
        &directory,
        "ffprobe-invalid",
        &json!({
            "streams": [],
            "format": {"start_time": "0", "duration": "1"}
        })
        .to_string(),
        0,
    );

    let segment = finalize_attempt(
        1,
        &partial_path,
        &ffprobe,
        Duration::from_secs(1),
        &AtomicBool::new(false),
        Some(5_000),
        None,
    )
    .unwrap();

    assert!(segment.is_none());
    assert_eq!(fs::read(&partial_path).unwrap(), b"invalid media");
}
