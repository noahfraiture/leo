mod camera;
pub mod cli;
mod server;
mod vapix;

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
    async fn ptz_relative_pan_returns_no_content() {
        for uri in [
            "/axis-cgi/com/ptz.cgi?rpan=10",
            "/axis-cgi/com/ptz.cgi?rpan=-10.5&camera=1",
            "/axis-cgi/com/ptz.cgi?rpan=-360",
            "/axis-cgi/com/ptz.cgi?rpan=360",
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
            assert_eq!(body(response).await, "");
        }
    }

    #[tokio::test]
    async fn ptz_errors_use_vapix_response_format() {
        for (uri, expected_body) in [
            ("/axis-cgi/com/ptz.cgi", "Error:Unsupported PTZ command"),
            (
                "/axis-cgi/com/ptz.cgi?camera=2",
                "Error:Only camera 1 is supported",
            ),
            (
                "/axis-cgi/com/ptz.cgi?camera=2&rpan=10",
                "Error:Only camera 1 is supported",
            ),
            (
                "/axis-cgi/com/ptz.cgi?camera=2&info=1",
                "Error:Only camera 1 is supported",
            ),
            ("/axis-cgi/com/ptz.cgi?info=2", "Error:info must be 1"),
            (
                "/axis-cgi/com/ptz.cgi?info=1&rpan=10",
                "Error:info cannot be combined with PTZ commands",
            ),
            (
                "/axis-cgi/com/ptz.cgi?rpan=-361",
                "Error:rpan must be between -360 and 360",
            ),
            (
                "/axis-cgi/com/ptz.cgi?rpan=361",
                "Error:rpan must be between -360 and 360",
            ),
            (
                "/axis-cgi/com/ptz.cgi?rpan=invalid",
                "Error:Invalid PTZ query",
            ),
            ("/axis-cgi/com/ptz.cgi?zoom=100", "Error:Invalid PTZ query"),
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain", "{uri}");
            assert_eq!(body(response).await, expected_body, "{uri}");
        }
    }
}
