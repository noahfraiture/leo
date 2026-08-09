use std::{str::FromStr, time::SystemTime};

use axum::{
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use tokio::{fs, process::Command};

use super::{ApiError, CameraState, entry::EntryRequest, success};

/// Official Surveillance Station Recording API name.
pub(super) const API: &str = "SYNO.SurveillanceStation.Recording";

/// Primary List v6 response containing catalogue records for one DS.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingList<T> {
    ds_id: u32,
    total: usize,
    recordings: Vec<T>,
}

/// One List v6 catalogue record, intentionally without timing fields.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordingInfo<'a> {
    id: u32,
    video_codec: u8,
    audio_codec: u8,
    height: u32,
    width: u32,
    camera_id: u32,
    camera_name: &'a str,
    size_byte: u64,
    file_path: &'a str,
    locked: bool,
}

/// Compatibility List v5 response containing timestamp-bearing events.
#[derive(Serialize)]
struct EventList<T> {
    offset: usize,
    total: usize,
    timestamp: u64,
    events: Vec<T>,
}

/// One List v5 event projected from a validated fixture recording.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Event<'a> {
    arch_id: u32,
    audio_codec: &'static str,
    bookmark: [(); 0],
    bookmark_count: u8,
    camera_id: u32,
    ds_id: u32,
    folder: &'a str,
    id: u32,
    img_height: u32,
    img_width: u32,
    start_time: u64,
    stop_time: u64,
    video_codec: &'static str,
}

/// Dispatches Recording List v5/v6 and Download v6 requests.
pub(super) async fn handle(
    cameras: CameraState,
    request: EntryRequest,
) -> Result<Response, ApiError> {
    match request.method.as_str() {
        "List" => {}
        "Download" => {
            if request.version != "6" {
                return Err(ApiError::UnsupportedVersion);
            }
            return download(cameras, request).await;
        }
        _ => return Err(ApiError::UnknownMethod),
    }
    let version = match request.version.as_str() {
        "5" => 5,
        "6" => 6,
        _ => return Err(ApiError::UnsupportedVersion),
    };

    let offset: usize = parse(request.offset.as_deref())?.unwrap_or_default();
    let limit: Option<usize> = parse(request.limit.as_deref())?;
    let from_time: u64 = parse(request.from_time.as_deref())?.unwrap_or_default();
    let to_time: u64 = parse(request.to_time.as_deref())?.unwrap_or_default();
    let ds_id: Option<u32> = parse(request.ds_id.as_deref())?;
    let effective_ds_id = ds_id.unwrap_or_default();
    let mount_id: Option<u32> = parse(request.mount_id.as_deref())?;
    let camera_ids: Option<Vec<u32>> = request
        .camera_ids
        .as_deref()
        .map(|ids| ids.split(',').map(str::parse).collect())
        .transpose()
        .map_err(|_| ApiError::InvalidRecordingParameters)?;

    let cameras = cameras.lock().map_err(|_| ApiError::Unknown)?;
    let mut recordings = cameras
        .iter()
        .flat_map(|camera| {
            camera
                .recordings
                .iter()
                .map(move |recording| (camera, recording))
        })
        .filter(|(_, recording)| {
            camera_ids
                .as_ref()
                .is_none_or(|ids: &Vec<u32>| ids.contains(&recording.camera_id))
                && if version == 6 {
                    recording.ds_id == effective_ds_id
                } else {
                    ds_id.is_none_or(|id| recording.ds_id == id)
                }
                && mount_id.is_none_or(|id| recording.mount_id == id)
                && (from_time == 0 || recording.stop_time > from_time)
                && (to_time == 0 || recording.start_time < to_time)
        })
        .collect::<Vec<_>>();
    recordings
        .sort_by_key(|(_, recording)| (recording.start_time, recording.camera_id, recording.id));

    let total = recordings.len();
    let recordings = recordings
        .into_iter()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX));
    match version {
        5 => {
            let events = recordings
                .map(|(_, recording)| {
                    let (video_codec, audio_codec) =
                        codec_names(recording.video_codec, recording.audio_codec);
                    Event {
                        arch_id: 0,
                        audio_codec,
                        bookmark: [],
                        bookmark_count: 0,
                        camera_id: recording.camera_id,
                        ds_id: recording.ds_id,
                        folder: recording
                            .file_path
                            .rsplit_once('/')
                            .map_or("", |(folder, _)| folder),
                        id: recording.id,
                        img_height: recording.height,
                        img_width: recording.width,
                        start_time: recording.start_time,
                        stop_time: recording.stop_time,
                        video_codec,
                    }
                })
                .collect();
            let timestamp = SystemTime::UNIX_EPOCH
                .elapsed()
                .map_err(|_| ApiError::Unknown)?
                .as_secs();
            Ok(success(EventList {
                offset,
                total,
                timestamp,
                events,
            }))
        }
        6 => Ok(success(RecordingList {
            ds_id: effective_ds_id,
            total,
            recordings: recordings
                .map(|(camera, recording)| RecordingInfo {
                    id: recording.id,
                    video_codec: recording.video_codec,
                    audio_codec: recording.audio_codec,
                    height: recording.height,
                    width: recording.width,
                    camera_id: recording.camera_id,
                    camera_name: &camera.name,
                    size_byte: recording.size_byte,
                    file_path: &recording.file_path,
                    locked: recording.locked,
                })
                .collect(),
        })),
        _ => unreachable!(),
    }
}

