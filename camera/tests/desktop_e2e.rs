#![cfg(all(unix, target_os = "macos"))]

use std::{
    env, ffi::OsStr, fs, net::TcpListener, os::unix::fs::PermissionsExt, path::Path,
    process::Command, sync::atomic::Ordering, time::Duration,
};

use backend::{
    analysis::AnalysisCheckpoint,
    recording::list_segments,
    session::{OperatorAction, list_sessions},
};
use serde_json::{Value, json};

#[path = "desktop_e2e/support.rs"]
mod support;

use support::{
    MockOpenAi, ProcessGuard, SHUTDOWN_TIMEOUT, assert_preview_ports_available, fixture,
    process_group_exists, read_application_log, real_openai_root, redacted_requests,
    request_contains_camera_frame, required_environment, start_camera, start_desktop_app,
    wait_for_desktop_result, write_desktop_settings,
};

const COMPLETE_ANALYSIS_SCENARIO: &str = "complete-analysis";
const ANALYSIS_RECOVERY_SCENARIO: &str = "analysis-recovery";
const RECORD_WITHOUT_PREVIEW_SCENARIO: &str = "record-without-preview";
const DEFAULT_FRAME_SETS_PER_PROMPT: usize = 5;

fn direct_openai_endpoint(base_url: Option<&OsStr>) -> Result<(), &'static str> {
    if base_url.is_some() {
        Err("OPENAI_BASE_URL must be unset; desktop paid validation targets OpenAI directly")
    } else {
        Ok(())
    }
}

#[test]
fn real_openai_endpoint_rejects_present_base_url_even_when_empty() {
    assert!(direct_openai_endpoint(None).is_ok());
    assert!(direct_openai_endpoint(Some(OsStr::new(""))).is_err());
    assert!(direct_openai_endpoint(Some(OsStr::new("custom-endpoint"))).is_err());
}

