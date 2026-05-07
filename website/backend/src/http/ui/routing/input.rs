use axum::{
    extract::{FromRequest, FromRequestParts, Request},
    http::request::Parts,
};

/// Marker extractor for routes that do not need request input.
pub struct NoInput;

impl<S> FromRequest<S> for NoInput
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request(_request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}

impl<S> FromRequestParts<S> for NoInput
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}
