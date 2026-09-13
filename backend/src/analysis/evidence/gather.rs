use super::{CandidateFrame, EvidenceError, EvidenceInterval, EvidenceManifest};
use std::collections::{BTreeMap, HashSet};

/// Adds new samples inside recorded participation periods and actual segment coverage.
/// Each requested interval is at most 60 seconds; one invocation adds at most 128 samples.
/// The caller records a single revisit round per criterion and must not treat unsampled time as failure.
pub fn gather_evidence(
    manifest: &mut EvidenceManifest,
    intervals: &[EvidenceInterval],
) -> Result<Vec<String>, EvidenceError> {
    gather_evidence_with_budget(manifest, intervals, 128)
}

/// Distributes a bounded number of new samples across the full requested time span.
/// Reducing a budget changes sample density instead of discarding the end of an interval.
pub fn gather_evidence_with_budget(
    manifest: &mut EvidenceManifest,
    intervals: &[EvidenceInterval],
    max_new_frames: usize,
) -> Result<Vec<String>, EvidenceError> {
    if max_new_frames == 0 || max_new_frames > 128 {
        return Err(EvidenceError::Invalid(
            "Additional evidence budget must be between 1 and 128 frames".into(),
        ));
    }
    let mut new = BTreeMap::new();
    let existing = manifest
        .frames
        .iter()
        .map(|f| f.id.clone())
        .collect::<HashSet<_>>();
    for interval in intervals {
        if interval.start_ms >= interval.end_ms
            || interval.end_ms > manifest.duration_ms
            || interval.end_ms - interval.start_ms > 60_000
        {
            return Err(EvidenceError::Invalid(
                "Additional evidence intervals must be inside the session and at most 60 seconds"
                    .into(),
            ));
        }
        let periods = manifest
            .eligible_periods
            .iter()
            .filter(|p| {
                p.camera_id == interval.camera_id
                    && p.start_ms < interval.end_ms
                    && p.end_ms > interval.start_ms
            })
            .collect::<Vec<_>>();
        if periods.is_empty() {
            return Err(EvidenceError::Invalid(
                "Camera was not eligible during requested interval".into(),
            ));
        }
        for period in periods {
            let start = interval.start_ms.max(period.start_ms);
            let end = interval.end_ms.min(period.end_ms);
            // Uniformly cover the requested span when dense extraction would exceed the revisit budget.
            let step = (period.sample_every_ms / 4)
                .max(100)
                .max((end - start).div_ceil(128));
            let mut offset = start;
            while offset < end {
                let id = format!("C{}-T{}", interval.camera_id, offset);
                if !existing.contains(&id) {
                    let utc = manifest
                        .session_start_utc_ms
                        .checked_add(
                            i64::try_from(offset)
                                .map_err(|_| EvidenceError::Invalid("Timeline overflow".into()))?,
                        )
                        .ok_or_else(|| EvidenceError::Invalid("Timeline overflow".into()))?;
                    if let Some((index, source)) =
                        manifest.sources.iter().enumerate().find(|(_, s)| {
                            s.camera_id == interval.camera_id
                                && s.start_utc_ms <= utc
                                && utc < s.end_utc_ms
                        })
                    {
                        new.insert(
                            (offset, interval.camera_id),
                            CandidateFrame {
                                id,
                                camera_id: interval.camera_id,
                                source_index: index,
                                session_offset_ms: offset,
                                recording_offset_ms: (utc - source.start_utc_ms) as u64,
                            },
                        );
                    }
                }
                offset = offset.saturating_add(step);
            }
        }
    }
    let new: Vec<_> = new.into_values().collect();
    let selected = if new.len() > max_new_frames {
        (0..max_new_frames)
            .map(|index| {
                let offset = if max_new_frames == 1 {
                    new.len() / 2
                } else {
                    index * (new.len() - 1) / (max_new_frames - 1)
                };
                new[offset].clone()
            })
            .collect::<Vec<_>>()
    } else {
        new
    };
    let ids = selected.iter().map(|f| f.id.clone()).collect();
    // New frames are available by citation but do not alter the original reader coverage plan.
    manifest.frames.extend(selected);
    manifest
        .frames
        .sort_by_key(|f| (f.session_offset_ms, f.camera_id));
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{EligiblePeriod, RecordingSource};
    #[test]
    fn new_samples_respect_participation_recording_gaps_and_do_not_rewrite_reader_windows() {
        let mut m = EvidenceManifest {
            session_id: uuid::Uuid::new_v4(),
            session_start_utc_ms: 1000,
            duration_ms: 5000,
            sources: vec![RecordingSource {
                relative_path: "clip.mkv".into(),
                camera_id: 1,
                start_utc_ms: 2000,
                end_utc_ms: 3000,
                size_bytes: 1,
                sha256: "hash".into(),
            }],
            frames: vec![],
            frame_sets: vec![],
            warnings: vec![],
            eligible_periods: vec![EligiblePeriod {
                camera_id: 1,
                start_ms: 1500,
                end_ms: 4000,
                sample_every_ms: 1000,
            }],
        };
        let ids = gather_evidence(
            &mut m,
            &[EvidenceInterval {
                camera_id: 1,
                start_ms: 0,
                end_ms: 5000,
            }],
        )
        .unwrap();
        assert_eq!(ids, vec!["C1-T1500", "C1-T1750"]);
        assert!(m.frame_sets.is_empty());
        assert!(
            gather_evidence(
                &mut m,
                &[EvidenceInterval {
                    camera_id: 2,
                    start_ms: 0,
                    end_ms: 1000
                }]
            )
            .is_err()
        );
        assert!(
            gather_evidence(
                &mut m,
                &[EvidenceInterval {
                    camera_id: 1,
                    start_ms: 0,
                    end_ms: 5000
                }]
            )
            .unwrap()
            .is_empty()
        );
    }
}
