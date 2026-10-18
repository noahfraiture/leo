use super::*;
use uuid::Uuid;

fn manifest() -> EvidenceManifest {
    let mut frames = Vec::new();
    let mut frame_sets = Vec::new();
    for time in [0, 1000, 2000, 5000] {
        let mut set = Vec::new();
        for camera in [1, 2] {
            let id = format!("C{camera}-T{time}");
            set.push(id.clone());
            frames.push(CandidateFrame {
                id,
                camera_id: camera,
                source_index: 0,
                session_offset_ms: time,
                recording_offset_ms: time,
            });
        }
        frame_sets.push(set);
    }
    EvidenceManifest {
        session_id: Uuid::new_v4(),
        session_start_utc_ms: 0,
        duration_ms: 6000,
        sources: vec![],
        frames,
        frame_sets,
        warnings: vec![],
        eligible_periods: vec![],
    }
}

#[test]
fn reference_budget_keeps_camera_sets_complete_and_full_coverage() {
    let m = manifest();
    let windows = plan_windows(&m, 6, 2, 2000, 1).unwrap();
    assert_eq!(windows.len(), 3);
    assert_eq!(windows[0].frame_ids, m.frame_sets[..2].concat());
    assert_eq!(windows[1].frame_ids, m.frame_sets[1..3].concat());
    assert_eq!(windows[2].frame_ids, m.frame_sets[3].clone());
    let covered = windows
        .iter()
        .flat_map(|w| &w.frame_ids)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(covered.len(), m.frames.len());
    for window in &windows {
        assert!(window.frame_ids.len() + 2 <= 6);
        assert!(window.end_ms - window.start_ms <= 2000);
    }
}

#[test]
fn budget_cannot_drop_references_or_split_a_camera_set() {
    assert!(plan_windows(&manifest(), 3, 2, 5000, 0).is_err());
    assert!(plan_windows(&manifest(), 2, 2, 5000, 0).is_err());
    assert!(plan_windows(&manifest(), 4, 2, 5000, 1).is_err());
}

#[test]
fn citation_resolution_does_not_accept_reference_or_unknown_ids() {
    let m = manifest();
    assert!(m.frame("R1").is_none());
    assert!(m.frame("C1-T3000").is_none());
    assert_eq!(m.frame("C2-T5000").unwrap().camera_id, 2);
}

#[test]
fn changed_source_cannot_resume() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("source.mkv");
    std::fs::write(&file, b"original").unwrap();
    let mut m = manifest();
    m.sources.push(RecordingSource {
        relative_path: "source.mkv".into(),
        camera_id: 1,
        start_utc_ms: 0,
        end_utc_ms: 6000,
        size_bytes: 8,
        sha256: super::materialize::file_hash(&file).unwrap(),
    });
    verify_sources(dir.path(), &m).unwrap();
    std::fs::write(file, b"modified").unwrap();
    assert!(verify_sources(dir.path(), &m).is_err());
}

#[test]
fn materialization_rejects_inconsistent_session_offsets_before_opening_media() {
    let mut evidence = manifest();
    evidence.sources.push(RecordingSource {
        relative_path: "missing.mkv".into(),
        camera_id: 1,
        start_utc_ms: 0,
        end_utc_ms: 6000,
        size_bytes: 1,
        sha256: "hash".into(),
    });
    evidence.frames[0].session_offset_ms = 1000;
    let error = materialize_frame(
        std::path::Path::new("/missing"),
        &evidence,
        &evidence.frames[0],
        crate::profiles::ImageSizePolicy::Original,
    )
    .unwrap_err();
    assert!(matches!(error, EvidenceError::Invalid(_)), "{error}");
}

#[test]
fn candidate_sources_reject_invalid_or_overflowing_segment_times() {
    let mut evidence = manifest();
    evidence.sources.push(RecordingSource {
        relative_path: "source.mkv".into(),
        camera_id: 1,
        start_utc_ms: 0,
        end_utc_ms: 6000,
        size_bytes: 1,
        sha256: "hash".into(),
    });
    assert!(evidence.source_for(&evidence.frames[0]).is_ok());
    for (start, end) in [(0, 0), (1000, 0), (i64::MIN, i64::MAX)] {
        evidence.sources[0].start_utc_ms = start;
        evidence.sources[0].end_utc_ms = end;
        assert!(matches!(
            evidence.source_for(&evidence.frames[0]),
            Err(EvidenceError::Invalid(_))
        ));
    }
}
