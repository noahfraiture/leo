pub mod camera;
pub mod server;

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header::CONTENT_TYPE},
        response::Response,
    };
    use tower::ServiceExt;

    use super::{camera::Camera, server::app};

    fn camera() -> Camera {
        Camera::new()
    }

    async fn get(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    async fn body(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let response = get(app(camera()), "/health").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_path_returns_not_found() {
        let response = get(app(camera()), "/missing").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ptz_info_lists_implemented_commands() {
        let response = get(app(camera()), "/axis-cgi/com/ptz.cgi?info=1").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
        assert_eq!(
            body(response).await,
            "Available commands:{camera=[n]}rpan=[offset]"
        );
    }

    #[tokio::test]
    async fn ptz_relative_pan_returns_no_content() {
        for uri in [
            "/axis-cgi/com/ptz.cgi?rpan=10",
            "/axis-cgi/com/ptz.cgi?rpan=-10.5&camera=1",
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
            assert_eq!(body(response).await, "");
        }
    }

    #[tokio::test]
    async fn ptz_errors_use_vapix_response_format() {
        for uri in [
            "/axis-cgi/com/ptz.cgi",
            "/axis-cgi/com/ptz.cgi?camera=2&rpan=10",
            "/axis-cgi/com/ptz.cgi?info=2",
            "/axis-cgi/com/ptz.cgi?info=1&rpan=10",
            "/axis-cgi/com/ptz.cgi?rpan=361",
            "/axis-cgi/com/ptz.cgi?rpan=invalid",
            "/axis-cgi/com/ptz.cgi?zoom=100",
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain", "{uri}");
            assert!(body(response).await.starts_with("Error:"), "{uri}");
        }
    }
}
