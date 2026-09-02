use backend::recording::RecorderStatus;

use crate::{
    preview::PreviewState,
    test_support::{RenderHarness, assert_button_disabled, opening_tag_before, ready_preview},
};

fn assert_stable_previews(html: &str) {
    assert_eq!(html.matches("<video").count(), 2, "{html}");
    assert_eq!(html.matches("camera-0-video").count(), 1, "{html}");
    assert_eq!(html.matches("camera-1-video").count(), 1, "{html}");
    assert_eq!(html.matches("<script").count(), 1, "{html}");
    assert_eq!(html.matches("reader.js").count(), 1, "{html}");
    assert!(html.contains("defer"), "{html}");
    assert_eq!(
        html.matches(r#"aria-label="Selected Salon "#).count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches(r#"aria-label="Select Salon "#).count(),
        1,
        "{html}"
    );
    assert_eq!(html.matches("aria-pressed=true").count(), 1, "{html}");
    let selected_button = opening_tag_before(html, "button", ">Selected</button>");
    assert!(
        selected_button.contains(r#"aria-label="Selected Salon "#),
        "{selected_button}"
    );
}

fn assert_no_placeholder_claims(html: &str) {
    for placeholder in ["LIVE", "14:42:18", "CAM 04", "Camera options"] {
        assert!(
            !html.contains(placeholder),
            "found placeholder claim {placeholder:?} in {html}"
        );
    }
}

#[test]
fn idle_renders_start_selection_storage_and_accessible_preview_controls() {
    let harness = RenderHarness::new();
    let session_root = harness.workflow().session_root.display().to_string();

    let html = harness.render(ready_preview());

    assert!(html.contains("Session idle"), "{html}");
    assert!(html.contains(r#"role="status""#), "{html}");
    assert!(html.contains(r#"aria-live="polite""#), "{html}");
    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains("Selected camera"), "{html}");
    assert!(
        html.contains("Initial sampling interval: 1 second"),
        "{html}"
    );
    assert!(html.contains(&session_root), "{html}");
    assert_stable_previews(&html);
    assert_no_placeholder_claims(&html);
}

#[test]
fn starting_renders_per_camera_readiness_without_a_fake_cancel_action() {
    let mut harness = RenderHarness::new();
    harness.start();
    harness.workflow_mut().cameras[0].recorder_status = RecorderStatus::Recording;

    let html = harness.render(ready_preview());

    assert!(html.contains("Starting session"), "{html}");
    assert_button_disabled(&html, "Start session", true);
    assert!(
        html.contains(r#"aria-label="Salon 1 recorder status: Recording""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Salon 2 recorder status: Starting""#),
        "{html}"
    );
    assert!(!html.contains("Cancel"), "{html}");
    assert!(html.contains("Staging directory"), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn active_recording_keeps_excluded_preview_mounted_and_enables_operator_controls() {
    let mut harness = RenderHarness::new();
    let directory = harness.activate();
    harness
        .workflow_mut()
        .select_camera(42)
        .expect("excluded camera should be selected");

    let html = harness.render(ready_preview());

    assert!(html.contains("Session active"), "{html}");
    assert!(html.contains("Elapsed time:"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_button_disabled(&html, "Stop session", false);
    assert!(
        html.contains(r#"aria-label="Analysis participation: Excluded""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Recorder status: Recording""#),
        "{html}"
    );
    assert!(html.contains("Include in analysis"), "{html}");
    assert!(html.contains("Sampling interval (seconds)"), "{html}");
    assert!(html.contains("Apply cadence"), "{html}");
    assert_stable_previews(&html);
    assert_no_placeholder_claims(&html);
}

#[test]
fn reconnecting_health_stays_separate_from_analysis_participation() {
    let mut harness = RenderHarness::new();
    harness.activate();
    harness.workflow_mut().cameras[1].recorder_status = RecorderStatus::Reconnecting;

    let html = harness.render(ready_preview());

    assert!(
        html.contains(r#"aria-label="Analysis participation: Excluded""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Recorder status: Reconnecting""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Salon 2 recorder status: Reconnecting""#),
        "{html}"
    );
    assert_button_disabled(&html, "Stop session", false);
    assert_stable_previews(&html);
}

#[test]
fn stopping_disables_session_controls_and_reports_finalization() {
    let mut harness = RenderHarness::new();
    let directory = harness.activate();
    harness
        .workflow_mut()
        .begin_stop()
        .expect("active render state should begin stopping");

    let html = harness.render(ready_preview());

    assert!(html.contains("Finalizing session"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_button_disabled(&html, "Stop session", true);
    assert!(!html.contains("Apply cadence"), "{html}");
    assert!(!html.contains("Include in analysis"), "{html}");
    assert!(!html.contains("Exclude from analysis"), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn faulted_session_renders_one_assertive_alert_and_recovery_guidance() {
    let mut harness = RenderHarness::new();
    let directory = harness.activate();
    harness
        .workflow_mut()
        .begin_stop()
        .expect("active render state should begin stopping");
    harness.workflow_mut().finish_fault(
        directory.clone(),
        "Recorder Stop failed during cleanup".into(),
    );

    let html = harness.render(ready_preview());

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(
        html.matches(r#"aria-live="assertive""#).count(),
        1,
        "{html}"
    );
    assert_eq!(
        html.matches("Recorder Stop failed during cleanup").count(),
        1,
        "{html}"
    );
    assert!(html.contains("Recorder cleanup was attempted"), "{html}");
    assert!(html.contains("restart Leo"), "{html}");
    assert!(html.contains("Faulted session directory"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_stable_previews(&html);
}

#[test]
fn unavailable_preview_keeps_recording_available_without_media_elements() {
    let harness = RenderHarness::new();
    let preview = PreviewState::Unavailable {
        message: "preview bridge did not start".into(),
    };

    let html = harness.render(preview);

    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains(r#"role="alert""#), "{html}");
    assert!(html.contains("Live preview is unavailable"), "{html}");
    assert!(html.contains("preview bridge did not start"), "{html}");
    assert!(html.contains("restart the app"), "{html}");
    assert_eq!(html.matches("<video").count(), 0, "{html}");
    assert_eq!(html.matches("reader.js").count(), 0, "{html}");
}
