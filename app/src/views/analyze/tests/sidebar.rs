use super::row_status;
use crate::analysis::{AnalysisPageState, test_snapshot};
use backend::analysis::RunStage;
use uuid::Uuid;

#[test]
fn sidebar_distinguishes_unloaded_history_paused_work_and_ready_results() {
    let id = Uuid::new_v4();
    let mut page = AnalysisPageState::default();
    assert_eq!(row_status(&page, id), "Select to load runs");
    let mut run = test_snapshot(id, Uuid::new_v4(), RunStage::Paused);
    page.apply_snapshot(run.clone());
    assert_eq!(row_status(&page, id), "Processing paused");
    run.checkpoint.stage = RunStage::Ready;
    page.apply_snapshot(run);
    assert_eq!(row_status(&page, id), "Ready for review");
    page.sessions.get_mut(&id).unwrap().error = Some("Could not load".into());
    assert_eq!(row_status(&page, id), "Needs attention");
}

#[test]
fn repair_list_distinguishes_current_capture_from_retained_or_failed_recordings() {
    use crate::{
        operator::SessionRunState,
        test_support::{RenderHarness, ready_preview},
    };

    for (phase, needs_repair) in [
        ("starting", false),
        ("active", false),
        ("stopping", false),
        ("idle", true),
        ("faulted", true),
        ("metadata-error", true),
    ] {
        let mut harness = RenderHarness::new();
        let directory = harness.activate();
        let retained = directory.parent().unwrap().join("older-retained-recording");
        std::fs::create_dir_all(retained.join("recordings")).unwrap();
        harness.operator_mut().refresh_sessions().unwrap();
        // Change lifecycle after discovery too, so a later fault immediately reveals its repair row.
        match phase {
            "starting" => {
                harness.operator_mut().session = SessionRunState::Starting {
                    directory: directory.clone(),
                }
            }
            "stopping" => {
                harness.operator_mut().begin_stop().unwrap();
            }
            "idle" => harness.operator_mut().session = SessionRunState::Idle,
            "faulted" => {
                harness
                    .operator_mut()
                    .begin_fault("Recorder failed".into(), true)
                    .unwrap();
            }
            "metadata-error" => {
                harness.operator_mut().metadata_error = Some("Metadata failed".into())
            }
            _ => {}
        }

        let html = harness.render_at(ready_preview(), "/analyze");
        assert!(html.contains("older-retained-recording"), "{phase}");
        assert_eq!(
            html.contains(&directory.display().to_string()),
            needs_repair,
            "{phase}"
        );
    }
}
