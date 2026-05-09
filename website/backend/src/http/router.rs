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
    run_analysis_jobs: bool,
}

pub async fn run(db: db::Database) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        db,
        run_analysis_jobs: true,
    };

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    println!("Listening on http://localhost:8080");
    Ok(axum::serve(listener, app(state)).await?)
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", ui::route::<ui::features::HomePage>(MethodFilter::GET))
        .route("/healthz", get(ui::features::healthz))
        .route("/video/{key}", get(crate::http::video::serve))
        .route(
            "/analysis",
            ui::route::<ui::features::AnalyzeRoute>(MethodFilter::POST),
        )
        .route(
            "/analysis/{analysis_id}",
            ui::route::<ui::features::AnalysisStatusRoute>(MethodFilter::GET),
        )
        .route(
            "/videos",
            ui::route::<ui::features::UploadVideoRoute>(MethodFilter::POST),
        )
        .route(
            "/videos/{video_key}/delete",
            ui::route::<ui::features::DeleteVideoRoute>(MethodFilter::POST),
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

    pub fn runs_analysis_jobs(&self) -> bool {
        self.run_analysis_jobs
    }

    #[cfg(test)]
    pub async fn for_test() -> Self {
        Self {
            db: crate::test::database::init()
                .await
                .expect("test database should initialize"),
            run_analysis_jobs: false,
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
        assert!(html.contains("alpinejs"));
        assert!(html.contains(r#"x-data="videoPlayer"#));
        assert!(!html.contains("Analysis status"));
        assert!(!html.contains("Upload and provider status"));
    }

    #[tokio::test]
    async fn home_page_renders_video_player_dropdown_with_uploaded_videos() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let html = response_text(response).await;

        assert!(html.contains("Preview video"));
        assert!(html.contains(r#"x-data="videoPlayer""#));
        assert!(html.contains(r#"x-model="selectedVideo""#));
        assert!(html.contains(r#"x-bind:src="selectedVideo""#));
        assert!(html.contains(video.path.as_str()));
        assert!(html.contains("sample.mp4"));
    }

    #[tokio::test]
    async fn analyze_route_requires_selected_videos() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/analysis")
                    .header("HX-Request", "true")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from("prompt=Summarize+the+video"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_text(response).await,
            "select at least one video to analyze"
        );
    }

    #[tokio::test]
    async fn analyze_route_rejects_missing_selected_video() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/analysis")
                    .header("HX-Request", "true")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "video_keys=missing.mp4&prompt=Summarize+the+video",
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_text(response).await,
            "selected video was not found"
        );
    }

    #[tokio::test]
    async fn analyze_route_rejects_more_than_ten_videos() {
        let body = (0..11)
            .map(|index| format!("video_keys=clip-{index}.mp4"))
            .chain(["prompt=Summarize+the+videos".to_owned()])
            .collect::<Vec<_>>()
            .join("&");

        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/analysis")
                    .header("HX-Request", "true")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_text(response).await,
            "select no more than 10 videos to analyze"
        );
    }

    #[tokio::test]
    async fn analyze_route_returns_polling_fragment_for_valid_request() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");
        let body = format!("video_keys={}&prompt=Summarize+the+video", video.file.key());

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/analysis")
                    .header("HX-Request", "true")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let html = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"id="analysis-result""#));
        assert!(html.contains("Analysis queued"));
        assert!(html.contains(r#"hx-get="/analysis/"#));
        assert!(html.contains(r#"hx-trigger="every 2s""#));
        assert!(!html.contains(r#"hx-trigger="load, every 2s""#));
    }

    #[tokio::test]
    async fn analysis_status_route_renders_completed_result_without_polling() {
        let state = AppState::for_test().await;
        let analysis = db::analysis::Analysis::create(
            state.db(),
            "Summarize the video",
            vec!["sample.mp4".to_owned()],
        )
        .await
        .expect("analysis should create");
        analysis
            .complete(state.db(), "The video shows a clean test result.")
            .await
            .expect("analysis should complete");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/analysis/{}", analysis.key()))
                    .header("HX-Request", "true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let html = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"id="analysis-result""#));
        assert!(html.contains("The video shows a clean test result."));
        assert!(!html.contains("hx-trigger"));
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
        assert!(html.contains(r#"id="video-workspace""#));
        assert!(html.contains(r#"x-data="videoPlayer""#));
        assert!(html.contains(r#"id="video-selection""#));
        assert!(html.contains("sample.mp4"));
        assert!(html.contains(r#"name="video_keys""#));
        assert!(html.contains("Delete"));
        assert!(html.contains(r#"type="button""#));
        assert!(html.contains(r#"hx-post="/videos/"#));
        assert!(html.contains(r#"/delete"#));
        assert!(html.contains(r##"hx-target="#video-workspace""##));
    }

    #[tokio::test]
    async fn video_delete_route_removes_video_and_returns_updated_picker() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!(
                        "/videos/{}/delete",
                        video.file.key().trim_start_matches('/')
                    ))
                    .header("HX-Request", "true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let html = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains(r#"id="video-workspace""#));
        assert!(html.contains(r#"x-data="videoPlayer""#));
        assert!(html.contains(r#"id="video-selection""#));
        assert!(html.contains("No videos have been uploaded yet."));
        assert!(!html.contains("sample.mp4"));

        let videos = db::video::Video::list(state.db())
            .await
            .expect("videos should list");
        assert!(videos.is_empty());
    }

    #[tokio::test]
    async fn video_route_serves_uploaded_video_bytes() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/video/{}", video.name))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");

        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type.as_deref(), Some("video/mp4"));
        assert_eq!(body.as_ref(), b"video bytes");
    }

    #[tokio::test]
    async fn video_route_returns_not_found_for_missing_video() {
        let response = test_app()
            .await
            .oneshot(
                HttpRequest::builder()
                    .uri("/video/missing.mp4")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
