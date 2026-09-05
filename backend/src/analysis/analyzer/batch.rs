use std::ops::Range;

use crate::{
    analysis::{error::Error, video::FrameSet},
    profiles::AnalysisProfile,
};

/// Plans atomic frame-set ranges with explicit image, time, and overlap limits.
pub fn plan_batches(
    frame_sets: &[FrameSet],
    profile: &AnalysisProfile,
) -> Result<Vec<Range<usize>>, Error> {
    profile.validate()?;
    if frame_sets.is_empty() {
        return Err(Error::NoAnalyzableFrames);
    }
    if frame_sets
        .windows(2)
        .any(|pair| pair[0].session_offset >= pair[1].session_offset)
    {
        return Err(Error::UnorderedFrameSets);
    }
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut processed = 0;
    loop {
        let mut end = start;
        let mut images: usize = 0;
        while let Some(frame_set) = frame_sets.get(end) {
            if frame_set.frames.len() > profile.max_images_per_prompt {
                return Err(Error::OversizedFrameSet {
                    images: frame_set.frames.len(),
                    limit: profile.max_images_per_prompt,
                });
            }
            let span = frame_set
                .session_offset
                .checked_sub(frame_sets[start].session_offset)
                .ok_or(Error::UnorderedFrameSets)?;
            let next_images =
                images
                    .checked_add(frame_set.frames.len())
                    .ok_or(Error::PlanValueOverflow {
                        field: "image count",
                    })?;
            if next_images > profile.max_images_per_prompt
                || span.as_millis() > u128::from(profile.max_prompt_span_ms)
            {
                break;
            }
            images = next_images;
            end += 1;
        }
        if end <= processed {
            return Err(Error::InvalidBatchOverlap);
        }
        ranges.push(start..end);
        if end == frame_sets.len() {
            return Ok(ranges);
        }
        if end - start <= profile.overlap_frame_sets {
            return Err(Error::InvalidBatchOverlap);
        }
        processed = end;
        start = end - profile.overlap_frame_sets;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::video::Frame;
    use std::{path::PathBuf, time::Duration};

    fn frames(counts: &[usize]) -> Vec<FrameSet> {
        counts
            .iter()
            .enumerate()
            .map(|(index, &count)| {
                let offset = Duration::from_millis(index as u64 * 500);
                FrameSet {
                    session_offset: offset,
                    frames: (0..count)
                        .map(|camera| Frame {
                            camera_id: camera as u32 + 1,
                            segment_start_utc_ms: 0,
                            segment_end_utc_ms: 5000,
                            sample_index: index,
                            session_offset: offset,
                            recording_offset: offset,
                            path: PathBuf::from("fixture.mkv"),
                        })
                        .collect(),
                }
            })
            .collect()
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // Expected batches contain ranges.
    fn image_and_span_limits_preserve_camera_sets_and_exact_overlap() {
        for (counts, images, span, overlap, expected) in [
            (vec![2, 1, 2, 1], 3, 1000, 0, vec![0..2, 2..4]),
            (vec![2, 1, 2, 1], 3, 1000, 1, vec![0..2, 1..3, 2..4]),
            (vec![1, 1, 1], 10, 500, 0, vec![0..2, 2..3]),
            (vec![1, 1, 1], 10, 500, 1, vec![0..2, 1..3]),
            (vec![2, 2], 4, 1000, 1, vec![0..2]),
        ] {
            let mut profile = crate::tests::analysis_profile(images, overlap);
            profile.max_prompt_span_ms = span;
            assert_eq!(
                plan_batches(&frames(&counts), &profile).unwrap(),
                expected,
                "counts={counts:?}"
            );
        }
    }

    #[test]
    fn invalid_plans_fail_before_any_provider_or_extraction_work() {
        assert!(matches!(
            plan_batches(&[], &crate::tests::analysis_profile(3, 0)),
            Err(Error::NoAnalyzableFrames)
        ));
        assert!(matches!(
            plan_batches(&frames(&[4]), &crate::tests::analysis_profile(3, 0)),
            Err(Error::OversizedFrameSet { .. })
        ));
        for counts in [vec![2, 1, 2], vec![1, 2, 3]] {
            let overlap = if counts[0] == 2 { 2 } else { 1 };
            assert!(matches!(
                plan_batches(
                    &frames(&counts),
                    &crate::tests::analysis_profile(3, overlap)
                ),
                Err(Error::InvalidBatchOverlap)
            ));
        }
    }
}
