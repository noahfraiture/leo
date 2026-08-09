use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Synology HTTP request failed")]
    Http(#[source] reqwest::Error),

    #[error("Synology API returned error code {code}")]
    Api { code: u32 },

    #[error("Synology API response is missing {field}")]
    MissingResponseField { field: &'static str },

    #[error("Synology HTTP request returned {status}")]
    HttpStatus { status: reqwest::StatusCode },

    #[error("recording destination I/O failed")]
    Io(#[from] std::io::Error),

    #[error("Synology download returned an unexpected successful JSON response")]
    UnexpectedJsonDownload,

    #[error("invalid catalogue UTC millisecond bounds {from_utc_ms}..{to_utc_ms}")]
    InvalidListRange { from_utc_ms: i64, to_utc_ms: i64 },

    #[error("recording {recording_id} has invalid UTC second bounds {start_time}..{stop_time}")]
    InvalidRecordingRange {
        recording_id: u64,
        start_time: i64,
        stop_time: i64,
    },

    #[error(
        "recording {recording_id} UTC timestamp {utc_seconds} cannot be stored as milliseconds"
    )]
    RecordingTimestampOverflow { recording_id: u64, utc_seconds: i64 },

    #[error("Synology catalogue ended after {loaded} of {total} recordings")]
    IncompletePagination { loaded: usize, total: usize },

    #[error("recording {recording_id} has invalid download range {start:?}..{end:?}")]
    InvalidDownloadRange {
        recording_id: u64,
        start: Duration,
        end: Duration,
    },
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