/// Returns a complete fixture or creates a bounded temporary MP4 clip.
async fn download(cameras: CameraState, request: EntryRequest) -> Result<Response, ApiError> {
    let id: u32 = parse(request.id.as_deref())?.unwrap_or_default();
    let mount_id: u32 = parse(request.mount_id.as_deref())?.unwrap_or_default();
    let offset: u64 = parse(request.offset_time_ms.as_deref())?.unwrap_or_default();
    let play_time: Option<u64> = parse(request.play_time_ms.as_deref())?;
    let recording = {
        let cameras = cameras.lock().map_err(|_| ApiError::Unknown)?;
        cameras
            .iter()
            .flat_map(|camera| &camera.recordings)
            .find(|recording| recording.id == id && recording.mount_id == mount_id)
            .cloned()
            .ok_or(ApiError::UnknownRecording)?
    };

    let duration = (recording.stop_time - recording.start_time)
        .checked_mul(1000)
        .ok_or(ApiError::InvalidRecordingParameters)?;
    if offset >= duration {
        return Err(ApiError::InvalidRecordingParameters);
    }
    let play_time = play_time.unwrap_or(duration - offset);
    if play_time == 0
        || offset
            .checked_add(play_time)
            .is_none_or(|end| end > duration)
    {
        return Err(ApiError::InvalidRecordingParameters);
    }

    let bytes = if offset == 0 && play_time == duration {
        fs::read(&recording.video_path)
            .await
            .map_err(|_| ApiError::ExecutionFailed)?
    } else {
        let directory = tempfile::TempDir::new().map_err(|_| ApiError::ExecutionFailed)?;
        let output_path = directory.path().join("recording.mp4");
        let output = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss"])
            .arg(format!("{offset}ms"))
            .arg("-i")
            .arg(&recording.video_path)
            .arg("-t")
            .arg(format!("{play_time}ms"))
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-c:v",
                "copy",
                "-movflags",
                "+faststart",
            ])
            .arg(&output_path)
            .output()
            .await
            .map_err(|_| ApiError::ExecutionFailed)?;
        if !output.status.success() {
            return Err(ApiError::ExecutionFailed);
        }
        fs::read(output_path)
            .await
            .map_err(|_| ApiError::ExecutionFailed)?
    };

    Ok(([(CONTENT_TYPE, "video/mp4")], bytes).into_response())
}

fn parse<T: FromStr>(value: Option<&str>) -> Result<Option<T>, ApiError> {
    value
        .map(str::parse)
        .transpose()
        .map_err(|_| ApiError::InvalidRecordingParameters)
}

