use std::collections::BTreeMap;

use axum::{
    extract::{Query, rejection::QueryRejection},
    response::Response,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, success};

const API: &str = "SYNO.API.Info";

#[derive(Deserialize)]
pub(super) struct InfoRequest {
    pub api: String,
    pub method: String,
    pub version: String,
    pub query: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiDescription {
    path: &'static str,
    min_version: u8,
    max_version: u8,
}

pub(super) async fn handle(
    request: Result<Query<InfoRequest>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(request) = request?;
    if request.api != API {
        return Err(ApiError::UnknownApi);
    }
    if request.method != "Query" {
        return Err(ApiError::UnknownMethod);
    }
    if request.version != "1" {
        return Err(ApiError::UnsupportedVersion);
    }

    Ok(success(descriptions(&request.query)))
}

fn descriptions(query: &str) -> BTreeMap<&'static str, ApiDescription> {
    [
        (
            super::camera::API,
            ApiDescription {
                path: "entry.cgi",
                min_version: 9,
                max_version: 9,
            },
        ),
        (
            super::external_recording::API,
            ApiDescription {
                path: "entry.cgi",
                min_version: 2,
                max_version: 2,
            },
        ),
    ]
    .into_iter()
    .filter(|(name, _)| {
        query == "ALL"
            || query
                .split(',')
                .any(|unit| unit == *name || unit.ends_with('.') && name.starts_with(unit))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use serde_json::json;

    use super::super::tests::{app, get, json_body};

    #[tokio::test]
    async fn filters_api_descriptions() {
        for (query, expected) in [
            (
                "ALL",
                &[
                    "SYNO.SurveillanceStation.Camera",
                    "SYNO.SurveillanceStation.ExternalRecording",
                ][..],
            ),
            (
                "SYNO.SurveillanceStation.Camera",
                &["SYNO.SurveillanceStation.Camera"][..],
            ),
            (
                "SYNO.SurveillanceStation.",
                &[
                    "SYNO.SurveillanceStation.Camera",
                    "SYNO.SurveillanceStation.ExternalRecording",
                ][..],
            ),
            (
                "Missing,SYNO.SurveillanceStation.Camera",
                &["SYNO.SurveillanceStation.Camera"][..],
            ),
            ("Missing", &[][..]),
        ] {
            let response = get(
                app(vec![]),
                &format!(
                    "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=1&query={query}"
                ),
            )
            .await;
            let body = json_body(response).await;
            let data = body["data"].as_object().unwrap();

            assert!(body["success"].as_bool().unwrap(), "{query}");
            assert_eq!(data.len(), expected.len(), "{query}");
            for name in expected {
                assert!(data.contains_key(*name), "{query}: {name}");
            }
        }
    }

    #[tokio::test]
    async fn requires_every_field_and_preserves_error_precedence() {
        for uri in [
            "/webapi/query.cgi?method=Query&version=1&query=ALL",
            "/webapi/query.cgi?api=SYNO.API.Info&version=1&query=ALL",
            "/webapi/query.cgi?api=SYNO.API.Info&method=Query&query=ALL",
            "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=1",
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
                "/webapi/query.cgi?api=Missing&method=Missing&version=anything&query=ALL",
                102,
            ),
            (
                "/webapi/query.cgi?api=SYNO.API.Info&method=Missing&version=anything&query=ALL",
                103,
            ),
            (
                "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=anything&query=ALL",
                104,
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
