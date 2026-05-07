use std::marker::PhantomData;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use hypertext::{Buffer, Renderable};

use crate::http::router::AppState;

use super::{Authz, ReuseGranted, Route, RouteContext, RouteError, RouteView};

/// A lazily rendered embedded route fragment.
///
/// Route execution has already happened by the time this value exists. It keeps
/// the resolved child view plus app state and renders the child's fragment only
/// when the parent response is finally rendered.
pub struct EmbeddedFragment<V> {
    state: AppState,
    view: V,
}

/// Fragment type returned after an embedded route has been fully resolved.
///
/// The route's input extraction, authz, and handler have already run by the
/// time this value exists. Rendering only emits the child fragment HTML.
pub type RouteFragment<R> = EmbeddedFragment<<R as Route>::View>;

/// Builder for embedding a route as a lazily rendered child fragment.
///
/// Typical flow:
/// - choose how the child input is provided with `.input(...)` or
///   `.current_input()`
/// - optionally call `.reuse_granted(...)` to derive child authz from the
///   parent granted value. Without this, the authz will be reperformed.
/// - finish with `.resolve(...)` to run the child route and get a fragment
pub struct EmbedBuilder<R, I = MissingInput, A = Authorize> {
    input: I,
    auth: A,
    route: PhantomData<R>,
}

/// Marker used before the embed builder has been configured with child input.
pub struct MissingInput;

/// Marker carrying explicitly supplied child input.
pub struct ExplicitInput<I>(I);

/// Marker indicating the child input should be re-extracted from the current
/// request parts.
pub struct CurrentInput;

/// Marker indicating the child route should run its own authz rule.
pub struct Authorize;

/// Marker carrying the parent granted value for child authz reuse.
pub struct Reuse<'a, ParentGranted>(&'a ParentGranted);

impl<V> EmbeddedFragment<V> {
    fn new(state: AppState, view: V) -> Self {
        Self { state, view }
    }
}

impl<R> EmbedBuilder<R> {
    fn new() -> Self {
        Self {
            input: MissingInput,
            auth: Authorize,
            route: PhantomData,
        }
    }
}

impl<R, A> EmbedBuilder<R, MissingInput, A>
where
    R: Route,
{
    /// Supply explicit input for the child route.
    ///
    /// Use this when the parent already has the exact child input value and
    /// does not need to re-extract it from the current request.
    pub fn input(self, input: R::Input) -> EmbedBuilder<R, ExplicitInput<R::Input>, A> {
        EmbedBuilder {
            input: ExplicitInput(input),
            auth: self.auth,
            route: PhantomData,
        }
    }

    /// Re-extract the child input from the current request head.
    ///
    /// This only supports `FromRequestParts` extractors, so it is suitable for
    /// path params, query params, headers, and similar request metadata. It
    /// does not replay body-consuming `FromRequest` extractors.
    pub fn current_input(self) -> EmbedBuilder<R, CurrentInput, A>
    where
        R::Input: FromRequestParts<AppState> + Send,
        <R::Input as FromRequestParts<AppState>>::Rejection: std::fmt::Display,
    {
        EmbedBuilder {
            input: CurrentInput,
            auth: self.auth,
            route: PhantomData,
        }
    }
}

impl<R, I> EmbedBuilder<R, I, Authorize> {
    /// Reuse the parent granted value instead of rerunning child authz.
    ///
    /// This is useful when the parent route has already proved a stronger or
    /// equivalent access condition and the child granted type can be derived
    /// through `ReuseGranted<ParentGranted>`.
    pub fn reuse_granted<'a, ParentGranted>(
        self,
        parent_granted: &'a ParentGranted,
    ) -> EmbedBuilder<R, I, Reuse<'a, ParentGranted>> {
        EmbedBuilder {
            input: self.input,
            auth: Reuse(parent_granted),
            route: PhantomData,
        }
    }
}

impl<V> Renderable for EmbeddedFragment<V>
where
    V: RouteView,
{
    fn render_to(&self, buffer: &mut Buffer) {
        self.view.fragment(&self.state).render_to(buffer);
    }
}

/// Begin embedding a child route.
///
/// Configure the builder with either:
/// - `.input(...)` to pass explicit child input
/// - `.current_input()` to re-extract child input from the current request
///
/// By default the child route reruns authz. Call `.reuse_granted(...)` to
/// derive the child granted value from the parent granted value instead.
pub fn embed<R>() -> EmbedBuilder<R>
where
    R: Route,
{
    EmbedBuilder::new()
}