#[test]
#[ignore = "requires a macOS GUI session, MediaMTX, FFmpeg, and FFprobe"]
fn desktop_operator_flow_records_two_cameras_and_analyzes() {
    let real_openai = env::var("LEO_E2E_REAL_OPENAI").as_deref() == Ok("1");
    let real_provider = if real_openai {
        direct_openai_endpoint(env::var_os("OPENAI_BASE_URL").as_deref())
            .unwrap_or_else(|message| panic!("{message}"));
        assert_eq!(
            env::var("LEO_E2E_REAL_OPENAI").as_deref(),
            Ok("1"),
            "real OpenAI E2E requires LEO_E2E_REAL_OPENAI=1"
        );
        assert_eq!(
            env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
            Ok("1"),
            "real OpenAI E2E requires LEO_RUN_PAID_OPENAI_TEST=1"
        );
        Some((
            required_environment("OPENAI_API_KEY"),
            required_environment("ANALYSIS_MODEL"),
        ))
    } else {
        None
    };

    assert_preview_ports_available();
    let settings_directory = tempfile::tempdir().expect("create desktop E2E settings directory");
    let temporary = if real_openai {
        None
    } else {
        Some(tempfile::tempdir().expect("create desktop E2E root"))
    };
    let root = temporary
        .as_ref()
        .map(|temporary| temporary.path().to_owned())
        .unwrap_or_else(real_openai_root);
    if real_openai {
        eprintln!("real OpenAI E2E artifacts: {}", root.display());
    }
    let logs = root.join("process-logs");
    fs::create_dir(&logs).expect("create process log directory");
    let data_root = root.join("data");
    fs::create_dir(&data_root).expect("create E2E data root");

    let (camera_1_rtsp, mut camera_1) = start_camera("camera 1", &fixture("salon-1.mp4"), &logs);
    let (camera_2_rtsp, mut camera_2) = start_camera("camera 2", &fixture("salon-2.mp4"), &logs);

    let mock_openai = (!real_openai).then(MockOpenAi::start);
    let mock_base_url = mock_openai
        .as_ref()
        .map(|mock| format!("http://{}/v1", mock.address));
    let settings_path = if let Some((api_key, model)) = &real_provider {
        write_desktop_settings(
            settings_directory.path(),
            &data_root,
            [camera_1_rtsp, camera_2_rtsp],
            DEFAULT_FRAME_SETS_PER_PROMPT,
            api_key,
            model,
            None,
        )
    } else {
        write_desktop_settings(
            settings_directory.path(),
            &data_root,
            [camera_1_rtsp, camera_2_rtsp],
            DEFAULT_FRAME_SETS_PER_PROMPT,
            "local-e2e-key",
            "local-e2e-model",
            Some(
                mock_base_url
                    .as_deref()
                    .expect("mock mode should have a provider URL"),
            ),
        )
    };
    drop(real_provider);
    let driver_ready = root.join("driver-ready");
    let driver_result = root.join("driver-result");
    let mut app = start_desktop_app(
        &settings_path,
        &driver_ready,
        &driver_result,
        COMPLETE_ANALYSIS_SCENARIO,
        mock_openai.is_some(),
        &logs,
    );
    let result = wait_for_desktop_result(&mut app, &driver_ready, &driver_result);
    let rendered_summary = result
        .strip_prefix("ok\n")
        .unwrap_or_else(|| panic!("{result}\n{}", app.diagnostics()))
        .trim();
    assert!(!rendered_summary.is_empty(), "rendered summary was empty");

    let camera_1_status = camera_1.stop(SHUTDOWN_TIMEOUT);
    let camera_2_status = camera_2.stop(SHUTDOWN_TIMEOUT);
    assert!(camera_1_status.success(), "camera 1: {camera_1_status}");
    assert!(camera_2_status.success(), "camera 2: {camera_2_status}");
    camera_1.assert_process_group_exited(SHUTDOWN_TIMEOUT);
    camera_2.assert_process_group_exited(SHUTDOWN_TIMEOUT);

    let sessions = list_sessions(&data_root.join("sessions"))
        .expect("list E2E sessions")
        .sessions;
    assert_eq!(sessions.len(), 1, "expected one completed E2E session");
    let stored = &sessions[0];
    assert!(stored.session.actions.iter().any(|(_, action)| {
        matches!(
            action,
            OperatorAction::SetMonitoringProfile { camera_ids, monitoring_profile_id: 2 } if camera_ids == &[1]
        )
    }));

    let marker = fs::symlink_metadata(stored.directory.join("recording-complete"))
        .expect("read E2E completion marker");
    assert!(marker.file_type().is_file());
    assert_eq!(marker.len(), 0);

    let segments = list_segments(&stored.directory.join("recordings"), &[1, 2])
        .expect("discover E2E recording segments");
    assert!(segments.iter().any(|segment| segment.camera_id == 1));
    assert!(segments.iter().any(|segment| segment.camera_id == 2));

    let checkpoint =
        AnalysisCheckpoint::read(&stored.directory.join("analysis.json"), stored.session.id)
            .expect("read E2E analysis checkpoint");
    assert!(checkpoint.total_batches > 0);
    assert_eq!(checkpoint.responses.len(), checkpoint.total_batches);
    if let Some(mock) = &mock_openai {
        assert_eq!(rendered_summary, "E2E mock analysis complete.");
        assert!(
            checkpoint
                .responses
                .iter()
                .all(|response| response.sequence_summary == "E2E mock analysis complete.")
        );
        let requests = mock.requests.lock().expect("mock requests mutex");
        assert!(
            !requests.is_empty(),
            "mock OpenAI server received no requests"
        );
        for camera_id in [1, 2] {
            assert!(
                requests
                    .iter()
                    .any(|request| request_contains_camera_frame(request, camera_id)),
                "mock requests contained no JPEG frame for camera {camera_id}:\n{}",
                redacted_requests(&requests),
            );
        }
    }

    let application_log = read_application_log(&data_root.join("logs"));
    assert!(
        !application_log.contains("A Copy Value created"),
        "Dioxus reported a signal ownership violation:\n{application_log}"
    );
    assert!(application_log.contains("recorder runtime stopped"));
    assert!(application_log.contains("preview stopped"));
    assert!(!application_log.contains("recorder runtime shutdown failed"));
    assert!(!application_log.contains("preview stop failed"));
}

