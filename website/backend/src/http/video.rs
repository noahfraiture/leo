use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_LENGTH, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};

use crate::{db, http::router::AppState};

pub async fn serve(Path(video_name): Path<String>, State(state): State<AppState>) -> Response {
    match db::video::Video::read_by_name(state.db(), &video_name).await {
        Ok(Some(asset)) => video_response(asset),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("video route failure: {error}"),
        )
            .into_response(),
    }
}

fn video_response(asset: db::video::VideoAsset) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(content_type(&asset.video.name)),
    );

    if let Ok(value) = HeaderValue::from_str(&asset.bytes.len().to_string()) {
        headers.insert(CONTENT_LENGTH, value);
    }

    (headers, Body::from(asset.bytes)).into_response()
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some(extension) if extension.eq_ignore_ascii_case("webm") => "video/webm",
        Some(extension) if extension.eq_ignore_ascii_case("mov") => "video/quicktime",
        Some(extension) if extension.eq_ignore_ascii_case("avi") => "video/x-msvideo",
        _ => "video/mp4",
    }
}
