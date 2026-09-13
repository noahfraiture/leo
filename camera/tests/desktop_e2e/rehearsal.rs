//! Explicit long capture rehearsal against fixture RTSP streams and the production desktop.

use super::*;
use std::path::PathBuf;

#[test]
#[ignore = "requires a macOS GUI session and a new LEO_E2E_OUTPUT_DIR"]
fn desktop_profiles_save_and_reload() {
    assert!(env::var_os("LEO_E2E_OUTPUT_DIR").is_some());
    assert_preview_ports_available();
    let root = real_openai_root();
    let data = root.join("data");
    fs::create_dir(&data).unwrap();
    let mock = MockOpenAi::start();
    let settings = write_desktop_settings(
        &root,
        &data,
        [
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
        ],
        5,
        "local-profiles",
        "local-profiles",
        Some(&format!("http://{}/v1", mock.address)),
    );
    // No cameras or capture are needed to validate profile persistence.
    let mut config: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    config["cameras"] = json!([]);
    fs::write(&settings, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    for scenario in ["save-profiles", "reload-profiles"] {
        let logs = root.join(scenario);
        fs::create_dir(&logs).unwrap();
        let ready = logs.join("driver-ready");
        let result = logs.join("driver-result");
        let mut app = start_desktop_app(&settings, &ready, &result, scenario, true, &logs);
        let message = wait_for_desktop_result(&mut app, &ready, &result);
        assert!(message.starts_with("ok\n"), "{message}");
    }
    let saved: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert_eq!(saved["workflow"]["execution"]["memoryBudgetMib"], 4096);
    assert_eq!(saved["workflow"]["execution"]["maxConcurrentJobs"], 3);
    assert_eq!(saved["monitoringProfiles"].as_array().unwrap().len(), 3);
    assert_eq!(saved["workflow"]["models"][0]["imageLimit"], 24);
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[test]
#[ignore = "takes 15 minutes; requires LEO_DESKTOP_E2E_REHEARSAL=1 and a new LEO_E2E_OUTPUT_DIR"]
fn desktop_capture_rehearsal() {
    assert_eq!(env::var("LEO_DESKTOP_E2E_REHEARSAL").as_deref(), Ok("1"));
    assert!(env::var_os("LEO_E2E_OUTPUT_DIR").is_some());
    assert_preview_ports_available();
    let root = real_openai_root();
    let logs = root.join("process-logs");
    fs::create_dir(&logs).unwrap();
    let data = root.join("data");
    fs::create_dir(&data).unwrap();
    let settings = tempfile::tempdir().unwrap();
    // A private source file allows one real camera or an owned fault-injection proxy.
    // Credentials stay in the temporary settings file and are never included in test output.
    let physical_source = env::var_os("LEO_E2E_PHYSICAL_SOURCE_FILE").map(|path| {
        let source: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        source["rtsp_url"]
            .as_str()
            .expect("physical source URL is required")
            .to_owned()
    });
    let expect_reconnect = env::var("LEO_E2E_EXPECT_RECONNECT").as_deref() == Ok("1");
    assert!(!expect_reconnect || physical_source.is_some());
    let (address_1, mut camera_1) = if physical_source.is_some() {
        ("127.0.0.1:1".parse().unwrap(), None)
    } else {
        let (address, camera) = start_camera("camera 1", &fixture("salon-1.mp4"), &logs);
        (address, Some(camera))
    };
    let (address_2, mut camera_2) = start_camera("camera 2", &fixture("salon-2.mp4"), &logs);
    let mock = MockOpenAi::start();
    let path = write_desktop_settings(
        settings.path(),
        &data,
        [address_1, address_2],
        5,
        "local-rehearsal",
        "local-rehearsal",
        Some(&format!("http://{}/v1", mock.address)),
    );
    if let Some(source) = physical_source {
        let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        config["cameras"][0]["rtspUrl"] = json!(source);
        config["cameras"][0]["name"] = json!("Physical Axis");
        fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    }
    let ready = root.join("driver-ready");
    let result = root.join("driver-result");
    let mut app = start_desktop_app(&path, &ready, &result, "recording-rehearsal", true, &logs);
    let message = wait_for_desktop_result(&mut app, &ready, &result);
    assert!(message.starts_with("ok\n"), "{message}");
    if let Some(camera) = &mut camera_1 {
        assert!(camera.stop(SHUTDOWN_TIMEOUT).success());
    }
    assert!(camera_2.stop(SHUTDOWN_TIMEOUT).success());
    assert!(
        mock.requests.lock().unwrap().is_empty(),
        "capture sent a model request"
    );
    validate_retained_rehearsal(&root, expect_reconnect);
}

#[test]
#[ignore = "requires an existing completed rehearsal in LEO_E2E_OUTPUT_DIR"]
fn desktop_validate_retained_rehearsal() {
    let root = PathBuf::from(env::var_os("LEO_E2E_OUTPUT_DIR").expect("existing rehearsal root"));
    assert!(root.is_absolute());
    let message = fs::read_to_string(root.join("driver-result")).unwrap();
    assert!(message.starts_with("ok\n"), "{message}");
    validate_retained_rehearsal(
        &root,
        env::var("LEO_E2E_EXPECT_RECONNECT").as_deref() == Ok("1"),
    );
}

/// Recheck saved media independently of capture so validation fixes need no new recording.
fn validate_retained_rehearsal(root: &Path, expect_reconnect: bool) {
    let data = root.join("data");
    let sessions = list_sessions(&data.join("sessions")).unwrap().sessions;
    assert_eq!(sessions.len(), 3);
    assert!(
        sessions
            .iter()
            .any(|stored| stored.session.end_offset >= Duration::from_secs(900))
    );
    let mut media = Vec::new();
    let mut coverage = Vec::new();
    for stored in &sessions {
        let marker = stored.directory.join("recording-complete");
        assert_eq!(fs::metadata(marker).unwrap().len(), 0);
        let segments = list_segments(&stored.directory.join("recordings"), &[1, 2]).unwrap();
        for camera_id in [1, 2] {
            let mut camera: Vec<_> = segments
                .iter()
                .filter(|segment| segment.camera_id == camera_id)
                .collect();
            assert!(!camera.is_empty());
            camera.sort_by_key(|segment| segment.start_utc_ms);
            let retained_ms: i64 = camera
                .iter()
                .map(|segment| segment.end_utc_ms - segment.start_utc_ms)
                .sum();
            let session_ms = stored.session.end_offset.as_millis() as i64;
            let start_ms = stored.session.start_utc_ms;
            let end_ms = start_ms + session_ms;
            let covered_ms: i64 = camera
                .iter()
                .map(|segment| {
                    (segment.end_utc_ms.min(end_ms) - segment.start_utc_ms.max(start_ms)).max(0)
                })
                .sum();
            let interrupted = expect_reconnect && camera_id == 1 && session_ms >= 900_000;
            let allowance = if interrupted { 30_000 } else { 5000 };
            let gap_limit = if interrupted { 30_000 } else { 1000 };
            assert!(
                covered_ms >= session_ms - allowance,
                "capture exceeded the permitted missing duration"
            );
            assert!(
                camera
                    .windows(2)
                    .all(|pair| (0..=gap_limit)
                        .contains(&(pair[1].start_utc_ms - pair[0].end_utc_ms))),
                "unexpected segment overlap or gap"
            );
            if interrupted {
                assert!(
                    camera.len() > 1,
                    "physical camera did not reconnect into a new segment"
                );
            }
            coverage.push(json!({"session_id":stored.session.id,"camera_id":camera_id,"session_ms":session_ms,"retained_ms":retained_ms,"covered_ms":covered_ms,"missing_ms":session_ms-covered_ms,"segments":camera.len()}));
        }
        // Decode every frame using its original time base. The default output time base
        // can round distinct variable-rate camera timestamps to the same frame number.
        for segment in segments {
            let output = Command::new("ffmpeg")
                .args(["-nostdin", "-v", "error", "-xerror", "-i"])
                .arg(&segment.path)
                .args([
                    "-map",
                    "0:v:0",
                    "-fps_mode",
                    "passthrough",
                    "-enc_time_base",
                    "demux",
                    "-f",
                    "null",
                    "-",
                ])
                .output()
                .expect("decode rehearsal media");
            assert!(
                output.status.success() && output.stderr.is_empty(),
                "retained media failed full decode: {}: {}",
                segment.path.display(),
                String::from_utf8_lossy(&output.stderr)
            );
            media.push(segment.path);
        }
    }
    fs::write(
        root.join("validated-media.json"),
        serde_json::to_vec_pretty(&media).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("coverage.json"),
        serde_json::to_vec_pretty(&coverage).unwrap(),
    )
    .unwrap();
}
