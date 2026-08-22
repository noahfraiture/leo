#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("recording I/O failed")]
    Io(#[from] std::io::Error),

    #[error("recording discovery requires at least one camera")]
    EmptyCameraList,

    #[error("recording camera ID must be non-zero")]
    ZeroCameraId,

    #[error("duplicate recording camera {camera_id}")]
    DuplicateCamera { camera_id: u32 },

    #[error("recordings root must be a direct directory")]
    InvalidRecordingsRoot,

    #[error("recording camera {camera_id} directory must be a direct directory")]
    InvalidCameraDirectory { camera_id: u32 },

    #[error("recording camera {camera_id} contains a symbolic link")]
    InvalidSegmentEntry { camera_id: u32 },

    #[error("FFprobe rejected recording media")]
    InvalidMedia,

    #[error("recording media has an invalid duration")]
    InvalidMediaDuration,

    #[error("FFprobe returned malformed JSON")]
    ProbeJson(#[source] serde_json::Error),

    #[error("FFprobe timed out")]
    ProbeTimeout,

    #[error("recording operation was shut down")]
    Shutdown,

    #[error("recorder timeout cannot be represented as positive FFmpeg microseconds")]
    InvalidRecorderTimeout,

    #[error("recorder executable preflight failed")]
    RecorderPreflightFailed,

    #[error("recorder executable preflight timed out")]
    RecorderPreflightTimeout,

    #[error("recording camera {camera_id} must use a valid RTSP URL")]
    InvalidCameraUrl { camera_id: u32 },

    #[error("recorder command channel closed")]
    RecorderCommandClosed,

    #[error("recorder command reply was dropped")]
    RecorderReplyDropped,

    #[error("recorder management thread failed")]
    RecorderThread,

    #[error("recorder startup failed")]
    RecorderStartupFailed,

    #[error("recorder startup cleanup was uncertain")]
    RecorderStartupCleanupFailed,

    #[error("recorder process cleanup failed")]
    RecorderCleanupFailed {
        #[source]
        source: Box<Error>,
    },

    #[error("recorder startup timed out")]
    RecorderStartupTimeout,

    #[error("a recording session is already active")]
    RecorderAlreadyActive,

    #[error("no recording session is active")]
    RecorderNotActive,

    #[error("FFmpeg recorder pipes were unavailable")]
    FfmpegPipes,

    #[error("FFmpeg recorder progress was invalid")]
    InvalidFfmpegProgress,

    #[error("FFmpeg recorder parser failed")]
    FfmpegParser,

    #[error("FFmpeg recorder parser thread failed")]
    FfmpegPump,

    #[error("FFmpeg recorder could not be stopped gracefully")]
    FfmpegQuit,

    #[error("FFmpeg recorder event receiver closed")]
    RecorderEventReceiverClosed,

    #[error("FFmpeg recorder output was not a direct regular file")]
    InvalidAttemptOutput,

    #[error("recording timestamp overflowed")]
    TimestampOverflow,

    #[error("recording camera {camera_id} contains overlapping segments")]
    OverlappingSegments { camera_id: u32 },
}

pub type Result<T> = std::result::Result<T, Error>;
