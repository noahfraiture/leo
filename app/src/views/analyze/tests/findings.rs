use super::assessment::reader_disagreement;
use crate::analysis::test_snapshot;
use backend::analysis::{Judgment, RunStage};
use uuid::Uuid;

#[test]
fn disagreement_requires_distinct_readers_comparing_matching_evidence() {
    let mut snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Reading);
    let criterion_id = snapshot.inputs.exercise.checklist.criteria[0].id;
    let mut job = snapshot.checkpoint.jobs[0].clone();
    job.id = "second-reader-job".into();
    job.profile_id = "reader-two".into();
    snapshot.checkpoint.jobs.push(job);
    let mut result = snapshot.reader_results[0].clone();
    result.job_id = "second-reader-job".into();
    result.completion.value.judgment = Judgment::Met;
    snapshot.reader_results.push(result);

    assert!(reader_disagreement(&snapshot, criterion_id));
    snapshot.checkpoint.jobs[1].frame_ids = vec!["different-window".into()];
    assert!(!reader_disagreement(&snapshot, criterion_id));
    snapshot.checkpoint.jobs[1].frame_ids = snapshot.checkpoint.jobs[0].frame_ids.clone();
    snapshot.checkpoint.jobs[1].profile_id = snapshot.checkpoint.jobs[0].profile_id.clone();
    assert!(!reader_disagreement(&snapshot, criterion_id));
}