#[test]
#[ignore = "requires a macOS GUI session, MediaMTX, FFmpeg, and FFprobe"]
fn desktop_analysis_resumes_after_transient_provider_failure() {
    assert_preview_ports_available();
    let settings_directory = tempfile::tempdir().expect("create desktop E2E settings directory");
    let root = tempfile::tempdir().expect("create desktop E2E root");
    let logs = root.path().join("process-logs");
    fs::create_dir(&logs).expect("create process log directory");
    let data_root = root.path().join("data");
    fs::create_dir(&data_root).expect("create E2E data root");

    let (camera_1_rtsp, mut camera_1) = start_camera("camera 1", &fixture("salon-1.mp4"), &logs);
    let (camera_2_rtsp, mut camera_2) = start_camera("camera 2", &fixture("salon-2.mp4"), &logs);
    let mock_openai = MockOpenAi::fail_once_on_request(2);
    let mock_base_url = format!("http://{}/v1", mock_openai.address);
    let settings_path = write_desktop_settings(
        settings_directory.path(),
        &data_root,
        [camera_1_rtsp, camera_2_rtsp],
        1,
        "local-e2e-key",
        "local-e2e-model",
        Some(&mock_base_url),
    );

    let driver_ready = root.path().join("driver-ready");
    let driver_result = root.path().join("driver-result");
    let mut app = start_desktop_app(
        &settings_path,
        &driver_ready,
        &driver_result,
        ANALYSIS_RECOVERY_SCENARIO,
        true,
        &logs,
    );
    let result = wait_for_desktop_result(&mut app, &driver_ready, &driver_result);
    let mut result_lines = result.lines();
    assert_eq!(
        result_lines.next(),
        Some("ok"),
        "{result}\n{}",
        app.diagnostics()
    );
    let partial_progress = result_lines
        .next()
        .unwrap_or_else(|| panic!("driver did not report partial progress: {result}"));
    let rendered_summary = result_lines
        .next()
        .unwrap_or_else(|| panic!("driver did not report the final summary: {result}"));
    assert_eq!(
        result_lines.next(),
        None,
        "unexpected driver output: {result}"
    );

    let camera_1_status = camera_1.stop(SHUTDOWN_TIMEOUT);
    let camera_2_status = camera_2.stop(SHUTDOWN_TIMEOUT);
    assert!(camera_1_status.success(), "camera 1: {camera_1_status}");
    assert!(camera_2_status.success(), "camera 2: {camera_2_status}");
    camera_1.assert_process_group_exited(SHUTDOWN_TIMEOUT);
    camera_2.assert_process_group_exited(SHUTDOWN_TIMEOUT);

    let sessions = list_sessions(&data_root.join("sessions"))
        .expect("list E2E sessions")
        .sessions;
    assert_eq!(sessions.len(), 1, "expected one completed E2E session");
    let stored = &sessions[0];
    let checkpoint =
        AnalysisCheckpoint::read(&stored.directory.join("analysis.json"), stored.session.id)
            .expect("read resumed E2E analysis checkpoint");
    assert!(
        checkpoint.total_batches > 1,
        "recovery scenario needs more than one batch"
    );
    assert_eq!(checkpoint.responses.len(), checkpoint.total_batches);
    assert_eq!(
        partial_progress,
        format!(
            "Analysis progress: 1 of {} batches",
            checkpoint.total_batches
        )
    );
    assert_eq!(rendered_summary, "E2E mock analysis complete.");
    assert_eq!(mock_openai.failed_requests.load(Ordering::SeqCst), 1);

    let requests = mock_openai.requests.lock().expect("mock requests mutex");
    assert_eq!(
        requests.len(),
        checkpoint.total_batches + 1,
        "only the failed provider request should be repeated"
    );
    assert_ne!(
        requests[0], requests[1],
        "the first batch must not be retried"
    );
    assert_eq!(
        requests[1], requests[2],
        "Resume should retry the failed batch from the saved prefix"
    );

    let application_log = read_application_log(&data_root.join("logs"));
    assert!(application_log.contains("analysis failed"));
    assert!(application_log.contains("analysis resumed"));
    assert!(application_log.contains("analysis completed"));
    assert!(!application_log.contains("recorder runtime shutdown failed"));
    assert!(!application_log.contains("preview stop failed"));
}

