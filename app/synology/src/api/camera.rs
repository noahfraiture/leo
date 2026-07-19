use axum::response::Response;
use serde::Serialize;

use super::{ApiError, CameraState, entry::EntryRequest, success};

pub(super) const API: &str = "SYNO.SurveillanceStation.Camera";

#[derive(Serialize)]
struct CameraList {
    total: usize,
    cameras: Vec<CameraInfo>,
}

#[derive(Serialize)]
struct CameraInfo {
    id: u32,
    name: String,
    ip: String,
    port: u16,
    status: u8,
    vendor: &'static str,
    model: &'static str,
    channel: &'static str,
}

pub(super) async fn handle(
    cameras: CameraState,
    request: EntryRequest,
) -> Result<Response, ApiError> {
    if request.method != "List" {
        return Err(ApiError::UnknownMethod);
    }
    if request.version != "9" {
        return Err(ApiError::UnsupportedVersion);
    }

    let cameras = cameras.lock().map_err(|_| ApiError::Unknown)?.clone();
    let mut response = Vec::with_capacity(cameras.len());

    // ponytail: sequential probes are enough for 3-5 cameras; parallelize if latency matters.
    for camera in cameras {
        let status = if camera.reachable().await { 1 } else { 3 };
        response.push(CameraInfo {
            id: camera.id,
            name: camera.name,
            ip: camera.address.ip().to_string(),
            port: camera.address.port(),
            status,
            vendor: "AXIS",
            model: "P3278-LV",
            channel: "1",
        });
    }

    Ok(success(CameraList {
        total: response.len(),
        cameras: response,
    }))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::super::tests::{get, json_body};
    use crate::{camera::Camera, server::app};

    #[tokio::test]
    async fn list_reports_network_reachability() {
        let reachable = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let reachable_address = reachable.local_addr().unwrap();
        let disconnected = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let disconnected_address = disconnected.local_addr().unwrap();
        drop(disconnected);

        let response = get(
            app(vec![
                Camera::new(0, reachable_address),
                Camera::new(1, disconnected_address),
            ]),
            "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Camera&method=List&version=9",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await,
            json!({
                "success": true,
                "data": {
                    "total": 2,
                    "cameras": [
                        {
                            "id": 1,
                            "name": "camera-1",
                            "ip": "127.0.0.1",
                            "port": reachable_address.port(),
                            "status": 1,
                            "vendor": "AXIS",
                            "model": "P3278-LV",
                            "channel": "1"
                        },
                        {
                            "id": 2,
                            "name": "camera-2",
                            "ip": "127.0.0.1",
                            "port": disconnected_address.port(),
                            "status": 3,
                            "vendor": "AXIS",
                            "model": "P3278-LV",
                            "channel": "1"
                        }
                    ]
                }
            })
        );
    }
}
