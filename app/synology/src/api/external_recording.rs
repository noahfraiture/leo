use axum::response::Response;
use serde::Serialize;

use super::{
    ApiError, CameraState,
    entry::{CameraId, EntryRequest, RecordingAction},
    success,
};

pub(super) const API: &str = "SYNO.SurveillanceStation.ExternalRecording";

#[derive(Serialize)]
struct ExternalRecordingResult {
    success: bool,
}

pub(super) async fn handle(
    cameras: CameraState,
    request: EntryRequest,
) -> Result<Response, ApiError> {
    if request.method != "Record" {
        return Err(ApiError::UnknownMethod);
    }
    if request.version != "2" {
        return Err(ApiError::UnsupportedVersion);
    }

    let (camera_id, recording) = match (request.camera_id, request.action) {
        (Some(CameraId::Valid(id)), Some(RecordingAction::Start)) => (id, true),
        (Some(CameraId::Valid(id)), Some(RecordingAction::Stop)) => (id, false),
        _ => return Err(ApiError::InvalidRecordingParameters),
    };
    let camera = cameras
        .lock()
        .map_err(|_| ApiError::Unknown)?
        .iter()
        .find(|camera| camera.id == camera_id)
        .cloned()
        .ok_or(ApiError::ExecutionFailed)?;
    if !camera.reachable().await {
        return Err(ApiError::ExecutionFailed);
    }

    let mut cameras = cameras.lock().map_err(|_| ApiError::Unknown)?;
    let camera = cameras
        .iter_mut()
        .find(|camera| camera.id == camera_id)
        .ok_or(ApiError::ExecutionFailed)?;
    camera.recording = recording;

    Ok(success(ExternalRecordingResult { success: true }))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::super::{
        CameraState, router,
        tests::{app, get, json_body},
    };
    use crate::camera::Camera;

    #[tokio::test]
    async fn starts_and_stops_one_camera() {
        let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cameras: CameraState = Arc::new(Mutex::new(vec![
            Camera::new(0, first.local_addr().unwrap()),
            Camera::new(1, second.local_addr().unwrap()),
        ]));
        let app = router().with_state(cameras.clone());

        get(
            app.clone(),
            "/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Record&version=2&cameraId=1&action=start",
        )
        .await;

        assert!(cameras.lock().unwrap()[0].recording);
        assert!(!cameras.lock().unwrap()[1].recording);

        get(
            app,
            "/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Record&version=2&cameraId=1&action=stop",
        )
        .await;

        assert!(!cameras.lock().unwrap()[0].recording);
    }

    #[tokio::test]
    async fn returns_documented_errors() {
        let reachable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let disconnected = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let disconnected_address = disconnected.local_addr().unwrap();
        drop(disconnected);
        let app = app(vec![
            Camera::new(0, reachable.local_addr().unwrap()),
            Camera::new(1, disconnected_address),
        ]);
        let base = "/webapi/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Record&version=2";

        for (suffix, code) in [
            ("&action=start", 401),
            ("&cameraId=nope&action=start", 401),
            ("&cameraId=1&action=nope", 401),
            ("&cameraId=99&action=start", 400),
            ("&cameraId=2&action=start", 400),
        ] {
            let response = get(app.clone(), &format!("{base}{suffix}")).await;

            assert_eq!(response.status(), StatusCode::OK, "{suffix}");
            assert_eq!(
                json_body(response).await,
                json!({"success": false, "error": {"code": code}}),
                "{suffix}"
            );
        }
    }
}
