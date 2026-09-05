use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Profile(#[from] crate::profiles::Error),

    #[error("session event I/O failed")]
    Io(#[from] std::io::Error),

    #[error("session event on line {line} is not valid JSON")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error("session event log is missing its final newline")]
    MissingFinalNewline,

    #[error("session event log must be a direct regular file")]
    InvalidEventFile,

    #[error("session directory must be a direct directory")]
    InvalidSessionDirectory,

    #[error("failed to serialize a session event")]
    Serialize(#[source] serde_json::Error),

    #[error("unsupported session event schema version {version}")]
    UnsupportedSchema { version: u8 },

    #[error("expected sequence {expected}, found {actual}")]
    NonContiguousSequence { expected: u64, actual: u64 },

    #[error("session start offset must be zero, found {actual}")]
    NonZeroSessionStartOffset { actual: u64 },

    #[error(
        "session offsets must be nondecreasing: previous offset {previous}, found {actual} at sequence {sequence}"
    )]
    DecreasingSessionOffset {
        sequence: u64,
        previous: u64,
        actual: u64,
    },

    #[error("session ID mismatch: expected {expected}, found {actual}")]
    SessionIdMismatch { expected: Uuid, actual: Uuid },

    #[error("session event log is missing its session start")]
    MissingSessionStart,

    #[error("session event sequence {sequence} contains another session start")]
    DuplicateSessionStart { sequence: u64 },

    #[error("duplicate camera {camera_id} in session start")]
    DuplicateCamera { camera_id: u32 },

    #[error("a session requires at least one camera")]
    EmptyCameraList,

    #[error("camera ID must be non-zero")]
    ZeroCameraId,

    #[error("camera {camera_id} has an invalid sampling interval")]
    InvalidSamplingInterval { camera_id: u32 },

    #[error("unknown camera {camera_id} in session action")]
    UnknownCamera { camera_id: u32 },

    #[error("cannot apply an action because the session has ended")]
    SessionEnded,

    #[error("session event log contains an action after session end")]
    ActionAfterSessionEnd,

    #[error("session event log contains more than one session end")]
    DuplicateSessionEnd,

    #[error("session event log is missing session end")]
    MissingSessionEnd,

    #[error("system time is before the Unix epoch")]
    SystemTime(#[from] std::time::SystemTimeError),

    #[error("UTC timestamp is outside the persisted millisecond range")]
    UtcTimestampOverflow,

    #[error("session offset is outside the persisted millisecond range")]
    SessionOffsetOverflow,

    #[error("session event sequence overflowed")]
    SequenceOverflow,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Whether a metadata operation may have left the event stream incomplete.
    pub fn is_write_failure(&self) -> bool {
        matches!(
            self,
            Self::Io(_)
                | Self::Serialize(_)
                | Self::SystemTime(_)
                | Self::UtcTimestampOverflow
                | Self::SessionOffsetOverflow
                | Self::SequenceOverflow
        )
    }
}
