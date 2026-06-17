//! HTTP video byte serving with range request support.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE},
    },
    response::{IntoResponse, Response},
};

use crate::{
    app::AppState,
    db,
    media::playback::{ByteRange, content_type, parse_byte_range},
};

pub async fn serve(
    Path(video_name): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    match db::video::Video::read_by_name(state.db(), &video_name).await {
        Ok(Some(asset)) => video_response(
            asset,
            headers.get(RANGE).and_then(|value| value.to_str().ok()),
        ),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("video route failure: {error}"),
        )
            .into_response(),
    }
}

fn video_response(asset: db::video::VideoAsset, range: Option<&str>) -> Response {
    let total_len = asset.bytes.len();

    if let Some(range) = range {
        let Some(range) = parse_byte_range(range, total_len) else {
            return range_not_satisfiable_response(&asset.video.name, total_len);
        };

        return partial_video_response(asset, range, total_len);
    }

    full_video_response(asset)
}

fn full_video_response(asset: db::video::VideoAsset) -> Response {
    let mut headers = video_headers(&asset.video.name);
    insert_header(&mut headers, CONTENT_LENGTH, asset.bytes.len());

    (headers, Body::from(asset.bytes)).into_response()
}

fn partial_video_response(
    asset: db::video::VideoAsset,
    range: ByteRange,
    total_len: usize,
) -> Response {
    let mut headers = video_headers(&asset.video.name);
    let range_len = range.end - range.start + 1;

    insert_header(&mut headers, CONTENT_LENGTH, range_len);
    insert_header(
        &mut headers,
        CONTENT_RANGE,
        format!("bytes {}-{}/{}", range.start, range.end, total_len),
    );

    (
        StatusCode::PARTIAL_CONTENT,
        headers,
        Body::from(asset.bytes[range.start..=range.end].to_vec()),
    )
        .into_response()
}

fn range_not_satisfiable_response(name: &str, total_len: usize) -> Response {
    let mut headers = video_headers(name);
    insert_header(&mut headers, CONTENT_RANGE, format!("bytes */{total_len}"));

    (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response()
}

fn video_headers(name: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type(name)));
    headers
}

fn insert_header(
    headers: &mut HeaderMap,
    name: axum::http::header::HeaderName,
    value: impl ToString,
) {
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        headers.insert(name, value);
    }
}
