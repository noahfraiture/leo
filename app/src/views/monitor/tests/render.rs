use backend::recording::RecorderStatus;

use crate::{
    preview::PreviewState,
    test_support::{RenderHarness, assert_button_disabled, ready_preview},
};

fn assert_two_stable_previews(html: &str) {
    assert_eq!(html.matches("<video").count(), 2, "{html}");
    assert_eq!(html.matches("camera-0-video").count(), 1, "{html}");
    assert_eq!(html.matches("camera-1-video").count(), 1, "{html}");
    assert_eq!(html.matches("reader.js").count(), 1, "{html}");
    assert_eq!(html.matches("aria-pressed=true").count(), 1, "{html}");
}

#[test]
fn idle_monitor_offers_recording_and_stable_previews() {
    let harness = RenderHarness::new();
    let session_root = harness.operator().session_root.display().to_string();

    let html = harness.render(ready_preview());

    assert!(html.contains("Session idle"), "{html}");
    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains("Monitoring profile"), "{html}");
    assert!(html.contains(&session_root), "{html}");
    assert_two_stable_previews(&html);
}

#[test]
fn active_monitor_keeps_recording_health_and_analysis_controls_separate() {
    let mut harness = RenderHarness::new();
    let directory = harness.activate();
    harness
        .operator_mut()
        .select_camera(42)
        .expect("configured camera should be selected");
    harness.operator_mut().cameras[1].recorder_status = RecorderStatus::Reconnecting;

    let html = harness.render(ready_preview());

    assert!(html.contains("Session active"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_button_disabled(&html, "Stop session", false);
    assert!(
        html.contains(r#"aria-label="Analysis participation: Excluded""#),
        "{html}"
    );
    assert!(
        html.contains(r#"aria-label="Recorder status: Reconnecting""#),
        "{html}"
    );
    assert!(html.contains("Include in analysis"), "{html}");
    assert!(html.contains("Monitoring profile"), "{html}");
    assert_two_stable_previews(&html);
}

#[test]
fn faulted_session_shows_one_recovery_alert() {
    let mut harness = RenderHarness::new();
    let directory = harness.activate();
    harness
        .operator_mut()
        .begin_stop()
        .expect("active session should begin stopping");
    harness.operator_mut().finish_fault(
        directory.clone(),
        "Recorder Stop failed during cleanup".into(),
    );

    let html = harness.render(ready_preview());

    assert_eq!(html.matches(r#"role="alert""#).count(), 1, "{html}");
    assert_eq!(
        html.matches("Recorder Stop failed during cleanup").count(),
        1,
        "{html}"
    );
    assert!(html.contains("Recorder cleanup was attempted"), "{html}");
    assert!(html.contains("restart Leo"), "{html}");
    assert!(html.contains(&directory.display().to_string()), "{html}");
    assert_two_stable_previews(&html);
}

#[test]
fn preview_failure_does_not_disable_recording() {
    let html = RenderHarness::new().render(PreviewState::Unavailable {
        message: "preview bridge did not start".into(),
    });

    assert_button_disabled(&html, "Start session", false);
    assert!(html.contains("Live preview is unavailable"), "{html}");
    assert!(html.contains("preview bridge did not start"), "{html}");
    assert_eq!(html.matches("<video").count(), 0, "{html}");
    assert_eq!(html.matches("reader.js").count(), 0, "{html}");
}
