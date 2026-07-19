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

    use super::super::tests::{get, json_body};
    use crate::server::app;

    #[tokio::test]
    async fn discovers_implemented_apis() {
        let response = get(
            app(vec![]),
            "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=1&query=ALL",
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await,
            json!({
                "success": true,
                "data": {
                    "SYNO.SurveillanceStation.Camera": {
                        "path": "entry.cgi",
                        "minVersion": 9,
                        "maxVersion": 9
                    },
                    "SYNO.SurveillanceStation.ExternalRecording": {
                        "path": "entry.cgi",
                        "minVersion": 2,
                        "maxVersion": 2
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn filters_exact_names_and_prefixes() {
        for query in [
            "SYNO.SurveillanceStation.Camera",
            "SYNO.SurveillanceStation.",
        ] {
            let response = get(
                app(vec![]),
                &format!(
                    "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=1&query={query}"
                ),
            )
            .await;
            let body = json_body(response).await;

            assert!(body["success"].as_bool().unwrap(), "{query}");
            assert!(
                body["data"]["SYNO.SurveillanceStation.Camera"].is_object(),
                "{query}"
            );
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