#[test]
#[ignore = "requires a macOS GUI session, MediaMTX, FFmpeg, and FFprobe"]
fn desktop_recording_remains_usable_without_preview() {
    let _occupied_preview_port =
        TcpListener::bind(("127.0.0.1", 8889)).expect("occupy preview TCP port 8889");
    let settings_directory = tempfile::tempdir().expect("create desktop E2E settings directory");
    let root = tempfile::tempdir().expect("create desktop E2E root");
    let logs = root.path().join("process-logs");
    fs::create_dir(&logs).expect("create process log directory");
    let data_root = root.path().join("data");
    fs::create_dir(&data_root).expect("create E2E data root");

    let (camera_1_rtsp, mut camera_1) = start_camera("camera 1", &fixture("salon-1.mp4"), &logs);
    let (camera_2_rtsp, mut camera_2) = start_camera("camera 2", &fixture("salon-2.mp4"), &logs);
    let mock_openai = MockOpenAi::start();
    let mock_base_url = format!("http://{}/v1", mock_openai.address);
    let settings_path = write_desktop_settings(
        settings_directory.path(),
        &data_root,
        [camera_1_rtsp, camera_2_rtsp],
        DEFAULT_FRAME_SETS_PER_PROMPT,
        "local-e2e-key",
        "local-e2e-model",
        Some(&mock_base_url),
    );

    let driver_ready = root.path().join("driver-ready");
    let driver_result = root.path().join("driver-result");
    let mut app = start_desktop_app(
        &settings_path,
        &driver_ready,
        &driver_result,
        RECORD_WITHOUT_PREVIEW_SCENARIO,
        true,
        &logs,
    );
    let result = wait_for_desktop_result(&mut app, &driver_ready, &driver_result);
    assert_eq!(
        result,
        "ok\nrecording continued across metadata failure and a second session",
        "{}",
        app.diagnostics()
    );

    let camera_1_status = camera_1.stop(SHUTDOWN_TIMEOUT);
    let camera_2_status = camera_2.stop(SHUTDOWN_TIMEOUT);
    assert!(camera_1_status.success(), "camera 1: {camera_1_status}");
    assert!(camera_2_status.success(), "camera 2: {camera_2_status}");
    camera_1.assert_process_group_exited(SHUTDOWN_TIMEOUT);
    camera_2.assert_process_group_exited(SHUTDOWN_TIMEOUT);

    let catalog = list_sessions(&data_root.join("sessions")).expect("list E2E sessions");
    assert_eq!(
        catalog.incomplete.len(),
        1,
        "metadata failure retains its recording folder"
    );
    let interrupted = &catalog.incomplete[0];
    let events =
        fs::read_to_string(interrupted.join("events.jsonl")).expect("read last saved events");
    assert!(events.contains("monitoring_profile_changed"));
    assert!(events.contains("camera_participation_changed"));
    assert!(!events.contains("session_ended"));
    let retained =
        list_segments(&interrupted.join("recordings"), &[1, 2]).expect("probe retained video");
    assert_eq!(retained.len(), 2);
    for segment in retained {
        assert!(
            segment.end_utc_ms - segment.start_utc_ms >= 4500,
            "video must span the failure"
        );
        let decoded = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(&segment.path)
            .args(["-f", "null", "-"])
            .output()
            .expect("decode retained video");
        assert!(decoded.status.success(), "retained video must be playable");
    }
    let sessions = catalog.sessions;
    assert_eq!(sessions.len(), 1, "expected one completed E2E session");
    let stored = &sessions[0];
    let marker = fs::symlink_metadata(stored.directory.join("recording-complete"))
        .expect("read E2E completion marker");
    assert!(marker.file_type().is_file());
    assert_eq!(marker.len(), 0);
    let segments = list_segments(&stored.directory.join("recordings"), &[1, 2])
        .expect("discover E2E recording segments");
    assert!(segments.iter().any(|segment| segment.camera_id == 1));
    assert!(segments.iter().any(|segment| segment.camera_id == 2));
    assert!(
        mock_openai
            .requests
            .lock()
            .expect("mock requests mutex")
            .is_empty(),
        "record-only scenario should not call the analysis provider"
    );

    let application_log = read_application_log(&data_root.join("logs"));
    assert!(application_log.contains("preview unavailable"));
    assert!(application_log.contains("preview port 127.0.0.1:8889 is unavailable"));
    assert!(application_log.contains("recorder runtime stopped"));
    assert!(!application_log.contains("recorder runtime shutdown failed"));
    assert!(!application_log.contains("preview stop failed"));
}

