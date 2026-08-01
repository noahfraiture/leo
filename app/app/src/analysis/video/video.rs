use std::time::{Duration, Instant};

use crate::preview::CameraSource;

struct Video {
    frame_rate: u32,
    sampling_schedule: SamplingSchedule,
}

impl Video {
    fn sample(&self) -> SampleSequence {
        todo!()
    }
}

struct Frame {
    timestamp: Instant,
    offset: Duration,
    // we should not have a dependency on preview. Should we extract the camera out of preview ?
    // but maybe we should just not use the same struct ? Because we are not at preview time but analysis time which is different ?
    camera: CameraSource,
    comment: Option<String>,
}

/// The sampling rate change define the new sampling rate to apply starting from
/// a given offset of the video.
struct SamplingRateChange {
    rate: u32,
    offset: Duration,
}

/// The sampling schedule of a video is the configuration of sampling to respect.
/// The sampling rate of a video might change over time.
///
/// The changes are ordered by offset.
struct SamplingSchedule {
    changes: Vec<SamplingRateChange>,
}

/// The sample sequence is the sequence of frames extracted from a video
/// respecting its sampling schedule.
///
/// The frames are ordered by timestamp.
struct SampleSequence {
    sequence: Vec<Frame>,
}

// question : how to be sure the videos are all synced on an instant ? Still better than an offset i think

/// A frame set is a group of frame from multiples sources synced at the same
/// timestamp.
///
/// Two videos might have only some frames in common set due to different
/// sampling rate.
pub struct FrameSet {
    timestamp: Instant,
    frames: Vec<Frame>,
}

impl FrameSet {
    fn from_sequences(sequences: Vec<SampleSequence>) -> Vec<FrameSet> {
        // Vector of peekable iterators on frames
        let mut iterators = sequences
            .into_iter()
            .map(|sequence| sequence.sequence.into_iter().peekable())
            .collect::<Vec<_>>();

        let mut frame_sets = Vec::new();
        loop {
            let Some(timestamp) = iterators
                .iter_mut()
                .filter_map(|frames| frames.peek().map(|frame| frame.timestamp))
                .min()
            else {
                return frame_sets;
            };

            let mut frame_set = FrameSet {
                timestamp: timestamp,
                frames: Vec::new(),
            };
            frame_set.frames = iterators
                .iter_mut()
                .filter_map(|it| {
                    it.peek()
                        .is_some_and(|frame| frame.timestamp == timestamp)
                        .then(|| it.next().unwrap())
                })
                .collect();

            frame_sets.push(frame_set);
        }
    }
}
