use axum::{
    extract::{Query, State, rejection::QueryRejection},
    response::Response,
};
use serde::{Deserialize, Deserializer};

use super::{ApiError, CameraState, camera, external_recording};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EntryRequest {
    pub api: String,
    pub method: String,
    pub version: String,
    pub camera_id: Option<CameraId>,
    pub action: Option<RecordingAction>,
}

pub(super) enum CameraId {
    Valid(u32),
    Invalid,
}

impl<'de> Deserialize<'de> for CameraId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(value.parse().map_or(Self::Invalid, Self::Valid))
    }
}

pub(super) enum RecordingAction {
    Start,
    Stop,
    Invalid,
}

impl<'de> Deserialize<'de> for RecordingAction {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(deserializer)?.as_str() {
            "start" => Self::Start,
            "stop" => Self::Stop,
            _ => Self::Invalid,
        })
    }
}

pub(super) async fn handle(
    State(cameras): State<CameraState>,
    request: Result<Query<EntryRequest>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(request) = request?;
    match request.api.as_str() {
        camera::API => camera::handle(cameras, request).await,
        external_recording::API => external_recording::handle(cameras, request).await,
        _ => Err(ApiError::UnknownApi),
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;

    use super::super::tests::{get, json_body};
    use crate::server::app;

    #[tokio::test]
    async fn requires_common_fields_and_preserves_error_precedence() {
        for uri in [
            "/webapi/entry.cgi?method=List&version=9",
            "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Camera&version=9",
            "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Camera&method=List",
        ] {
            let response = get(app(vec![]), uri).await;
            assert_eq!(
                json_body(response).await,
                json!({"success": false, "error": {"code": 101}}),
                "{uri}"
            );
        }

        for (uri, code) in [
            (
                "/webapi/entry.cgi?api=Missing&method=Missing&version=anything&cameraId=nope&action=nope",
                102,
            ),
            (
                "/webapi/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Missing&version=anything&cameraId=nope&action=nope",
                103,
            ),
            (
                "/webapi/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Record&version=anything&cameraId=nope&action=nope",
                104,
            ),
            (
                "/webapi/entry.cgi?api=SYNO.SurveillanceStation.ExternalRecording&method=Record&version=2&cameraId=nope&action=nope",
                401,
            ),
        ] {
            let response = get(app(vec![]), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(
                json_body(response).await,
                json!({"success": false, "error": {"code": code}}),
                "{uri}"
            );
        }
    }
}
