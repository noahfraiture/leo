use super::super::video::{FrameSet, SampleSequence, SamplingSchedule, recording_gap_warnings};
use super::materialize::file_hash;
use super::{
    CandidateFrame, EligiblePeriod, EvidenceError, EvidenceManifest, EvidenceWindow,
    RecordingSource,
};
use crate::{recording::RecordingSegment, session::Session};
use std::{collections::HashMap, fs, path::Path};

/// Freezes the recorded participation/cadence and source hashes without changing sampling policy.
/// This performs disk I/O and must run off the UI thread.
pub fn build_evidence(
    directory: &Path,
    session: &Session,
    segments: &[RecordingSegment],
) -> Result<EvidenceManifest, EvidenceError> {
    let mut sources = Vec::new();
    let mut by_path = HashMap::new();
    for segment in segments {
        let relative_path = segment
            .path
            .strip_prefix(directory)
            .map_err(|_| EvidenceError::Invalid("Recording is outside the session".into()))?
            .to_path_buf();
        let metadata = fs::metadata(&segment.path)?;
        by_path.insert(segment.path.clone(), sources.len());
        sources.push(RecordingSource {
            relative_path,
            camera_id: segment.camera_id,
            start_utc_ms: segment.start_utc_ms,
            end_utc_ms: segment.end_utc_ms,
            size_bytes: metadata.len(),
            sha256: file_hash(&segment.path)?,
        });
    }
    let mut eligible_periods = Vec::new();
    let sequences = session
        .cameras
        .iter()
        .map(|camera| {
            let schedule = SamplingSchedule::from_session(session, camera.id)?;
            eligible_periods.extend(schedule.periods.iter().map(|p| EligiblePeriod {
                camera_id: camera.id,
                start_ms: p.start.as_millis() as u64,
                end_ms: p.end.as_millis() as u64,
                sample_every_ms: p.sample_every.as_millis() as u64,
            }));
            SampleSequence::from_segments(session.start_utc_ms, &schedule, segments)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sets = FrameSet::from_sequences(sequences)?;
    let mut frames = Vec::new();
    let mut frame_sets = Vec::new();
    for set in sets {
        let mut ids = Vec::new();
        for frame in set.frames {
            let offset = u64::try_from(frame.session_offset.as_millis()).map_err(|_| {
                EvidenceError::Invalid("Timeline exceeds supported duration".into())
            })?;
            let id = format!("C{}-T{}", frame.camera_id, offset);
            ids.push(id.clone());
            frames.push(CandidateFrame {
                id,
                camera_id: frame.camera_id,
                source_index: *by_path
                    .get(&frame.path)
                    .ok_or_else(|| EvidenceError::Invalid("Missing segment".into()))?,
                session_offset_ms: offset,
                recording_offset_ms: frame.recording_offset.as_millis() as u64,
            });
        }
        frame_sets.push(ids);
    }
    if frames.is_empty() {
        return Err(EvidenceError::Invalid(
            "No eligible recording frames".into(),
        ));
    }
    Ok(EvidenceManifest {
        session_id: session.id,
        session_start_utc_ms: session.start_utc_ms,
        duration_ms: session.end_offset.as_millis() as u64,
        sources,
        frames,
        frame_sets,
        warnings: recording_gap_warnings(session, segments)?,
        eligible_periods,
    })
}

/// Plans complete camera sets after reserving room for every required reference image.
/// All readers use the same resulting windows; invalid capacity is reported before inference.
pub fn plan_windows(
    manifest: &EvidenceManifest,
    image_limit: usize,
    reference_count: usize,
    max_span_ms: u64,
    overlap_sets: usize,
) -> Result<Vec<EvidenceWindow>, EvidenceError> {
    let capacity = image_limit
        .checked_sub(reference_count)
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            EvidenceError::Invalid("References leave no room for candidate evidence".into())
        })?;
    if manifest.frame_sets.is_empty() || max_span_ms == 0 {
        return Err(EvidenceError::Invalid(
            "Empty evidence or zero time span".into(),
        ));
    }
    let offsets = manifest
        .frames
        .iter()
        .map(|f| (f.id.as_str(), f.session_offset_ms))
        .collect::<HashMap<_, _>>();
    let start_at = |i: usize| -> Result<u64, EvidenceError> {
        manifest.frame_sets[i]
            .first()
            .and_then(|id| offsets.get(id.as_str()).copied())
            .ok_or_else(|| EvidenceError::Invalid("Invalid camera set".into()))
    };
    let mut windows = Vec::new();
    let mut start = 0;
    let mut processed = 0;
    while start < manifest.frame_sets.len() {
        let begin_ms = start_at(start)?;
        let mut end = start;
        let mut frame_ids = Vec::new();
        while end < manifest.frame_sets.len() {
            let set = &manifest.frame_sets[end];
            if set.len() > capacity {
                return Err(EvidenceError::Invalid(
                    "Required references and one complete camera set exceed the image limit".into(),
                ));
            }
            let end_ms = start_at(end)?;
            if end_ms < begin_ms {
                return Err(EvidenceError::Invalid(
                    "Camera sets are out of order".into(),
                ));
            }
            if frame_ids.len() + set.len() > capacity || end_ms - begin_ms > max_span_ms {
                break;
            }
            if set
                .iter()
                .any(|id| offsets.get(id.as_str()) != Some(&end_ms))
            {
                return Err(EvidenceError::Invalid(
                    "Camera set has inconsistent timestamps".into(),
                ));
            }
            frame_ids.extend(set.clone());
            end += 1;
        }
        if end <= processed {
            // Overlap cannot bridge a recording gap wider than the allowed prompt span.
            if end == processed
                && start < processed
                && start_at(processed)? - begin_ms > max_span_ms
            {
                start = processed;
                continue;
            }
            return Err(EvidenceError::Invalid(
                "Overlap prevents new evidence".into(),
            ));
        }
        windows.push(EvidenceWindow {
            frame_ids,
            start_ms: begin_ms,
            end_ms: start_at(end - 1)?,
        });
        if end == manifest.frame_sets.len() {
            break;
        }
        if end - start <= overlap_sets {
            return Err(EvidenceError::Invalid(
                "Overlap prevents new evidence".into(),
            ));
        }
        processed = end;
        start = end - overlap_sets;
    }
    Ok(windows)
}