async fn extract_current_input<R>(context: &RouteContext) -> Result<R::Input, RouteError>
where
    R: Route,
    R::Input: FromRequestParts<AppState> + Send,
    <R::Input as FromRequestParts<AppState>>::Rejection: std::fmt::Display,
{
    let mut parts = context.request_parts().clone();
    R::Input::from_request_parts(&mut parts, context.state())
        .await
        .map_err(|rejection| RouteError::EmbeddedInput {
            route: std::any::type_name::<R>(),
            message: rejection.to_string(),
        })
}

/// Internal strategy trait for resolving child input from a builder state.
#[async_trait]
pub trait ResolveEmbedInput<R: Route> {
    async fn resolve_input(self, context: &RouteContext) -> Result<R::Input, RouteError>;
}

#[async_trait]
impl<R> ResolveEmbedInput<R> for ExplicitInput<R::Input>
where
    R: Route,
    R::Input: Send,
{
    async fn resolve_input(self, _context: &RouteContext) -> Result<R::Input, RouteError> {
        Ok(self.0)
    }
}

#[async_trait]
impl<R> ResolveEmbedInput<R> for CurrentInput
where
    R: Route,
    R::Input: FromRequestParts<AppState> + Send,
    <R::Input as FromRequestParts<AppState>>::Rejection: std::fmt::Display,
{
    async fn resolve_input(self, context: &RouteContext) -> Result<R::Input, RouteError> {
        extract_current_input::<R>(context).await
    }
}

/// Internal strategy trait for resolving the child granted authz value.
#[async_trait]
pub trait ResolveEmbedGranted<R: Route> {
    async fn resolve_granted(
        self,
        context: &RouteContext,
        input: &R::Input,
    ) -> Result<<R::Authz as Authz<R::Input>>::Granted, RouteError>;
}

#[async_trait]
impl<R> ResolveEmbedGranted<R> for Authorize
where
    R: Route,
    R::Input: Sync,
    <R::Authz as Authz<R::Input>>::Granted: Send + 'static,
{
    async fn resolve_granted(
        self,
        context: &RouteContext,
        input: &R::Input,
    ) -> Result<<R::Authz as Authz<R::Input>>::Granted, RouteError> {
        Ok(R::Authz::authorize(context.state(), context.authz_request(), input).await?)
    }
}

#[async_trait]
impl<R, ParentGranted> ResolveEmbedGranted<R> for Reuse<'_, ParentGranted>
where
    R: Route,
    ParentGranted: Sync,
    <R::Authz as Authz<R::Input>>::Granted: ReuseGranted<ParentGranted> + Send + 'static,
{
    async fn resolve_granted(
        self,
        _context: &RouteContext,
        _input: &R::Input,
    ) -> Result<<R::Authz as Authz<R::Input>>::Granted, RouteError> {
        Ok(<R::Authz as Authz<R::Input>>::Granted::reuse_from(self.0))
    }
}

