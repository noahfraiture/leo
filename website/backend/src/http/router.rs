use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::Response,
    routing::{MethodFilter, get},
};

use crate::{db, http::ui};

/// Shared application services passed through axum state and reused by UI
/// route dispatch.
///
/// This must stay cheap to clone because axum state extraction and the custom
/// route adapter pass cloned `AppState` values through per-request async
/// boundaries rather than sharing a mutable singleton. Fields should therefore
/// be handles or internally shared types, not large owned payloads.
#[derive(Clone)]
pub struct AppState {
    db: db::Database,
}

pub async fn run(db: db::Database) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState { db };

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    println!("Listening on http://localhost:8080");
    Ok(axum::serve(listener, app(state)).await?)
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", ui::route::<ui::features::HomePage>(MethodFilter::GET))
        .route("/healthz", get(ui::features::healthz))
        .route(
            "/analysis",
            ui::route::<ui::features::AnalyzeRoute>(MethodFilter::POST),
        )
        .route(
            "/videos",
            ui::route::<ui::features::UploadVideoRoute>(MethodFilter::POST),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(512 * 1024 * 1024))
        .layer(middleware::from_fn(log_request))
}

async fn log_request(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let is_htmx = request
        .headers()
        .get("HX-Request")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("true"));

    println!("[http] {method} {uri} htmx={is_htmx}");

    next.run(request).await
}

impl AppState {
    pub fn db(&self) -> &db::Database {
        &self.db
    }

    #[cfg(test)]
    pub async fn for_test() -> Self {
        Self {
            db: crate::test::database::init()
                .await
                .expect("test database should initialize"),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request as HttpRequest, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    async fn response_text(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        String::from_utf8(body.to_vec()).expect("response body should be utf8")
    }

    async fn test_app() -> Router {
        app(AppState::for_test().await)
    }

    #[tokio::test]
    async fn healthz_returns_ok() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_text(response).await, "OK");
    }

    #[tokio::test]
    async fn home_page_renders_video_upload_shell() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let html = response_text(response).await;

        assert!(html.contains("Video analysis"));
        assert!(html.contains("Upload videos"));
        assert!(html.contains("Analysis status"));
        assert!(html.contains("Ready"));
    }

    #[tokio::test]
    async fn analyze_route_returns_dummy_fragment() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/analysis")
                    .header("HX-Request", "true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let html = response_text(response).await;

        assert!(html.contains(r#"id="analysis-result""#));
        assert!(html.contains("Video analysis is not implemented yet."));
    }

    #[tokio::test]
    async fn video_upload_route_returns_updated_video_picker() {
        let boundary = "leo-test-boundary";
        let body = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"video\"; filename=\"sample.mp4\"\r\n\
             Content-Type: video/mp4\r\n\r\n\
             video bytes\r\n\
             --{boundary}--\r\n"
        );

        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/videos")
                    .header("HX-Request", "true")
                    .header(
                        "Content-Type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let html = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"id="video-selection""#));
        assert!(html.contains("sample.mp4"));
        assert!(html.contains(r#"name="video_keys""#));
    }
}
