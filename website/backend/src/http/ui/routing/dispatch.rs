use axum::{
    extract::{FromRequest, FromRequestParts, Request, State},
    response::{IntoResponse, Response},
    routing::{MethodFilter, MethodRouter, on},
};
use axum_htmx::HxRequest;

use crate::http::router::AppState;

use super::{Authz, AuthzRequest, Route, RouteContext, RouteError, RouteView};

/// Register a route backed by the shared UI route contract for the provided
/// HTTP method filter.
///
/// The adapter:
/// - extracts the declared input
/// - detects whether the request came from HTMX
/// - runs the typed authz rule before the handler
/// - runs the route handler
/// - renders either the full document or the HTMX fragment
///
/// `R::Input` can be any axum extractor implementing `FromRequest<AppState>`,
/// including tuples such as `(Extension<T>, Path<P>, Form<F>)`. When using a
/// tuple input, any body-consuming extractor must be the final element.
pub fn route<R>(method: MethodFilter) -> MethodRouter<AppState>
where
    R: Route,
    R::Input: FromRequest<AppState> + Send + Sync + 'static,
    <R::Input as FromRequest<AppState>>::Rejection: IntoResponse,
    <R::Authz as Authz<R::Input>>::Granted: Send + 'static,
{
    on(
        method,
        |State(state): State<AppState>, request: Request| async move {
            dispatch_route::<R>(state, request).await
        },
    )
}

async fn dispatch_route<R>(state: AppState, request: Request) -> Response
where
    R: Route,
    R::Input: FromRequest<AppState> + Send + Sync + 'static,
    <R::Input as FromRequest<AppState>>::Rejection: IntoResponse,
    <R::Authz as Authz<R::Input>>::Granted: Send + 'static,
{
    let (context, request, is_htmx) = match prepare_route_request(state.clone(), request).await {
        Ok(result) => result,
        Err(response) => return response,
    };
    let input = match extract_route_input::<R>(state.clone(), request).await {
        Ok(input) => input,
        Err(response) => return response,
    };

    let view = match resolve::<R>(&context, input).await {
        Ok(view) => view,
        Err(error) => return error.into_response(),
    };

    render_route_view(&state, &view, is_htmx)
}

/// Resolve a route view from already-extracted input.
///
/// This is the shared route execution path used by mounted HTTP routes:
/// 1. run the typed authz rule against the extracted input
/// 2. call the route handler
/// 3. return the typed view without rendering it yet
async fn resolve<R>(context: &RouteContext, input: R::Input) -> Result<R::View, RouteError>
where
    R: Route,
    <R::Authz as Authz<R::Input>>::Granted: Send + 'static,
{
    let granted = R::Authz::authorize(context.state(), context.authz_request(), &input).await?;
    R::handle(context, granted, input).await
}

/// Split the incoming request into reusable route context plus the request that
/// will be consumed by the input extractor.
async fn prepare_route_request(
    state: AppState,
    request: Request,
) -> Result<(RouteContext, Request, bool), Response> {
    let (mut parts, body) = request.into_parts();
    let request_parts = parts.clone();
    let authz_request = AuthzRequest::from_headers(&parts.headers);
    let is_htmx = match HxRequest::from_request_parts(&mut parts, &state).await {
        Ok(HxRequest(is_htmx)) => is_htmx,
        Err(rejection) => return Err(rejection.into_response()),
    };
    let request = Request::from_parts(parts, body);
    let context = RouteContext::new(state, authz_request, request_parts);

    Ok((context, request, is_htmx))
}

/// Extract the typed route input from the request.
///
/// This follows axum's normal extraction model, so request extensions added by
/// middleware are available through `Extension<T>` inside `R::Input`.
async fn extract_route_input<R>(state: AppState, request: Request) -> Result<R::Input, Response>
where
    R: Route,
    R::Input: FromRequest<AppState> + Send + Sync + 'static,
    <R::Input as FromRequest<AppState>>::Rejection: IntoResponse,
{
    R::Input::from_request(request, &state)
        .await
        .map_err(IntoResponse::into_response)
}

/// Render the final HTTP response at the route boundary.
fn render_route_view<V>(state: &AppState, view: &V, is_htmx: bool) -> Response
where
    V: RouteView,
{
    if is_htmx {
        view.render_fragment(state)
    } else {
        view.render_document(state)
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{HeaderValue, StatusCode},
    };
    use hypertext::prelude::*;

    use super::*;

    struct MountedView;

    impl RouteView for MountedView {
        fn document(&self, _state: &AppState) -> impl Renderable {
            rsx! { <main>"document-body"</main> }
        }

        fn fragment(&self, _state: &AppState) -> impl Renderable {
            rsx! { <aside>"fragment-body"</aside> }
        }

        fn fragment_status() -> StatusCode {
            StatusCode::ACCEPTED
        }
    }

    struct MountedRoute;

    #[async_trait]
    impl Route for MountedRoute {
        type Input = super::super::NoInput;
        type Authz = super::super::Public;
        type View = MountedView;

        async fn handle(
            _context: &RouteContext,
            _granted: (),
            _input: Self::Input,
        ) -> Result<Self::View, RouteError> {
            Ok(MountedView)
        }
    }

    async fn response_body(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        String::from_utf8(body.to_vec()).expect("response body should be utf8")
    }

    #[tokio::test]
    async fn mounted_route_renders_document_for_normal_request() {
        let request = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("request should build");

        let response = dispatch_route::<MountedRoute>(AppState::for_test().await, request).await;
        let status = response.status();
        let body = response_body(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("document-body"));
        assert!(!body.contains("fragment-body"));
    }

    #[tokio::test]
    async fn mounted_route_renders_fragment_for_htmx_request() {
        let request = Request::builder()
            .uri("/")
            .header("HX-Request", HeaderValue::from_static("true"))
            .body(Body::empty())
            .expect("request should build");

        let response = dispatch_route::<MountedRoute>(AppState::for_test().await, request).await;
        let status = response.status();
        let body = response_body(response).await;

        assert_eq!(status, StatusCode::ACCEPTED);
        assert!(body.contains("fragment-body"));
        assert!(!body.contains("document-body"));
    }
}