/// Converts numeric codec identifiers into the strings used by List v5.
fn codec_names(video: u8, audio: u8) -> (&'static str, &'static str) {
    let video = match video {
        0 => "Unknown",
        1 => "MJPEG",
        2 => "MPEG4",
        3 => "H.264",
        5 => "MXPEG",
        6 => "H.265",
        7 => "H.264+",
        _ => "Unknown",
    };
    let audio = match audio {
        0 => "",
        1 => "PCM",
        2 => "G711",
        3 => "G726",
        4 => "AAC",
        5 => "AMR",
        6 => "UserDefine",
        _ => "Unknown",
    };
    (video, audio)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        net::SocketAddr,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use axum::{
        body::to_bytes,
        http::{StatusCode, header::CONTENT_TYPE},
    };
    use serde_json::{Value, json};

    use super::super::tests::{app, get, json_body};
    use crate::{camera::Camera, recording::Recording};

    const LIST: &str = "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Recording&method=List";
    const DOWNLOAD: &str =
        "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Recording&method=Download&version=6";

    fn recording(
        id: u32,
        camera_id: u32,
        ds_id: u32,
        mount_id: u32,
        start_time: u64,
        stop_time: u64,
        file_path: &str,
    ) -> Recording {
        Recording {
            id,
            camera_id,
            ds_id,
            mount_id,
            start_time,
            stop_time,
            file_path: file_path.into(),
            video_path: PathBuf::from("private/wrong-folder/video.mp4"),
            video_codec: 3,
            audio_codec: 0,
            width: 1920,
            height: 1080,
            size_byte: u64::from(id) * 10,
            locked: id.is_multiple_of(2),
        }
    }

    fn catalogue() -> Vec<Camera> {
        let mut first = Camera::new(0, SocketAddr::from(([127, 0, 0, 1], 8001)));
        first.recordings = vec![
            recording(14, 1, 0, 20, 300, 400, "other/day/camera-1-300.mp4"),
            recording(11, 1, 1, 10, 100, 200, "logical/day-one/camera-1-100.mp4"),
        ];
        let mut second = Camera::new(1, SocketAddr::from(([127, 0, 0, 1], 8002)));
        second.recordings = vec![
            recording(13, 2, 1, 11, 200, 300, "logical/day-two/camera-2-200.mp4"),
            recording(12, 2, 1, 10, 100, 150, "logical/day-one/camera-2-100.mp4"),
        ];
        vec![first, second]
    }

    fn download_catalogue(video_path: &Path) -> Vec<Camera> {
        let mut camera = Camera::new(0, SocketAddr::from(([127, 0, 0, 1], 8001)));
        let mut fixture = recording(11, 1, 0, 0, 100, 105, "logical/day-one/camera-1-100.mp4");
        fixture.video_path = video_path.to_path_buf();
        camera.recordings = vec![fixture];
        vec![camera]
    }

    fn unix_time() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    async fn list(suffix: &str) -> Value {
        json_body(get(app(catalogue()), &format!("{LIST}&version=6{suffix}")).await).await
    }

    async fn listed_ids(suffix: &str) -> (u64, Vec<u64>) {
        let body = list(suffix).await;
        let data = &body["data"];
        (
            data["total"].as_u64().unwrap(),
            data["recordings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|recording| recording["id"].as_u64().unwrap())
                .collect(),
        )
    }

    async fn assert_download_error(cameras: Vec<Camera>, uri: &str, code: u16) {
        let response = get(app(cameras), uri).await;
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        assert_eq!(
            json_body(response).await,
            json!({"success": false, "error": {"code": code}}),
            "{uri}"
        );
    }

    #[tokio::test]
    async fn downloads_full_recording_bytes_on_plain_and_filename_routes() {
        let directory = tempfile::tempdir().unwrap();
        let video_path = directory.path().join("fixture.mp4");
        let fixture = b"\0fixture mp4 bytes\xff";
        fs::write(&video_path, fixture).unwrap();

        for uri in [
            format!("{DOWNLOAD}&id=11"),
            "/webapi/entry.cgi/camera-1.mp4?api=SYNO.SurveillanceStation.Recording&method=Download&version=6&id=11"
                .to_owned(),
        ] {
            let response = get(app(download_catalogue(&video_path)), &uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "video/mp4",
                "{uri}"
            );
            assert_eq!(
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                fixture.as_slice(),
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn download_defaults_missing_id_and_mount_and_reports_unknown_recordings() {
        let directory = tempfile::tempdir().unwrap();
        let video_path = directory.path().join("fixture.mp4");
        fs::write(&video_path, b"fixture").unwrap();

        for uri in [
            DOWNLOAD.to_owned(),
            format!("{DOWNLOAD}&id=0"),
            format!("{DOWNLOAD}&id=99"),
            format!("{DOWNLOAD}&id=11&mountId=1"),
            "/webapi/entry.cgi/file.mp4?api=SYNO.SurveillanceStation.Recording&method=Download&version=6"
                .to_owned(),
        ] {
            assert_download_error(download_catalogue(&video_path), &uri, 414).await;
        }
    }

    #[tokio::test]
    async fn download_rejects_malformed_and_invalid_ranges() {
        let directory = tempfile::tempdir().unwrap();
        let video_path = directory.path().join("fixture.mp4");
        fs::write(&video_path, b"fixture").unwrap();

        for fields in [
            "id=nope",
            "mountId=nope&id=11",
            "offsetTimeMs=nope&id=11",
            "playTimeMs=nope&id=11",
            "playTimeMs=0&id=11",
            "offsetTimeMs=5000&id=11",
            "offsetTimeMs=4000&playTimeMs=1001&id=11",
        ] {
            let uri = format!("{DOWNLOAD}&{fields}");
            assert_download_error(download_catalogue(&video_path), &uri, 401).await;
        }
    }

    #[tokio::test]
    async fn download_maps_missing_media_and_ffmpeg_failure_to_execution_failed() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.mp4");
        assert_download_error(
            download_catalogue(&missing),
            &format!("{DOWNLOAD}&id=11"),
            400,
        )
        .await;

        let invalid = directory.path().join("invalid.mp4");
        fs::write(&invalid, b"not an mp4").unwrap();
        assert_download_error(
            download_catalogue(&invalid),
            &format!("{DOWNLOAD}&id=11&offsetTimeMs=1000&playTimeMs=2000"),
            400,
        )
        .await;
    }

    #[tokio::test]
    async fn download_preserves_method_and_version_error_precedence() {
        let malformed = "&id=nope&mountId=nope&offsetTimeMs=nope&playTimeMs=nope";
        for (uri, code) in [
            (
                format!(
                    "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Recording&method=Missing&version=6{malformed}"
                ),
                103,
            ),
            (
                format!(
                    "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Recording&method=Download&version=5{malformed}"
                ),
                104,
            ),
        ] {
            assert_download_error(catalogue(), &uri, code).await;
        }
    }

    #[tokio::test]
    #[ignore = "requires Nix-provided FFmpeg and FFprobe"]
    async fn downloads_partial_recording() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../camera/fixtures/default.mp4")
            .canonicalize()
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        for (fields, expected_duration) in [
            ("&offsetTimeMs=1000&playTimeMs=2000", 2.0),
            ("&offsetTimeMs=1000", 4.0),
        ] {
            let response = get(
                app(download_catalogue(&fixture)),
                &format!("{DOWNLOAD}&id=11{fields}"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK, "{fields}");
            assert_eq!(
                response.headers().get(CONTENT_TYPE).unwrap(),
                "video/mp4",
                "{fields}"
            );

            let output_path = directory
                .path()
                .join(format!("partial-{expected_duration}.mp4"));
            fs::write(
                &output_path,
                to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            )
            .unwrap();
            let output = Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=codec_name:format=duration",
                    "-of",
                    "json",
                ])
                .arg(&output_path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "FFprobe failed for {fields}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let probe: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(probe["streams"][0]["codec_name"], "h264", "{fields}");
            let duration = probe["format"]["duration"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap();
            assert!(
                (duration - expected_duration).abs() <= 0.2,
                "duration for {fields} was {duration}"
            );
        }
    }

    #[tokio::test]
    async fn advertises_recording_versions_five_through_six() {
        let response = get(
            app(vec![]),
            "/webapi/query.cgi?api=SYNO.API.Info&method=Query&version=1&query=SYNO.SurveillanceStation.Recording",
        )
        .await;

        assert_eq!(
            json_body(response).await,
            json!({
                "success": true,
                "data": {
                    "SYNO.SurveillanceStation.Recording": {
                        "path": "entry.cgi",
                        "minVersion": 5,
                        "maxVersion": 6
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn lists_the_exact_version_six_schema_and_ignores_sid() {
        assert_eq!(
            list("&cameraIds=2&fromTime=150&toTime=250&dsId=1&_sid=ignored").await,
            json!({
                "success": true,
                "data": {
                    "dsId": 1,
                    "recordings": [{
                        "id": 13,
                        "videoCodec": 3,
                        "audioCodec": 0,
                        "height": 1080,
                        "width": 1920,
                        "cameraId": 2,
                        "cameraName": "camera-2",
                        "sizeByte": 130,
                        "filePath": "logical/day-two/camera-2-200.mp4",
                        "locked": false
                    }],
                    "total": 1
                }
            })
        );
    }

    #[tokio::test]
    async fn lists_the_exact_version_five_schema_with_a_current_numeric_timestamp() {
        let before = unix_time();
        let response = get(
            app(catalogue()),
            &format!("{LIST}&version=5&cameraIds=1&fromTime=150&toTime=250"),
        )
        .await;
        let mut body = json_body(response).await;
        let after = unix_time();

        let timestamp = body["data"]["timestamp"].as_u64().unwrap();
        assert!((before..=after).contains(&timestamp));
        body["data"]["timestamp"] = json!(0);
        assert_eq!(
            body,
            json!({
                "success": true,
                "data": {
                    "events": [{
                        "archId": 0,
                        "audioCodec": "",
                        "bookmark": [],
                        "bookmarkCount": 0,
                        "cameraId": 1,
                        "dsId": 1,
                        "folder": "logical/day-one",
                        "id": 11,
                        "imgHeight": 1080,
                        "imgWidth": 1920,
                        "startTime": 100,
                        "stopTime": 200,
                        "videoCodec": "H.264"
                    }],
                    "offset": 0,
                    "timestamp": 0,
                    "total": 1
                }
            })
        );
    }

    #[tokio::test]
    async fn version_six_filters_by_its_effective_ds_id() {
        for (suffix, ds_id, ids) in [("", 0, &[14][..]), ("&dsId=1", 1, &[11, 12, 13][..])] {
            let body = list(suffix).await;

            assert_eq!(body["data"]["dsId"], ds_id, "{suffix}");
            assert_eq!(body["data"]["total"], ids.len(), "{suffix}");
            assert_eq!(
                body["data"]["recordings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|recording| recording["id"].as_u64().unwrap())
                    .collect::<Vec<_>>(),
                ids,
                "{suffix}"
            );
        }
    }

    #[tokio::test]
    async fn version_five_without_ds_id_returns_events_from_every_ds() {
        let response = get(app(catalogue()), &format!("{LIST}&version=5")).await;
        let body = json_body(response).await;

        assert_eq!(
            body["data"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| (
                    event["id"].as_u64().unwrap(),
                    event["dsId"].as_u64().unwrap()
                ))
                .collect::<Vec<_>>(),
            [(11, 1), (12, 1), (13, 1), (14, 0)]
        );
    }

    #[tokio::test]
    async fn filters_by_camera_storage_and_half_open_time_range() {
        for (suffix, total, ids) in [
            ("&dsId=1&cameraIds=2", 2, &[12, 13][..]),
            ("&dsId=1&mountId=10", 2, &[11, 12][..]),
            ("&dsId=1&fromTime=150&toTime=200", 1, &[11][..]),
            ("&dsId=1&fromTime=200&toTime=300", 1, &[13][..]),
            ("&dsId=1&fromTime=0&toTime=0", 3, &[11, 12, 13][..]),
        ] {
            assert_eq!(listed_ids(suffix).await, (total, ids.to_vec()), "{suffix}");
        }
    }

    #[tokio::test]
    async fn paginates_after_filtering_and_keeps_the_filtered_total() {
        assert_eq!(
            listed_ids("&dsId=1&cameraIds=2&offset=1&limit=1").await,
            (2, vec![13])
        );
    }

    #[tokio::test]
    async fn accepts_an_offset_beyond_the_filtered_total() {
        assert_eq!(
            list("&dsId=1&cameraIds=2&offset=9").await,
            json!({
                "success": true,
                "data": {"dsId": 1, "recordings": [], "total": 2}
            })
        );
    }

    #[tokio::test]
    async fn rejects_malformed_list_fields() {
        for field in [
            "offset=nope",
            "limit=-1",
            "cameraIds=1,nope",
            "fromTime=-1",
            "toTime=nope",
            "dsId=nope",
            "mountId=nope",
        ] {
            assert_eq!(
                list(&format!("&{field}")).await,
                json!({"success": false, "error": {"code": 401}}),
                "{field}"
            );
        }
    }

    #[tokio::test]
    async fn validates_api_method_and_version_before_list_fields() {
        let malformed = "&offset=nope&limit=nope&cameraIds=nope&fromTime=nope&toTime=nope&dsId=nope&mountId=nope";
        for (uri, code) in [
            (
                format!("/webapi/entry.cgi?api=Missing&method=Missing&version=nope{malformed}"),
                102,
            ),
            (
                format!(
                    "/webapi/entry.cgi?api=SYNO.SurveillanceStation.Recording&method=Missing&version=nope{malformed}"
                ),
                103,
            ),
            (format!("{LIST}&version=4{malformed}"), 104),
            (format!("{LIST}&version=5{malformed}"), 401),
        ] {
            let response = get(app(catalogue()), &uri).await;
            assert_eq!(
                json_body(response).await,
                json!({"success": false, "error": {"code": code}}),
                "{uri}"
            );
        }
    }

    async fn v5_event(video_codec: u8, audio_codec: u8) -> Value {
        let mut cameras = catalogue();
        let recording = cameras[0]
            .recordings
            .iter_mut()
            .find(|recording| recording.id == 11)
            .unwrap();
        recording.video_codec = video_codec;
        recording.audio_codec = audio_codec;
        let response = get(app(cameras), &format!("{LIST}&version=5&limit=1")).await;
        json_body(response).await["data"]["events"][0].clone()
    }

    #[tokio::test]
    async fn version_six_preserves_numeric_codecs() {
        let mut cameras = catalogue();
        let recording = cameras[0]
            .recordings
            .iter_mut()
            .find(|recording| recording.id == 11)
            .unwrap();
        recording.video_codec = 255;
        recording.audio_codec = 254;
        let response = get(app(cameras), &format!("{LIST}&version=6&dsId=1&limit=1")).await;
        let body = json_body(response).await;

        assert_eq!(body["data"]["recordings"][0]["videoCodec"], 255);
        assert_eq!(body["data"]["recordings"][0]["audioCodec"], 254);
    }

    #[tokio::test]
    async fn version_five_maps_every_documented_codec() {
        for (codec, name) in [
            (0, "Unknown"),
            (1, "MJPEG"),
            (2, "MPEG4"),
            (3, "H.264"),
            (5, "MXPEG"),
            (6, "H.265"),
            (7, "H.264+"),
        ] {
            assert_eq!(v5_event(codec, 0).await["videoCodec"], name, "{codec}");
        }
        for (codec, name) in [
            (0, ""),
            (1, "PCM"),
            (2, "G711"),
            (3, "G726"),
            (4, "AAC"),
            (5, "AMR"),
            (6, "UserDefine"),
        ] {
            assert_eq!(v5_event(3, codec).await["audioCodec"], name, "{codec}");
        }
    }

    #[tokio::test]
    async fn version_five_maps_unknown_runtime_codecs_to_unknown() {
        let event = v5_event(4, 7).await;

        assert_eq!(event["videoCodec"], "Unknown");
        assert_eq!(event["audioCodec"], "Unknown");
    }

    #[tokio::test]
    async fn invalid_codec_outside_version_five_page_does_not_fail_the_page() {
        let mut cameras = catalogue();
        let recording = cameras[0]
            .recordings
            .iter_mut()
            .find(|recording| recording.id == 14)
            .unwrap();
        recording.video_codec = 4;
        recording.audio_codec = 7;
        let response = get(app(cameras), &format!("{LIST}&version=5&limit=1")).await;
        let body = json_body(response).await;

        assert_eq!(body["success"], true);
        assert_eq!(body["data"]["events"][0]["id"], 11);
    }
}
