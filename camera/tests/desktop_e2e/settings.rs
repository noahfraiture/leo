//! Native Settings edits stay attached to their camera and survive delayed host rendering.

use super::*;

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_settings_typing_survives_delayed_render_and_camera_selection() {
    let (original, saved) = run_scenario("settings-text");
    assert_eq!(saved["cameras"][0]["name"], "abc");
    assert_eq!(saved["cameras"][1]["name"], "Second camera retained");
    for index in 0..2 {
        assert_eq!(
            saved["cameras"][index]["rtspUrl"],
            original["cameras"][index]["rtspUrl"]
        );
    }
    assert_eq!(saved["monitoringProfiles"][0]["name"], "abc");
    assert_eq!(saved["monitoringProfiles"][0]["sampleEveryMs"], 123);
    assert_eq!(saved["recorderTimeoutSecs"], 123);
    assert_eq!(saved["workflow"]["providers"][0]["name"], "abc");
    assert_eq!(saved["workflow"]["providers"][0]["apiKey"], "abc");
    assert_eq!(
        saved["workflow"]["providers"][0]["baseUrl"],
        original["workflow"]["providers"][0]["baseUrl"]
    );
    assert_eq!(saved["workflow"]["models"][0]["name"], "abc");
    assert_eq!(saved["workflow"]["models"][0]["model"], "abc");
    assert_eq!(saved["workflow"]["models"][0]["imageLimit"], 123);
    assert_eq!(saved["workflow"]["models"][0]["maxOutputTokens"], 123);
    assert_eq!(saved["workflow"]["modelSets"][0]["name"], "abc");
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_profile_removal_rechecks_camera_assignments() {
    let (original, saved) = run_scenario("profile-events");
    assert_eq!(saved["cameras"], original["cameras"]);
    assert_eq!(saved["monitoringProfiles"], original["monitoringProfiles"]);
    assert_eq!(saved["nextMonitoringProfileId"], 4);
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_camera_events_keep_the_other_camera_intact() {
    let (original, saved) = run_scenario("camera-events");
    assert_eq!(saved["cameras"].as_array().unwrap().len(), 1);
    assert_eq!(saved["cameras"][0]["id"], 2);
    assert_eq!(saved["cameras"][0]["name"], "Surviving camera");
    assert_eq!(
        saved["cameras"][0]["rtspUrl"],
        original["cameras"][1]["rtspUrl"]
    );
    assert_eq!(saved["workflow"], original["workflow"]);
    assert_eq!(saved["nextCameraId"], original["nextCameraId"]);
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_legacy_reader_events_stay_with_their_model() {
    let (original, saved) = run_scenario("model-set-events");
    assert_eq!(saved["workflow"]["models"].as_array().unwrap().len(), 1);
    assert_eq!(
        saved["workflow"]["models"][0],
        original["workflow"]["models"][1]
    );
    assert_eq!(
        saved["workflow"]["modelSets"][0]["readerIds"],
        serde_json::json!(["remaining-model"])
    );
    assert_eq!(
        saved["workflow"]["providers"],
        original["workflow"]["providers"]
    );
}

fn run_scenario(scenario: &str) -> (Value, Value) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let mock = MockOpenAi::start();
    let settings = write_desktop_settings(
        root.path(),
        &data,
        [
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
        ],
        5,
        "local-settings",
        "local-settings",
        Some(&format!("http://{}/v1", mock.address)),
    );
    let mut original: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    if scenario == "model-set-events" {
        // This settings-only boundary needs no camera or inference activity.
        original["cameras"] = serde_json::json!([]);
        let mut remaining = original["workflow"]["models"][0].clone();
        remaining["id"] = serde_json::json!("remaining-model");
        remaining["name"] = serde_json::json!("Remaining model");
        original["workflow"]["models"]
            .as_array_mut()
            .unwrap()
            .push(remaining);
        original["workflow"]["modelSets"][0]["readerIds"] = serde_json::json!([]);
        original["workflow"]["modelSets"][0]["reviewerId"] = serde_json::json!("remaining-model");
        original["workflow"]["modelSets"][0]["formatterId"] = serde_json::json!("remaining-model");
        fs::write(&settings, serde_json::to_vec_pretty(&original).unwrap()).unwrap();
    }
    let logs = root.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let ready = root.path().join("driver-ready");
    let result = root.path().join("driver-result");
    let mut app = start_desktop_app(&settings, &ready, &result, scenario, true, &logs);
    let message = wait_for_desktop_result(&mut app, &ready, &result);
    assert!(message.starts_with("ok\n"), "{message}");

    let saved: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    assert!(mock.requests.lock().unwrap().is_empty());
    (original, saved)
}
