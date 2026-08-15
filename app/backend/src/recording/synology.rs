use std::{ops::Range, path::Path, time::Duration};

use reqwest::Url;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

use crate::analysis::Video;

use super::error::{Error, Result};

/// Client for the supported Synology Surveillance Station recording endpoints.
pub(crate) struct SynologyClient {
    http: reqwest::Client,
    base_url: Url,
    sid: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    code: u32,
}

#[derive(Deserialize)]
struct LoginData {
    sid: String,
}

impl<T> ApiResponse<T> {
    fn into_data(self) -> Result<T> {
        if !self.success {
            let error = self
                .error
                .ok_or(Error::MissingResponseField { field: "error" })?;
            return Err(Error::Api { code: error.code });
        }

        self.data
            .ok_or(Error::MissingResponseField { field: "data" })
    }
}

#[derive(Deserialize)]
struct ListData {
    total: usize,
    events: Vec<ListEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListEvent {
    id: u64,
    camera_id: u32,
    start_time: i64,
    stop_time: i64,
}

impl SynologyClient {
    /// Creates an unauthenticated client rooted at one DSM origin.
    pub(crate) fn new(base_url: Url) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            sid: None,
        }
    }

    /// Opens one explicit Surveillance Station SID session for later requests.
    pub(crate) async fn login(&mut self, account: &str, password: &str) -> Result<()> {
        let mut url = self.base_url.clone();
        url.set_path("/webapi/auth.cgi");
        url.query_pairs_mut()
            .append_pair("api", "SYNO.API.Auth")
            .append_pair("method", "login")
            .append_pair("version", "2")
            .append_pair("account", account)
            .append_pair("passwd", password)
            .append_pair("session", "SurveillanceStation")
            .append_pair("format", "sid");
        let login = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json::<ApiResponse<LoginData>>()
            .await?
            .into_data()?;

        self.sid = Some(login.sid);
        Ok(())
    }

    /// Lists recordings intersecting the requested cameras and UTC time bounds.
    pub(crate) async fn list_videos(
        &self,
        camera_ids: &[u32],
        from_utc_ms: i64,
        to_utc_ms: i64,
    ) -> Result<Vec<Video>> {
        if from_utc_ms >= to_utc_ms {
            return Err(Error::InvalidListRange {
                from_utc_ms,
                to_utc_ms,
            });
        }
        let camera_ids = camera_ids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let from_time = from_utc_ms.div_euclid(1_000).to_string();
        let to_seconds = to_utc_ms.div_euclid(1_000);
        let to_seconds = if to_utc_ms.rem_euclid(1_000) == 0 {
            to_seconds
        } else {
            to_seconds + 1
        };
        let to_time = to_seconds.to_string();
        let mut offset = 0;
        let mut videos = Vec::new();

        loop {
            let mut url = self.base_url.clone();
            url.set_path("/webapi/entry.cgi");
            url.query_pairs_mut()
                .append_pair("api", "SYNO.SurveillanceStation.Recording")
                .append_pair("method", "List")
                .append_pair("version", "5")
                .append_pair("cameraIds", &camera_ids)
                .append_pair("fromTime", &from_time)
                .append_pair("toTime", &to_time)
                .append_pair("offset", &offset.to_string())
                .append_pair("limit", "100");
            if let Some(sid) = &self.sid {
                url.query_pairs_mut().append_pair("_sid", sid);
            }
            let response = self
                .http
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .json::<ApiResponse<ListData>>()
                .await?
                .into_data()?;
            let page_len = response.events.len();

            for event in response.events {
                if event.stop_time <= event.start_time {
                    return Err(Error::InvalidRecordingRange {
                        recording_id: event.id,
                        start_time: event.start_time,
                        stop_time: event.stop_time,
                    });
                }
                let start_utc_ms = event.start_time.checked_mul(1_000).ok_or(
                    Error::RecordingTimestampOverflow {
                        recording_id: event.id,
                        utc_seconds: event.start_time,
                    },
                )?;
                let end_utc_ms = event.stop_time.checked_mul(1_000).ok_or(
                    Error::RecordingTimestampOverflow {
                        recording_id: event.id,
                        utc_seconds: event.stop_time,
                    },
                )?;
                videos.push(Video {
                    recording_id: event.id,
                    camera_id: event.camera_id,
                    start_utc_ms,
                    end_utc_ms,
                });
            }
            if videos.len() >= response.total {
                videos
                    .sort_by_key(|video| (video.camera_id, video.start_utc_ms, video.recording_id));
                return Ok(videos);
            }
            if page_len == 0 {
                return Err(Error::IncompletePagination {
                    loaded: videos.len(),
                    total: response.total,
                });
            }
            offset += page_len;
        }
    }

    /// Streams one recording-relative range into a destination file.
    pub(crate) async fn download(
        &self,
        video: &Video,
        range: Range<Duration>,
        destination: &Path,
    ) -> Result<()> {
        let invalid_range = || Error::InvalidDownloadRange {
            recording_id: video.recording_id,
            start: range.start,
            end: range.end,
        };
        if video.end_utc_ms <= video.start_utc_ms || range.start >= range.end {
            return Err(invalid_range());
        }
        let duration_ms = video.end_utc_ms.abs_diff(video.start_utc_ms);
        let offset_ms = u64::try_from(range.start.as_millis()).map_err(|_| invalid_range())?;
        let requested_end_ms =
            u64::try_from(range.end.as_nanos().div_ceil(1_000_000)).map_err(|_| invalid_range())?;
        let end_ms = requested_end_ms.min(duration_ms);
        if offset_ms >= end_ms {
            return Err(invalid_range());
        }

        let mut url = self.base_url.clone();
        url.set_path("/webapi/entry.cgi");
        url.query_pairs_mut()
            .append_pair("api", "SYNO.SurveillanceStation.Recording")
            .append_pair("method", "Download")
            .append_pair("version", "6")
            .append_pair("id", &video.recording_id.to_string())
            .append_pair("offsetTimeMs", &offset_ms.to_string())
            .append_pair("playTimeMs", &(end_ms - offset_ms).to_string());
        if let Some(sid) = &self.sid {
            url.query_pairs_mut().append_pair("_sid", sid);
        }

        let mut response = self.http.get(url).send().await?;
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                status: response.status(),
            });
        }
        if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.starts_with("application/json") || value.starts_with("text/")
            })
        {
            return match response
                .json::<ApiResponse<serde_json::Value>>()
                .await?
                .into_data()
            {
                Err(error) => Err(error),
                Ok(_) => Err(Error::UnexpectedJsonDownload),
            };
        }

        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::NamedTempFile::new_in(parent)?;
        let mut file = tokio::fs::File::from_std(temporary.reopen()?);
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        temporary
            .persist(destination)
            .map_err(|error| Error::Io(error.error))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, VecDeque},
        convert::Infallible,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use axum::{
        Json, Router,
        body::Body,
        extract::{Query, State},
        http::{StatusCode, header::CONTENT_TYPE},
        response::{IntoResponse, Response},
        routing::get,
    };
    use futures_util::{StreamExt, stream};
    use reqwest::Url;
    use serde_json::{Value, json};
    use tokio::net::TcpListener;

    use crate::{analysis::Video, recording::Error};

    use super::SynologyClient;

    #[derive(Clone)]
    struct ServerState {
        responses: Arc<Mutex<VecDeque<ServerResponse>>>,
        queries: Arc<Mutex<Vec<HashMap<String, String>>>>,
    }

    enum ServerResponse {
        Json(Value),
        PlainJson(Value),
        Media(Vec<Vec<u8>>),
        MediaError(Vec<u8>),
        Status(StatusCode),
    }

    impl From<Value> for ServerResponse {
        fn from(value: Value) -> Self {
            Self::Json(value)
        }
    }

    async fn entry(
        State(state): State<ServerState>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Response {
        state.queries.lock().unwrap().push(query);
        match state.responses.lock().unwrap().pop_front().unwrap() {
            ServerResponse::Json(value) => Json(value).into_response(),
            ServerResponse::PlainJson(value) => Response::builder()
                .header(CONTENT_TYPE, "text/plain; charset=UTF-8")
                .body(Body::from(value.to_string()))
                .unwrap(),
            ServerResponse::Media(chunks) => {
                let chunks = stream::iter(chunks.into_iter().map(Ok::<_, Infallible>));
                Response::builder()
                    .header(CONTENT_TYPE, "video/mp4")
                    .body(Body::from_stream(chunks))
                    .unwrap()
            }
            ServerResponse::MediaError(prefix) => {
                let chunks = stream::once(async move { Ok::<_, std::io::Error>(prefix) }).chain(
                    stream::once(async {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        Err::<Vec<u8>, _>(std::io::Error::other("response body failed"))
                    }),
                );
                Response::builder()
                    .header(CONTENT_TYPE, "video/mp4")
                    .body(Body::from_stream(chunks))
                    .unwrap()
            }
            ServerResponse::Status(status) => status.into_response(),
        }
    }

    async fn spawn_server<T>(responses: Vec<T>) -> (Url, Arc<Mutex<Vec<HashMap<String, String>>>>)
    where
        T: Into<ServerResponse>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let queries = Arc::new(Mutex::new(Vec::new()));
        let state = ServerState {
            responses: Arc::new(Mutex::new(responses.into_iter().map(Into::into).collect())),
            queries: Arc::clone(&queries),
        };
        let app = Router::new()
            .route("/webapi/entry.cgi", get(entry))
            .route("/webapi/auth.cgi", get(entry))
            .with_state(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        (Url::parse(&format!("http://{address}")).unwrap(), queries)
    }

    #[tokio::test]
    async fn lists_primary_events_response() {
        let (base_url, queries) = spawn_server(vec![json!({
            "success": true,
            "data": {
                "total": 1,
                "events": [{
                    "id": 10334,
                    "cameraId": 26,
                    "startTime": 1_623_036_368,
                    "stopTime": 1_623_036_369
                }]
            }
        })])
        .await;

        let videos = SynologyClient::new(base_url)
            .list_videos(&[26], 1_623_036_367_000, 1_623_036_370_000)
            .await
            .unwrap();

        assert_eq!(
            videos,
            vec![Video {
                recording_id: 10334,
                camera_id: 26,
                start_utc_ms: 1_623_036_368_000,
                end_utc_ms: 1_623_036_369_000,
            }]
        );
        let queries = queries.lock().unwrap();
        let query = &queries[0];
        assert_eq!(query["api"], "SYNO.SurveillanceStation.Recording");
        assert_eq!(query["method"], "List");
        assert_eq!(query["version"], "5");
        assert_eq!(query["cameraIds"], "26");
        assert_eq!(query["fromTime"], "1623036367");
        assert_eq!(query["toTime"], "1623036370");
        assert_eq!(query["offset"], "0");
        assert!(query.contains_key("limit"));
        assert!(!query.contains_key("_sid"));
    }

    #[tokio::test]
    async fn lists_all_recording_pages() {
        let (base_url, queries) = spawn_server(vec![
            json!({
                "success": true,
                "data": {
                    "total": 3,
                    "events": [
                        {"id": 10, "cameraId": 1, "startTime": 100, "stopTime": 110},
                        {"id": 11, "cameraId": 1, "startTime": 110, "stopTime": 120}
                    ]
                }
            }),
            json!({
                "success": true,
                "data": {
                    "total": 3,
                    "events": [
                        {"id": 12, "cameraId": 1, "startTime": 120, "stopTime": 130}
                    ]
                }
            }),
        ])
        .await;

        let videos = SynologyClient::new(base_url)
            .list_videos(&[1], 100_000, 130_000)
            .await
            .unwrap();

        assert_eq!(
            videos
                .iter()
                .map(|video| video.recording_id)
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0]["offset"], "0");
        assert_eq!(queries[1]["offset"], "2");
    }

    #[tokio::test]
    async fn rejects_incomplete_pagination() {
        let (base_url, queries) = spawn_server(vec![json!({
            "success": true,
            "data": {"total": 1, "events": []}
        })])
        .await;

        let error = SynologyClient::new(base_url)
            .list_videos(&[1], 100_000, 130_000)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::IncompletePagination {
                loaded: 0,
                total: 1
            }
        ));
        assert_eq!(queries.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sorts_recordings_by_camera_start_and_id() {
        let (base_url, _) = spawn_server(vec![json!({
            "success": true,
            "data": {
                "total": 5,
                "events": [
                    {"id": 30, "cameraId": 2, "startTime": 300, "stopTime": 310},
                    {"id": 12, "cameraId": 1, "startTime": 200, "stopTime": 210},
                    {"id": 11, "cameraId": 1, "startTime": 200, "stopTime": 210},
                    {"id": 10, "cameraId": 1, "startTime": 100, "stopTime": 110},
                    {"id": 20, "cameraId": 2, "startTime": 100, "stopTime": 110}
                ]
            }
        })])
        .await;

        let videos = SynologyClient::new(base_url)
            .list_videos(&[2, 1], 100_000, 310_000)
            .await
            .unwrap();

        assert_eq!(
            videos
                .iter()
                .map(|video| video.recording_id)
                .collect::<Vec<_>>(),
            vec![10, 11, 12, 20, 30]
        );
    }

    #[tokio::test]
    async fn rejects_malformed_recording_range() {
        let (base_url, _) = spawn_server(vec![json!({
            "success": true,
            "data": {
                "total": 1,
                "events": [
                    {"id": 46, "cameraId": 8, "startTime": 200, "stopTime": 100}
                ]
            }
        })])
        .await;

        let error = SynologyClient::new(base_url)
            .list_videos(&[8], 100_000, 200_000)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::InvalidRecordingRange {
                recording_id: 46,
                start_time: 200,
                stop_time: 100
            }
        ));
    }

    #[tokio::test]
    async fn rejects_recording_timestamp_overflow() {
        let start_time = i64::MAX / 1_000;
        let stop_time = start_time + 1;
        let (base_url, _) = spawn_server(vec![json!({
            "success": true,
            "data": {
                "total": 1,
                "events": [{
                    "id": 46,
                    "cameraId": 8,
                    "startTime": start_time,
                    "stopTime": stop_time
                }]
            }
        })])
        .await;

        let error = SynologyClient::new(base_url)
            .list_videos(&[8], 100_000, 200_000)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::RecordingTimestampOverflow {
                recording_id: 46,
                utc_seconds
            } if utc_seconds == stop_time
        ));
    }

    #[tokio::test]
    async fn returns_synology_list_error() {
        let (base_url, _) = spawn_server(vec![json!({
            "success": false,
            "error": {"code": 401}
        })])
        .await;

        let error = SynologyClient::new(base_url)
            .list_videos(&[8], 100_000, 200_000)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Api { code: 401 }));
    }

    #[tokio::test]
    async fn rejects_invalid_list_range_without_request() {
        let (base_url, queries) = spawn_server(vec![json!({
            "success": true,
            "data": {"total": 0, "events": []}
        })])
        .await;

        let error = SynologyClient::new(base_url)
            .list_videos(&[8], 200_000, 100_000)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::InvalidListRange {
                from_utc_ms: 200_000,
                to_utc_ms: 100_000
            }
        ));
        assert!(queries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn catalogue_subsecond_bounds_round_outward() {
        let (base_url, queries) = spawn_server(vec![json!({
            "success": true,
            "data": {"total": 0, "events": []}
        })])
        .await;

        SynologyClient::new(base_url)
            .list_videos(&[26], -1, 1)
            .await
            .unwrap();

        let queries = queries.lock().unwrap();
        assert_eq!(queries[0]["fromTime"], "-1");
        assert_eq!(queries[0]["toTime"], "1");
    }

    #[tokio::test]
    async fn login_adds_sid_to_later_requests() {
        let (base_url, queries) = spawn_server(vec![
            json!({"success": true, "data": {"sid": "test-sid"}}),
            json!({"success": true, "data": {"total": 0, "events": []}}),
        ])
        .await;
        let mut client = SynologyClient::new(base_url);

        client.login("operator", "password").await.unwrap();
        client.list_videos(&[8], 100_000, 200_000).await.unwrap();

        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0]["api"], "SYNO.API.Auth");
        assert_eq!(queries[0]["method"], "login");
        assert_eq!(queries[0]["version"], "2");
        assert_eq!(queries[0]["account"], "operator");
        assert_eq!(queries[0]["passwd"], "password");
        assert_eq!(queries[0]["session"], "SurveillanceStation");
        assert_eq!(queries[0]["format"], "sid");
        assert!(!queries[0].contains_key("_sid"));
        assert_eq!(queries[1]["_sid"], "test-sid");
    }

    #[tokio::test]
    async fn login_errors_do_not_retain_password_url() {
        let (base_url, _) =
            spawn_server(vec![ServerResponse::Status(StatusCode::UNAUTHORIZED)]).await;

        let error = SynologyClient::new(base_url)
            .login("operator", "do-not-retain")
            .await
            .unwrap_err();

        match error {
            Error::Http(source) => assert!(source.url().is_none()),
            other => panic!("expected HTTP error, found {other:?}"),
        }
    }

    #[tokio::test]
    async fn catalogue_http_errors_do_not_retain_sid_url() {
        const SID: &str = "do-not-retain-sid";
        let (base_url, _) = spawn_server(vec![
            ServerResponse::Json(json!({"success": true, "data": {"sid": SID}})),
            ServerResponse::Status(StatusCode::SERVICE_UNAVAILABLE),
        ])
        .await;
        let mut client = SynologyClient::new(base_url);
        client.login("operator", "password").await.unwrap();

        let error = client
            .list_videos(&[8], 100_000, 200_000)
            .await
            .unwrap_err();

        let Error::Http(source) = &error else {
            panic!("expected HTTP error, found {error:?}");
        };
        assert!(
            source.url().is_none(),
            "HTTP error retained credential-bearing URL: {source:?}"
        );
        assert!(!format!("{error:?}").contains(SID));
        assert!(!error.to_string().contains(SID));
    }

    #[tokio::test]
    async fn streams_clamped_recording_range_to_file() {
        let (base_url, queries) = spawn_server(vec![ServerResponse::Media(vec![
            b"first-".to_vec(),
            b"second-".to_vec(),
            b"third".to_vec(),
        ])])
        .await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };

        SynologyClient::new(base_url)
            .download(
                &video,
                Duration::from_secs(2)..Duration::from_secs(12),
                &destination,
            )
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read(destination).await.unwrap(),
            b"first-second-third"
        );
        let queries = queries.lock().unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0]["api"], "SYNO.SurveillanceStation.Recording");
        assert_eq!(queries[0]["method"], "Download");
        assert_eq!(queries[0]["version"], "6");
        assert_eq!(queries[0]["id"], "46");
        assert_eq!(queries[0]["offsetTimeMs"], "2000");
        assert_eq!(queries[0]["playTimeMs"], "8000");
        assert!(!queries[0].contains_key("_sid"));
    }

    #[tokio::test]
    async fn download_submillisecond_bounds_round_outward() {
        let (base_url, queries) =
            spawn_server(vec![ServerResponse::Media(vec![b"media".to_vec()])]).await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_000_010,
        };

        SynologyClient::new(base_url)
            .download(
                &video,
                Duration::from_nanos(1_999_999)..Duration::from_nanos(2_000_001),
                &destination,
            )
            .await
            .unwrap();

        let queries = queries.lock().unwrap();
        assert_eq!(queries[0]["offsetTimeMs"], "1");
        assert_eq!(queries[0]["playTimeMs"], "2");
    }

    #[tokio::test]
    async fn rejects_invalid_download_range_without_request() {
        let (base_url, queries) =
            spawn_server(vec![ServerResponse::Media(vec![b"unused".to_vec()])]).await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };

        let error = SynologyClient::new(base_url)
            .download(
                &video,
                Duration::from_secs(10)..Duration::from_secs(11),
                &destination,
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::InvalidDownloadRange {
                recording_id: 46,
                ..
            }
        ));
        assert!(queries.lock().unwrap().is_empty());
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn rejects_reversed_submillisecond_download_range() {
        let (base_url, queries) =
            spawn_server(vec![ServerResponse::Media(vec![b"unused".to_vec()])]).await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_000_010,
        };

        let error = SynologyClient::new(base_url)
            .download(
                &video,
                Duration::from_nanos(1_999_999)..Duration::from_nanos(1_000_001),
                &destination,
            )
            .await
            .unwrap_err();

        assert!(matches!(error, Error::InvalidDownloadRange { .. }));
        assert!(queries.lock().unwrap().is_empty());
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn returns_destination_write_error() {
        let (base_url, _) =
            spawn_server(vec![ServerResponse::Media(vec![b"media".to_vec()])]).await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("missing/recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };

        let error = SynologyClient::new(base_url)
            .download(&video, Duration::ZERO..Duration::from_secs(1), &destination)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Io(_)));
    }

    #[tokio::test]
    async fn stream_error_preserves_existing_and_absent_destinations() {
        let (base_url, _) = spawn_server(vec![
            ServerResponse::MediaError(b"partial".to_vec()),
            ServerResponse::MediaError(b"partial".to_vec()),
        ])
        .await;
        let destination_dir = tempfile::tempdir().unwrap();
        let existing = destination_dir.path().join("existing.mp4");
        let absent = destination_dir.path().join("absent.mp4");
        tokio::fs::write(&existing, b"original").await.unwrap();
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };
        let client = SynologyClient::new(base_url);

        assert!(
            client
                .download(&video, Duration::ZERO..Duration::from_secs(1), &existing)
                .await
                .is_err()
        );
        assert!(
            client
                .download(&video, Duration::ZERO..Duration::from_secs(1), &absent)
                .await
                .is_err()
        );

        assert_eq!(tokio::fs::read(existing).await.unwrap(), b"original");
        assert!(!absent.exists());
    }

    #[tokio::test]
    async fn returns_non_success_download_status() {
        let (base_url, _) = spawn_server(vec![ServerResponse::Status(
            StatusCode::SERVICE_UNAVAILABLE,
        )])
        .await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };

        let error = SynologyClient::new(base_url)
            .download(&video, Duration::ZERO..Duration::from_secs(1), &destination)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            Error::HttpStatus {
                status: StatusCode::SERVICE_UNAVAILABLE
            }
        ));
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn returns_synology_download_error() {
        let (base_url, _) = spawn_server(vec![ServerResponse::PlainJson(json!({
            "success": false,
            "error": {"code": 414}
        }))])
        .await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };

        let error = SynologyClient::new(base_url)
            .download(&video, Duration::ZERO..Duration::from_secs(1), &destination)
            .await
            .unwrap_err();

        assert!(matches!(error, Error::Api { code: 414 }));
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn download_json_errors_do_not_retain_sid_url() {
        const SID: &str = "do-not-retain-sid";
        let (base_url, _) = spawn_server(vec![
            ServerResponse::Json(json!({"success": true, "data": {"sid": SID}})),
            ServerResponse::PlainJson(json!({"success": "not-a-boolean"})),
        ])
        .await;
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = destination_dir.path().join("recording.mp4");
        let video = Video {
            recording_id: 46,
            camera_id: 8,
            start_utc_ms: 1_000_000,
            end_utc_ms: 1_010_000,
        };
        let mut client = SynologyClient::new(base_url);
        client.login("operator", "password").await.unwrap();

        let error = client
            .download(&video, Duration::ZERO..Duration::from_secs(1), &destination)
            .await
            .unwrap_err();

        let Error::Http(source) = &error else {
            panic!("expected HTTP error, found {error:?}");
        };
        assert!(
            source.url().is_none(),
            "HTTP error retained credential-bearing URL: {source:?}"
        );
        assert!(!format!("{error:?}").contains(SID));
        assert!(!error.to_string().contains(SID));
    }
}
