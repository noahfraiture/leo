use std::{io, process::ExitStatus, time::Duration};

#[derive(Debug, thiserror::Error)]
pub(in crate::analysis) enum Error {
    #[error("camera {camera_id} is not part of the session")]
    UnknownCamera { camera_id: u32 },

    #[error("camera {camera_id} has a zero sampling interval")]
    InvalidSamplingInterval { camera_id: u32 },

    #[error("camera {camera_id} has an action at {offset:?} after session end {session_end:?}")]
    ActionAfterSessionEnd {
        camera_id: u32,
        offset: Duration,
        session_end: Duration,
    },

    #[error(
        "camera {camera_id} has an invalid sampling period {start:?}..{end:?} with interval {sample_every:?}"
    )]
    InvalidSamplingPeriod {
        camera_id: u32,
        start: Duration,
        end: Duration,
        sample_every: Duration,
    },

    #[error(
        "camera {camera_id} has overlapping or unordered sampling periods ending at {previous_end:?} and starting at {start:?}"
    )]
    UnorderedSamplingPeriods {
        camera_id: u32,
        previous_end: Duration,
        start: Duration,
    },

    #[error(
        "camera {camera_id} sample at {session_offset:?} is outside the supported UTC timestamp range"
    )]
    UtcTimestampOverflow {
        camera_id: u32,
        session_offset: Duration,
    },

    #[error("camera {camera_id} sample at {session_offset:?} has no recording coverage")]
    MissingRecording {
        camera_id: u32,
        session_offset: Duration,
    },

    #[error("camera {camera_id} sample at {session_offset:?} matches multiple recordings")]
    OverlappingRecordings {
        camera_id: u32,
        session_offset: Duration,
    },

    #[error("camera {camera_id} sample sequence is not ordered: {previous:?} precedes {current:?}")]
    UnorderedSequence {
        camera_id: u32,
        previous: Duration,
        current: Duration,
    },

    #[error("duplicate camera {camera_id} frame at {session_offset:?}")]
    DuplicateCameraFrame {
        camera_id: u32,
        session_offset: Duration,
    },

    #[error("failed to create temporary frame directory")]
    CreateFrameTempDir {
        #[source]
        source: io::Error,
    },

    #[error("failed to start FFmpeg")]
    FfmpegSpawn {
        #[source]
        source: io::Error,
    },

    #[error("failed to wait for FFmpeg")]
    FfmpegWait {
        #[source]
        source: io::Error,
    },

    #[error("FFmpeg exited unsuccessfully with {status}")]
    FfmpegExit { status: ExitStatus },

    #[error("FFmpeg exited successfully without producing a frame")]
    MissingFrameOutput,

    #[error("failed to read the extracted frame")]
    ReadFrameOutput {
        #[source]
        source: io::Error,
    },

    #[error("FFmpeg output is not a JPEG")]
    InvalidJpeg,
}

pub(super) type Result<T> = std::result::Result<T, Error>;
