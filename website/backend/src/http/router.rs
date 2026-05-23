use std::{env, path::PathBuf};

use axum::{
    Router,
    extract::{DefaultBodyLimit, Request},
    middleware::{self, Next},
    response::Response,
    routing::{MethodFilter, delete, get, post, put},
};
use serde_json::json;

use crate::{
    db,
    http::{metrics::AppMetrics, ui},
    upload::{
        ChunkedUploadStore, MAX_VIDEO_UPLOAD_SIZE_BYTES, VIDEO_UPLOAD_CHUNK_REQUEST_LIMIT_BYTES,
    },
};

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
    chunked_uploads: ChunkedUploadStore,
    metrics: AppMetrics,
    run_analysis_jobs: bool,
}

pub async fn run(
    db: db::Database,
    upload_bucket_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = AppState {
        db,
        chunked_uploads: ChunkedUploadStore::new(upload_bucket_path.join(".partial"))?,
        metrics: AppMetrics::default(),
        run_analysis_jobs: true,
    };
    crate::http::canary::spawn_canary(state.clone());

    let port = env::var("PORT")
        .unwrap_or_else(|_| "8080".to_owned())
        .parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    println!(
        "{}",
        json!({
            "level": "info",
            "component": "http",
            "event": "listening",
            "addr": format!("0.0.0.0:{port}"),
        })
    );
    Ok(axum::serve(listener, app(state)).await?)
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", ui::route::<ui::features::HomePage>(MethodFilter::GET))
        .route(
            "/analyses",
            ui::route::<ui::features::AnalysesPage>(MethodFilter::GET),
        )
        .route("/healthz", get(ui::features::healthz))
        .route("/metrics", get(crate::http::metrics::serve_metrics))
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
        .route("/videos/uploads", post(ui::features::start_chunked_upload))
        .route(
            "/videos/uploads/{upload_id}/chunks/{chunk_index}",
            put(ui::features::upload_chunk).layer(DefaultBodyLimit::max(
                VIDEO_UPLOAD_CHUNK_REQUEST_LIMIT_BYTES,
            )),
        )
        .route(
            "/videos/uploads/{upload_id}/complete",
            post(ui::features::complete_chunked_upload),
        )
        .route(
            "/videos/uploads/{upload_id}",
            delete(ui::features::cancel_chunked_upload),
        )
        .route(
            "/videos/{video_key}/delete",
            ui::route::<ui::features::DeleteVideoRoute>(MethodFilter::POST),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(MAX_VIDEO_UPLOAD_SIZE_BYTES))
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

    println!(
        "{}",
        json!({
            "level": "info",
            "component": "http",
            "event": "request",
            "method": method.to_string(),
            "uri": uri.to_string(),
            "htmx": is_htmx,
        })
    );

    next.run(request).await
}

impl AppState {
    pub fn db(&self) -> &db::Database {
        &self.db
    }

    pub fn chunked_uploads(&self) -> &ChunkedUploadStore {
        &self.chunked_uploads
    }

    pub fn metrics(&self) -> &AppMetrics {
        &self.metrics
    }

    pub fn runs_analysis_jobs(&self) -> bool {
        self.run_analysis_jobs
    }