impl<R, I, A> EmbedBuilder<R, I, A>
where
    R: Route,
    I: ResolveEmbedInput<R>,
    A: ResolveEmbedGranted<R>,
{
    /// Resolve the configured child route into a lazily rendered fragment.
    ///
    /// This runs child input resolution, authz resolution, and `R::handle(...)`
    /// immediately, then stores the resulting view for fragment rendering by
    /// the parent.
    pub async fn resolve(self, context: &RouteContext) -> Result<RouteFragment<R>, RouteError> {
        let input = self.input.resolve_input(context).await?;
        let granted = self.auth.resolve_granted(context, &input).await?;
        let view = R::handle(context, granted, input).await?;

        Ok(EmbeddedFragment::new(context.state().clone(), view))
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::{
        extract::FromRequestParts,
        http::{HeaderMap, HeaderValue, request::Parts},
    };
    use hypertext::prelude::*;

    use super::*;
    use crate::http::router::AppState;

    async fn test_context(headers: HeaderMap) -> RouteContext {
        let request = axum::http::Request::builder()
            .uri("/embedded")
            .body(())
            .expect("request should build");
        let (mut parts, _) = request.into_parts();
        parts.headers = headers.clone();

        RouteContext::new(
            AppState::for_test().await,
            super::super::AuthzRequest::from_headers(&headers),
            parts,
        )
    }

    struct TestView {
        document_html: String,
        fragment_html: String,
    }

    impl RouteView for TestView {
        fn document(&self, _state: &AppState) -> impl Renderable {
            let html = self.document_html.clone();
            rsx! { <div>(html)</div> }
        }

        fn fragment(&self, _state: &AppState) -> impl Renderable {
            let html = self.fragment_html.clone();
            rsx! { <div>(html)</div> }
        }
    }

    struct ExplicitRoute;

    struct ExplicitAuthz;

    #[async_trait]
    impl Authz<String> for ExplicitAuthz {
        type Granted = String;

        async fn authorize(
            _state: &AppState,
            _request: &super::super::AuthzRequest,
            input: &String,
        ) -> Result<Self::Granted, super::super::AuthzError> {
            Ok(format!("authorized:{input}"))
        }
    }

    #[async_trait]
    impl Route for ExplicitRoute {
        type Input = String;
        type Authz = ExplicitAuthz;
        type View = TestView;

        async fn handle(
            _context: &RouteContext,
            granted: String,
            input: Self::Input,
        ) -> Result<Self::View, RouteError> {
            Ok(TestView {
                document_html: format!("doc:{granted}:{input}"),
                fragment_html: format!("frag:{granted}:{input}"),
            })
        }
    }

    #[tokio::test]
    async fn embed_with_explicit_input_authorizes_and_renders_fragment() {
        let context = test_context(HeaderMap::new()).await;

        let fragment = embed::<ExplicitRoute>()
            .input("item-42".to_owned())
            .resolve(&context)
            .await
            .expect("explicit embed should resolve");

        let html = fragment.render().into_inner();

        assert!(html.contains("frag:authorized:item-42:item-42"));
        assert!(!html.contains("doc:authorized:item-42:item-42"));
    }

    struct CurrentInput(String);

    impl FromRequestParts<AppState> for CurrentInput {
        type Rejection = &'static str;

        async fn from_request_parts(
            parts: &mut Parts,
            _state: &AppState,
        ) -> Result<Self, Self::Rejection> {
            let Some(value) = parts.headers.get("x-test-input") else {
                return Err("missing x-test-input");
            };

            let value = value.to_str().map_err(|_| "invalid x-test-input")?;
            Ok(Self(value.to_owned()))
        }
    }

    struct ParentGrant(&'static str);

    struct ChildGrant(String);

    impl ReuseGranted<ParentGrant> for ChildGrant {
        fn reuse_from(parent: &ParentGrant) -> Self {
            Self(format!("reused:{}", parent.0))
        }
    }

    struct ReusedAuthz;

    #[async_trait]
    impl Authz<CurrentInput> for ReusedAuthz {
        type Granted = ChildGrant;

        async fn authorize(
            _state: &AppState,
            _request: &super::super::AuthzRequest,
            _input: &CurrentInput,
        ) -> Result<Self::Granted, super::super::AuthzError> {
            panic!("reuse_granted should skip child authorization");
        }
    }

    struct CurrentReuseRoute;

    #[async_trait]
    impl Route for CurrentReuseRoute {
        type Input = CurrentInput;
        type Authz = ReusedAuthz;
        type View = TestView;

        async fn handle(
            _context: &RouteContext,
            granted: ChildGrant,
            input: Self::Input,
        ) -> Result<Self::View, RouteError> {
            Ok(TestView {
                document_html: String::from("unused"),
                fragment_html: format!("grant={} input={}", granted.0, input.0),
            })
        }
    }

    #[tokio::test]
    async fn embed_with_current_input_and_reused_granted_skips_auth() {
        let mut headers = HeaderMap::new();
        headers.insert("x-test-input", HeaderValue::from_static("session-123"));
        let context = test_context(headers).await;
        let parent_granted = ParentGrant("user-456");

        let fragment = embed::<CurrentReuseRoute>()
            .current_input()
            .reuse_granted(&parent_granted)
            .resolve(&context)
            .await
            .expect("current embed should resolve");

        let html = fragment.render().into_inner();

        assert!(html.contains("grant=reused:user-456 input=session-123"));
    }

    struct MissingHeaderRoute;

    #[async_trait]
    impl Route for MissingHeaderRoute {
        type Input = CurrentInput;
        type Authz = super::super::Public;
        type View = TestView;

        async fn handle(
            _context: &RouteContext,
            _granted: (),
            _input: Self::Input,
        ) -> Result<Self::View, RouteError> {
            unreachable!("input extraction should fail before handle");
        }
    }

    #[tokio::test]
    async fn current_input_reports_embedded_input_error() {
        let context = test_context(HeaderMap::new()).await;

        let error = match embed::<MissingHeaderRoute>()
            .current_input()
            .resolve(&context)
            .await
        {
            Ok(_) => panic!("missing header should fail"),
            Err(error) => error,
        };

        match error {
            RouteError::EmbeddedInput { route, message } => {
                assert!(route.ends_with("MissingHeaderRoute"));
                assert_eq!(message, "missing x-test-input");
            }
            other => panic!("unexpected route error: {other:?}"),
        }
    }
}