#[test]
fn desktop_settings_file_is_strict_private_and_complete() {
    let settings_directory = tempfile::tempdir().expect("create settings directory");
    let data_directory = tempfile::tempdir().expect("create data directory");
    let data_root = data_directory.path().join("data");
    fs::create_dir(&data_root).expect("create data root");
    let camera_addresses = [
        "127.0.0.1:8554".parse().expect("parse camera 1 address"),
        "127.0.0.1:8555".parse().expect("parse camera 2 address"),
    ];
    let mock_base_url = "http://127.0.0.1:3000/v1";

    let path = write_desktop_settings(
        settings_directory.path(),
        &data_root,
        camera_addresses,
        DEFAULT_FRAME_SETS_PER_PROMPT,
        "local-e2e-key",
        "local-e2e-model",
        Some(mock_base_url),
    );

    let bytes = fs::read(&path).expect("read desktop settings");
    assert!(bytes.ends_with(b"\n"), "settings should end with a newline");
    let settings: Value = serde_json::from_slice(&bytes).expect("parse desktop settings");
    let expected = json!({
        "schemaVersion": 3,
        "nextCameraId": 3,
        "cameras": [
            {
                "id": 1,
                "name": "Salon 1",
                "rtspUrl": format!("rtsp://{}/axis-media/media.amp", camera_addresses[0]),
                "initiallyIncludedInAnalysis": true,
                "initialMonitoringProfileId": 1
            },
            {
                "id": 2,
                "name": "Salon 2",
                "rtspUrl": format!("rtsp://{}/axis-media/media.amp", camera_addresses[1]),
                "initiallyIncludedInAnalysis": true,
                "initialMonitoringProfileId": 2
            }
        ],
        "dataRoot": data_root,
        "recorderTimeoutSecs": 10,
        "monitoringProfiles": [
            {"id": 1, "name": "Standard", "sampleEveryMs": 1000},
            {"id": 2, "name": "Stable", "sampleEveryMs": 2000}
        ],
        "nextMonitoringProfileId": 3,
        "analysisProfiles": [{"id": 1, "name": "Fixture", "model": "local-e2e-model", "maxImagesPerPrompt": 10, "maxPromptSpanMs": 4000, "overlapFrameSets": 0, "imageSize": "original", "imageDetail": "providerDefault", "maxOutputTokens": null}],
        "nextAnalysisProfileId": 2,
        "defaultAnalysisProfileId": 1,
        "openai": {
            "apiKey": "local-e2e-key",
            "baseUrl": mock_base_url
        },
        "logLevel": "info"
    });
    assert!(
        settings == expected,
        "settings should match the complete strict E2E schema"
    );
    assert!(
        settings
            .get("dataRoot")
            .and_then(Value::as_str)
            .is_some_and(|path| Path::new(path).is_absolute() && Path::new(path) == data_root),
        "settings should contain the absolute E2E data root"
    );

    let mode = fs::metadata(path)
        .expect("read settings metadata")
        .permissions()
        .mode()
        & 0o777;
    assert!(mode == 0o600, "settings should have mode 0o600");
}

#[test]
fn process_group_probe_detects_a_live_descendant_after_the_leader_exits() {
    let temporary = tempfile::tempdir().expect("create process group test root");
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & exit 0"]);
    let mut process = ProcessGuard::spawn("process group probe", &mut command, temporary.path());
    let status = process
        .wait_until(Duration::from_secs(2))
        .expect("wait for process group leader")
        .expect("process group leader should exit");

    assert!(status.success());
    assert!(
        process_group_exists(process.process_group).expect("probe process group"),
        "background descendant should keep the process group alive"
    );
}

#[test]
fn camera_frame_request_requires_a_source_followed_by_a_jpeg() {
    let request = json!({
        "input": [
            {"content": [{"type": "input_text", "text": "Frame source: camera 1 (Front) at 00:00:00.000"}]},
            {"content": [{"type": "input_image", "image_url": "data:image/jpeg;base64,abc"}]},
            {"content": [{"type": "input_text", "text": "Frame source: camera 2 (Side) at 00:00:00.000"}]}
        ]
    });

    assert!(request_contains_camera_frame(&request, 1));
    assert!(!request_contains_camera_frame(&request, 2));
}

#[test]
fn request_diagnostics_redact_every_image_url() {
    let diagnostics = redacted_requests(&[json!({
        "input": [{
            "content": [{
                "type": "input_image",
                "image_url": "sensitive image without the expected prefix"
            }]
        }]
    })]);

    assert!(!diagnostics.contains("sensitive image"));
    assert!(diagnostics.contains("<redacted image>"));
}
