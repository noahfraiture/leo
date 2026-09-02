use backend::analysis::{AnalysisCheckpoint, AnalysisResponse, AnalysisWarning};
use uuid::Uuid;

use super::row_status;

fn checkpoint(
    total_batches: usize,
    response_count: usize,
    warnings: Vec<AnalysisWarning>,
) -> Result<Option<AnalysisCheckpoint>, String> {
    let response = AnalysisResponse {
        observations: Vec::new(),
        sequence_summary: String::new(),
        checklist_progress: Vec::new(),
    };
    Ok(Some(AnalysisCheckpoint {
        schema_version: 2,
        session_id: Uuid::from_u128(1),
        checklist: "Complete the exercise".into(),
        plan_fingerprint: "0123456789abcdef".into(),
        total_batches,
        warnings,
        responses: vec![response; response_count],
    }))
}

#[test]
fn row_status_uses_the_approved_priority_and_zero_response_rule() {
    let none = Ok(None);
    let invalid = Err("invalid".into());
    let zero = checkpoint(2, 0, Vec::new());
    let partial = checkpoint(2, 1, Vec::new());
    let complete = checkpoint(1, 1, Vec::new());
    let warning = checkpoint(
        1,
        1,
        vec![AnalysisWarning::RecordingGap {
            camera_id: 1,
            start_offset_ms: 0,
            end_offset_ms: 1,
        }],
    );

    assert_eq!(row_status(&invalid, true, true), "Invalid checkpoint");
    assert_eq!(row_status(&none, true, true), "Running");
    assert_eq!(row_status(&none, false, true), "Failed");
    assert_eq!(row_status(&zero, false, false), "In progress");
    assert_eq!(row_status(&partial, false, false), "In progress");
    assert_eq!(row_status(&complete, false, false), "Complete");
    assert_eq!(row_status(&warning, false, false), "Complete with warning");
    assert_eq!(row_status(&none, false, false), "Not started");
}
