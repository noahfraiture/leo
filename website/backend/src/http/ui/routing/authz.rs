//! Authorization traits and reusable grant helpers for UI routes.

use async_trait::async_trait;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use thiserror::Error;

use crate::app::AppState;

/// Request metadata available to typed authorization rules before the handler
/// starts. This is intentionally small; route-specific data should come from
/// the typed page input instead.
pub struct AuthzRequest {
    headers: HeaderMap,
}

impl AuthzRequest {
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub(super) fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            headers: headers.clone(),
        }
    }
}

/// Typed pre-handler authorization rule for a page.
///
/// The rule can inspect request metadata and the already-extracted page input,
/// then either reject the request or return the authorized value that the page
/// handler will receive.
#[async_trait]
pub trait Authz<I>: Send + Sync + 'static {
    type Granted: Send;

    async fn authorize(
        state: &AppState,
        request: &AuthzRequest,
        input: &I,
    ) -> Result<Self::Granted, AuthzError>;
}

/// Reuse a granted authz value from a parent route when embedding a child
/// route without rerunning authz.
///
/// This is intentionally a pure projection from a parent granted value into the
/// child granted value. Use it when the parent route already holds an
/// authorization result that safely implies the child route's granted value.
///
/// Simple generic cases are covered by blanket impls:
/// - `T -> T`
/// - `T -> Option<T>`
///
/// More exotic reuse is also possible by projecting a richer parent grant into
/// a narrower child grant. For example, a parent
/// `PortfolioAdminGrant { user, portfolio_id, permissions }` could be reused as
/// a child `PortfolioViewerGrant { user, portfolio_id }` without redoing the
/// same access check.
///
/// If the child authz depends on fresh request input or other checks that
/// cannot be derived from the parent granted value alone, do not implement
/// `ReuseGranted`; rerun normal authz instead.
pub trait ReuseGranted<Parent>: Sized {
    fn reuse_from(parent: &Parent) -> Self;
}

impl<T: Clone> ReuseGranted<T> for T {
    fn reuse_from(parent: &T) -> Self {
        parent.clone()
    }
}

impl<T: Clone> ReuseGranted<T> for Option<T> {
    fn reuse_from(parent: &T) -> Self {
        Some(parent.clone())
    }
}

#[derive(Debug, Error)]
pub enum AuthzError {
    #[error(transparent)]
    Db(#[from] surrealdb::Error),
    #[error("{0}")]
    Forbidden(&'static str),
}

impl IntoResponse for AuthzError {
    fn into_response(self) -> Response {
        match self {
            Self::Db(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("database authorization failure: {error}"),
            )
                .into_response(),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message).into_response(),
        }
    }
}

/// Public routes do not need authenticated context.
pub struct Public;

#[async_trait]
impl<I: Sync> Authz<I> for Public {
    type Granted = ();

    async fn authorize(
        _state: &AppState,
        _request: &AuthzRequest,
        _input: &I,
    ) -> Result<Self::Granted, AuthzError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ReuseGranted;

    #[test]
    fn reuse_granted_clones_identical_type() {
        let granted = String::from("user-123");

        let reused = <String as ReuseGranted<String>>::reuse_from(&granted);

        assert_eq!(reused, "user-123");
    }

    #[test]
    fn reuse_granted_wraps_optional_type() {
        let granted = String::from("user-123");

        let reused = <Option<String> as ReuseGranted<String>>::reuse_from(&granted);

        assert_eq!(reused, Some(String::from("user-123")));
    }

    struct PortfolioAdminGrant {
        user_id: String,
        portfolio_id: String,
        permissions: Vec<String>,
    }

    #[derive(Debug, PartialEq)]
    struct PortfolioViewerGrant {
        user_id: String,
        portfolio_id: String,
    }

    impl ReuseGranted<PortfolioAdminGrant> for PortfolioViewerGrant {
        fn reuse_from(parent: &PortfolioAdminGrant) -> Self {
            Self {
                user_id: parent.user_id.clone(),
                portfolio_id: parent.portfolio_id.clone(),
            }
        }
    }

    #[test]
    fn reuse_granted_can_project_richer_parent_into_narrower_child() {
        let parent = PortfolioAdminGrant {
            user_id: String::from("user-123"),
            portfolio_id: String::from("portfolio-42"),
            permissions: vec![String::from("read"), String::from("write")],
        };

        let reused =
            <PortfolioViewerGrant as ReuseGranted<PortfolioAdminGrant>>::reuse_from(&parent);

        assert_eq!(
            reused,
            PortfolioViewerGrant {
                user_id: String::from("user-123"),
                portfolio_id: String::from("portfolio-42"),
            }
        );
        assert_eq!(parent.permissions.len(), 2);
    }
}