    #[cfg(test)]
    pub async fn for_test() -> Self {
        let test_database = crate::test::database::init_with_bucket_path()
            .await
            .expect("test database should initialize");

        Self {
            db: test_database.db,
            chunked_uploads: ChunkedUploadStore::new(
                test_database.upload_bucket_path.join(".partial"),
            )
            .expect("test chunk staging should initialize"),
            metrics: AppMetrics::default(),
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
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::upload::{MAX_VIDEO_UPLOAD_SIZE_BYTES, VIDEO_UPLOAD_CHUNK_SIZE_BYTES};

    use super::*;

    async fn response_text(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        String::from_utf8(body.to_vec()).expect("response body should be utf8")
    }

    async fn response_json(response: Response) -> Value {
        serde_json::from_str(&response_text(response).await).expect("response body should be json")
    }

    async fn test_app() -> Router {
        app(AppState::for_test().await)
    }

    async fn start_chunked_upload(app: &Router, filename: &str, size: u64) -> String {
        let response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/videos/uploads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({ "filename": filename, "size": size }).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(
            payload["chunk_size"].as_u64(),
            Some(VIDEO_UPLOAD_CHUNK_SIZE_BYTES as u64)
        );
        assert_eq!(
            payload["max_size"].as_u64(),
            Some(MAX_VIDEO_UPLOAD_SIZE_BYTES as u64)
        );
        payload["upload_id"]
            .as_str()
            .expect("upload id should be returned")
            .to_owned()
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
        assert!(html.contains("<html lang=en>") || html.contains(r#"<html lang="en">"#));
        assert!(!html.contains("<html lang=en data-theme="));
        assert!(!html.contains(r#"<html lang="en" data-theme="#));
        assert!(html.contains("Upload videos"));
        assert!(html.contains("Uploads are limited to 4 GiB."));
        assert!(html.contains("alpinejs"));
        assert!(html.contains(r#"x-data="chunkedVideoUpload"#));
        assert!(html.contains(r#"x-ref="video""#));
        assert!(html.contains(r#"x-on:submit.prevent="upload""#));
        assert!(html.contains(r#"x-text="status""#));
        assert!(html.contains("maxChunkAttempts"));
        assert!(html.contains("uploadChunkWithRetry"));
        assert!(html.contains(r#"x-data="videoPlayer"#));
        assert!(html.contains(r#"id="provider-switch""#));
        assert!(html.contains(r#"name="provider""#));
        assert!(html.contains(r#"value="gemini""#));
        assert!(html.contains(r#"value="openai""#));
        assert!(html.contains(r#"aria-label="Gemini""#));
        assert!(html.contains(r#"aria-label="OpenAI""#));
        assert!(html.contains(r#"x-model="provider""#));
        assert!(html.contains(r#"x-show="provider === 'openai'""#));
        assert!(html.contains("x-cloak"));
        assert!(html.contains(r#"name="frame_sample_rate_fps""#));
        assert!(html.contains(r#"value="2""#));
        assert!(html.contains(r#"value="4""#));
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
        assert!(html.contains("0.00 MB"));
        assert!(!html.contains("11 bytes"));
    }

    #[tokio::test]
    async fn home_page_renders_recent_analysis_widget() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");
        let analysis = db::analysis::Analysis::create(
            state.db(),
            "Summarize the uploaded video",
            vec![video.file.key().to_owned()],
        )
        .await
        .expect("analysis should create");
        analysis
            .complete(state.db(), "The video contains a short test scene.")
            .await
            .expect("analysis should complete");

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

        assert!(html.contains("Recent analyses"));
        assert!(html.contains(r#"href="/analyses""#));
        assert!(html.contains("sample.mp4"));
        assert!(html.contains("Summarize the uploaded video"));
        assert!(html.contains("The video contains a short test scene."));
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
    async fn analyze_route_rejects_unknown_provider() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");
        let body = format!(
            "provider=anthropic&video_keys={}&prompt=Summarize+the+video",
            video.file.key()
        );

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

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_text(response).await,
            "unsupported analysis provider"
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
    async fn analysis_status_route_renders_failure_diagnostics_and_events() {
        let state = AppState::for_test().await;
        let analysis = db::analysis::Analysis::create(
            state.db(),
            "Summarize the video",
            vec!["sample.mp4".to_owned()],
        )
        .await
        .expect("analysis should create");
        db::analysis::AnalysisEvent::record(
            state.db(),
            db::analysis::NewAnalysisEvent {
                analysis_key: analysis.key(),
                provider: "openai".to_owned(),
                stage: "provider_request".to_owned(),
                level: "error".to_owned(),
                message: "provider request failed".to_owned(),
                attempt: Some(3),
                attempts: Some(3),
                payload_bytes: Some(2048),
                offset_bytes: None,
                size_bytes: None,
                duration_ms: Some(9000),
            },
        )
        .await
        .expect("event should record");
        analysis
            .fail_with_diagnostic(
                state.db(),
                db::analysis::AnalysisFailureDiagnostic {
                    stage: "provider_request".to_owned(),
                    kind: "timeout".to_owned(),
                    retryable: true,
                    attempt: Some(3),
                    attempts: Some(3),
                    payload_bytes: Some(2048),
                    message: "provider request failed".to_owned(),
                },
            )
            .await
            .expect("analysis should fail");

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
        let html = response_text(response).await;

        assert!(html.contains("Failure diagnostics"));
        assert!(html.contains("provider_request"));
        assert!(html.contains("timeout"));
        assert!(html.contains("provider request failed"));
        assert!(html.contains("Event history"));
    }

    #[tokio::test]
    async fn analyses_page_renders_paginated_analysis_history() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");
        let analysis = db::analysis::Analysis::create(
            state.db(),
            "List the visible actions",
            vec![video.file.key().to_owned()],
        )
        .await
        .expect("analysis should create");
        analysis
            .complete(state.db(), "A person moves through the frame.")
            .await
            .expect("analysis should complete");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri("/analyses")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let html = response_text(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("Analysis history"));
        assert!(html.contains("sample.mp4"));
        assert!(html.contains("List the visible actions"));
        assert!(html.contains("A person moves through the frame."));
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
        assert!(html.contains(r#"/delete""#));
        assert!(html.contains(r##"hx-target="#video-workspace""##));
    }

    #[tokio::test]
    async fn chunked_video_upload_completes_and_returns_updated_workspace() {
        let state = AppState::for_test().await;
        let app = app(state.clone());
        let first = b"video ".to_vec();
        let second = b"bytes".to_vec();
        let upload_id =
            start_chunked_upload(&app, "sample.mp4", (first.len() + second.len()) as u64).await;

        for (index, chunk) in [first.as_slice(), second.as_slice()]
            .into_iter()
            .enumerate()
        {
            let response = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("PUT")
                        .uri(format!("/videos/uploads/{upload_id}/chunks/{index}"))
                        .header("Content-Type", "application/octet-stream")
                        .body(Body::from(chunk.to_vec()))
                        .expect("request should build"),
                )
                .await
                .expect("request should complete");

            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/videos/uploads/{upload_id}/complete"))
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
        assert!(html.contains("sample.mp4"));

        let stored = db::video::Video::read_by_name(state.db(), "sample.mp4")
            .await
            .expect("video should read")
            .expect("video should exist");
        assert_eq!(stored.bytes, b"video bytes");
    }

    #[tokio::test]
    async fn chunked_upload_accepts_duplicate_completed_chunk_retries() {
        let state = AppState::for_test().await;
        let app = app(state.clone());
        let upload_id = start_chunked_upload(&app, "retry.mp4", 11).await;

        for index in [0, 0, 1] {
            let chunk = if index == 0 { "video " } else { "bytes" };
            let response = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("PUT")
                        .uri(format!("/videos/uploads/{upload_id}/chunks/{index}"))
                        .header("Content-Type", "application/octet-stream")
                        .body(Body::from(chunk))
                        .expect("request should build"),
                )
                .await
                .expect("request should complete");

            assert_eq!(response.status(), StatusCode::OK);
        }

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/videos/uploads/{upload_id}/complete"))
                    .header("HX-Request", "true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::OK);

        let stored = db::video::Video::read_by_name(state.db(), "retry.mp4")
            .await
            .expect("video should read")
            .expect("video should exist");
        assert_eq!(stored.bytes, b"video bytes");
    }

    #[tokio::test]
    async fn chunked_upload_start_rejects_invalid_sizes() {
        let app = test_app().await;

        let empty_response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/videos/uploads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({ "filename": "empty.mp4", "size": 0 }).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(empty_response.status(), StatusCode::BAD_REQUEST);

        let oversized_response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/videos/uploads")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "filename": "oversized.mp4",
                            "size": MAX_VIDEO_UPLOAD_SIZE_BYTES as u64 + 1
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(oversized_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn chunked_upload_rejects_oversized_chunk_body() {
        let app = test_app().await;
        let upload_id = start_chunked_upload(
            &app,
            "oversized-chunk.mp4",
            VIDEO_UPLOAD_CHUNK_SIZE_BYTES as u64 + 1,
        )
        .await;

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri(format!("/videos/uploads/{upload_id}/chunks/0"))
                    .header("Content-Type", "application/octet-stream")
                    .body(Body::from(vec![0; VIDEO_UPLOAD_CHUNK_SIZE_BYTES + 1]))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn chunked_upload_rejects_out_of_order_chunk_indexes() {
        let app = test_app().await;
        let upload_id = start_chunked_upload(&app, "sample.mp4", 10).await;

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri(format!("/videos/uploads/{upload_id}/chunks/1"))
                    .header("Content-Type", "application/octet-stream")
                    .body(Body::from("video"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn chunked_upload_cancel_removes_session() {
        let app = test_app().await;
        let upload_id = start_chunked_upload(&app, "sample.mp4", 10).await;

        let cancel_response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("DELETE")
                    .uri(format!("/videos/uploads/{upload_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(cancel_response.status(), StatusCode::NO_CONTENT);

        let complete_response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri(format!("/videos/uploads/{upload_id}/complete"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(complete_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn metrics_route_exports_analysis_and_upload_counters() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");
        let app = app(state);
        let upload_id = start_chunked_upload(&app, "metrics.mp4", 5).await;

        let chunk_response = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .method("PUT")
                    .uri(format!("/videos/uploads/{upload_id}/chunks/0"))
                    .header("Content-Type", "application/octet-stream")
                    .body(Body::from("bytes"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(chunk_response.status(), StatusCode::OK);

        let body = format!(
            "provider=openai&video_keys={}&prompt=Summarize",
            video.file.key()
        );
        let analysis_response = app
            .clone()
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
        assert_eq!(analysis_response.status(), StatusCode::OK);

        let metrics_response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let metrics = response_text(metrics_response).await;

        assert!(metrics.contains("leo_analysis_submissions_total{provider=\"openai\"} 1"));
        assert!(metrics.contains("leo_upload_sessions_total{result=\"started\"} 1"));
        assert!(metrics.contains("leo_upload_chunks_total{result=\"accepted\"} 1"));
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
    async fn video_route_serves_requested_byte_range() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/video/{}", video.name))
                    .header("Range", "bytes=0-4")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let accept_ranges = response
            .headers()
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_range = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");

        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(accept_ranges.as_deref(), Some("bytes"));
        assert_eq!(content_range.as_deref(), Some("bytes 0-4/11"));
        assert_eq!(content_length.as_deref(), Some("5"));
        assert_eq!(body.as_ref(), b"video");
    }

    #[tokio::test]
    async fn video_route_serves_open_ended_byte_range() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/video/{}", video.name))
                    .header("Range", "bytes=6-")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let content_range = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");

        assert_eq!(status, StatusCode::PARTIAL_CONTENT);
        assert_eq!(content_range.as_deref(), Some("bytes 6-10/11"));
        assert_eq!(body.as_ref(), b"bytes");
    }

    #[tokio::test]
    async fn video_route_rejects_unsatisfiable_byte_range() {
        let state = AppState::for_test().await;
        let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let response = app(state)
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/video/{}", video.name))
                    .header("Range", "bytes=99-100")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let accept_ranges = response
            .headers()
            .get("accept-ranges")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let content_range = response
            .headers()
            .get("content-range")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");

        assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(accept_ranges.as_deref(), Some("bytes"));
        assert_eq!(content_range.as_deref(), Some("bytes */11"));
        assert!(body.is_empty());
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
