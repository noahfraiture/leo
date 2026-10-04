use super::citations::citation_labels;
use crate::analysis::test_snapshot;
use backend::analysis::RunStage;
use uuid::Uuid;

#[test]
fn resolved_uncertainty_still_needs_attention_when_the_requirement_is_not_met() {
    use super::assessment::ResultFilter;
    use backend::analysis::Judgment;
    let mut snapshot = test_snapshot(Uuid::new_v4(), Uuid::new_v4(), RunStage::Ready);
    let assessment = &mut snapshot.checkpoint.assessments[0];
    assessment.initial.judgment = Judgment::CannotVerify;
    assessment.judgment = Judgment::NotMet;
    let id = assessment.criterion_id;
    assert!(ResultFilter::Resolved.includes(&snapshot, id));
    assert!(ResultFilter::NeedsAttention.includes(&snapshot, id));
}

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
