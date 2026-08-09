use axum::{
    Json,
    extract::rejection::QueryRejection,
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// Synology error codes returned in HTTP-200 JSON envelopes.
pub(super) enum ApiError {
    Unknown = 100,
    InvalidParameters = 101,
    UnknownApi = 102,
    UnknownMethod = 103,
    UnsupportedVersion = 104,
    ExecutionFailed = 400,
    InvalidRecordingParameters = 401,
    UnknownRecording = 414,
}

/// Standard failed Synology JSON envelope.
#[derive(Serialize)]
struct Failure {
    success: bool,
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: u16,
}

impl From<QueryRejection> for ApiError {
    fn from(_: QueryRejection) -> Self {
        Self::InvalidParameters
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        Json(Failure {
            success: false,
            error: ErrorBody { code: self as u16 },
        })
        .into_response()
    }
}
