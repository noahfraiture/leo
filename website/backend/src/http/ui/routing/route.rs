//! Core typed route and route view traits.

use async_trait::async_trait;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use hypertext::prelude::*;

use crate::app::AppState;

use super::{Authz, RouteContext, RouteError};

/// Rendering contract for a UI route result.
///
/// A single route view must be able to render:
/// - the full document returned for a normal browser navigation
/// - the fragment returned for an HTMX request
///
/// The view renders from `&self` so parent views can embed child route views
/// lazily and defer HTML generation until the outer response boundary.
pub trait RouteView {
    /// Render the full HTML payload for a normal browser navigation.
    ///
    /// This should include the route's complete page body for non-HTMX
    /// requests. Parent layouts that want to wrap child content should do that
    /// here, after any child fragments have already been resolved during
    /// handling.
    fn document(&self, state: &AppState) -> impl Renderable;

    /// Render the partial HTML payload returned to HTMX requests.
    ///
    /// This is also the render mode used when a route view is embedded inside a
    /// parent route via `ui::embed::<R>()`, so it should only emit the inner
    /// replaceable content rather than a full page document.
    fn fragment(&self, state: &AppState) -> impl Renderable;

    /// HTTP status used when the route is rendered as a fragment.
    ///
    /// Most fragment renders return `200 OK`, but routes that do not support
    /// HTMX replacement or embedding can override this, for example with
    /// `404 Not Found`.
    fn fragment_status() -> StatusCode {
        StatusCode::OK
    }

    fn render_document(&self, state: &AppState) -> Response
    where
        Self: Sized,
    {
        Html(self.document(state).render().into_inner()).into_response()
    }

    fn render_fragment(&self, state: &AppState) -> Response
    where
        Self: Sized,
    {
        (
            Self::fragment_status(),
            self.fragment(state).render().into_inner(),
        )
            .into_response()
    }
}

/// Shared contract for server-rendered UI routes.
///
/// Each route declares:
/// - `Input`: route-specific request data such as query/path params or form data
/// - `Authz`: the typed authorization rule that must succeed before `handle`
/// - `View`: the value that knows how to render full-document and HTMX responses
///
/// `Route` is intentionally a type-level, stateless contract. Implementors
/// should be marker types; runtime data belongs in `RouteContext`, `Input`, the
/// granted authz value, or the returned `View`.
///
/// `Input` may be a single extractor like `Form<T>` or a tuple like
/// `(Extension<MyCtx>, Path<Params>, Form<Body>)`. Keep any body-consuming
/// extractor last so extraction matches axum's rules.
#[async_trait]
pub trait Route: Send + Sync + 'static {
    type Input;
    type Authz: Authz<Self::Input>;
    type View: RouteView + Send;

    async fn handle(
        context: &RouteContext,
        granted: <Self::Authz as Authz<Self::Input>>::Granted,
        input: Self::Input,
    ) -> Result<Self::View, RouteError>;
}
