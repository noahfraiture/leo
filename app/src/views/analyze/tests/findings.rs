use super::assessment::reader_disagreement;
use super::citations::citation_labels;
use crate::analysis::test_snapshot;
use backend::analysis::{Judgment, RunStage};
use uuid::Uuid;

#[test]
fn candidate_citations_follow_recording_time_then_camera_without_repeated_or_unknown_ids() {
    let mut snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Ready);
    let template = snapshot.checkpoint.evidence.frames[0].clone();
    snapshot.checkpoint.evidence.frames = [(2, 13_000), (1, 40_000), (2, 4_000), (1, 13_000)]
        .into_iter()
        .map(|(camera_id, session_offset_ms)| {
            let mut frame = template.clone();
            frame.id = format!("C{camera_id}-T{session_offset_ms}");
            frame.camera_id = camera_id;
            frame.session_offset_ms = session_offset_ms;
            frame
        })
        .collect();
    let mut ids: Vec<_> = snapshot
        .checkpoint
        .evidence
        .frames
        .iter()
        .map(|frame| frame.id.clone())
        .collect();
    ids.extend(["C2-T13000".into(), "unknown-candidate".into()]);

    assert_eq!(
        citation_labels(&snapshot, &ids)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        ["C2-T4000", "C1-T13000", "C2-T13000", "C1-T40000"]
    );
}

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
