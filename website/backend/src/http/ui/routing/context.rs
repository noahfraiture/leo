//! Per-request context passed to typed UI route handlers.

use axum::http::request::Parts;

use crate::app::AppState;

use super::AuthzRequest;

/// Request-scoped context shared by mounted routes and embedded route
/// composition.
///
/// This carries the request data that route execution can safely reuse across
/// composition boundaries:
/// - `state`: application services and configuration
/// - `authz_request`: request metadata used by authz rules
/// - `request_parts`: cloned request head data for parts-based child extractors
///
/// It does not preserve the request body. Embedded routes that need to follow
/// the current request can only re-extract from request parts, not replay
/// body-consuming extractors.
pub struct RouteContext {
    state: AppState,
    authz_request: AuthzRequest,
    request_parts: Parts,
}

impl RouteContext {
    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub(super) fn authz_request(&self) -> &AuthzRequest {
        &self.authz_request
    }

    pub(super) fn request_parts(&self) -> &Parts {
        &self.request_parts
    }

    pub(super) fn new(state: AppState, authz_request: AuthzRequest, request_parts: Parts) -> Self {
        Self {
            state,
            authz_request,
            request_parts,
        }
    }
}
