use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use thiserror::Error;

use super::AuthzError;

#[derive(Debug, Error)]
pub enum RouteError {
    #[error(transparent)]
    Authz(#[from] AuthzError),
    #[error(transparent)]
    Db(#[from] surrealdb::Error),
    #[error("{0}")]
    BadRequest(&'static str),
    #[error("failed to extract embedded route input for {route}: {message}")]
    EmbeddedInput {
        route: &'static str,
        message: String,
    },
    #[error("{0}")]
    Forbidden(&'static str),
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        match self {
            Self::Authz(error) => error.into_response(),
            Self::Db(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("database route failure: {error}"),
            )
                .into_response(),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message).into_response(),
            Self::EmbeddedInput { .. } => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()
            }
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message).into_response(),
        }
    }
}
