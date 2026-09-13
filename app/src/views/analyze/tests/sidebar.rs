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
