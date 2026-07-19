use std::sync::{Arc, Mutex};

use axum::{Json, Router, response::IntoResponse, routing::get};
use serde::Serialize;

use crate::camera::Camera;

mod camera;
mod entry;
mod error;
mod external_recording;
mod info;

use error::ApiError;

pub(crate) type CameraState = Arc<Mutex<Vec<Camera>>>;

pub(crate) fn router() -> Router<CameraState> {
    Router::new()
        .route("/query.cgi", get(info::handle))
        .route("/entry.cgi", get(entry::handle))
}

#[derive(Serialize)]
struct Success<T> {
    success: bool,
    data: T,
}

pub(super) fn success<T: Serialize>(data: T) -> axum::response::Response {
    Json(Success {
        success: true,
        data,
    })
    .into_response()
}

#[cfg(test)]
pub(super) mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::Request,
        response::Response,
    };
    use serde_json::Value;
    use tower::ServiceExt;

    pub async fn get(app: Router, uri: &str) -> Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    pub async fn json_body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }
}
