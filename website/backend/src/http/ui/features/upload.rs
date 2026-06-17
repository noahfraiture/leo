//! Multipart and chunked upload routes for video files.

use async_trait::async_trait;
use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use hypertext::prelude::*;
use serde::Deserialize;
use serde_json::json;

use crate::{
    app::AppState,
    db,
    http::ui::{Public, Route, RouteContext, RouteError, RouteView, document},
    upload::ChunkedUploadError,
};

use super::home::video_workspace;

pub struct UploadVideoRoute;

pub struct UploadVideoView {
    videos: Vec<db::video::Video>,
}

#[derive(Deserialize)]
pub struct StartChunkedUpload {
    filename: String,
    size: u64,
}

#[async_trait]
impl Route for UploadVideoRoute {
    type Input = Multipart;
    type Authz = Public;
    type View = UploadVideoView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        mut input: Self::Input,
    ) -> Result<Self::View, RouteError> {
        let (filename, bytes) = uploaded_video(&mut input).await?;

        db::video::Video::upload(context.state().db(), filename, bytes).await?;
        let videos = db::video::Video::list(context.state().db()).await?;

        Ok(UploadVideoView { videos })
    }
}

impl RouteView for UploadVideoView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | Videos",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    <section class="space-y-6 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm">
                        <h1 class="text-2xl font-semibold text-base-content">"Uploaded videos"</h1>
                        (video_workspace(&self.videos))
                        <a class="btn btn-primary" href="/">"Back to analysis"</a>
                    </section>
                </main>
            },
        )
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        video_workspace(&self.videos)
    }
}

async fn uploaded_video(multipart: &mut Multipart) -> Result<(String, Bytes), RouteError> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| RouteError::BadRequest("invalid multipart upload"))?
    {
        if field.name() != Some("video") {
            continue;
        }

        let filename = field
            .file_name()
            .map(str::to_owned)
            .ok_or(RouteError::BadRequest("video upload requires a file name"))?;
        let bytes = field
            .bytes()
            .await
            .map_err(|_| RouteError::BadRequest("invalid video upload"))?;

        if bytes.is_empty() {
            return Err(RouteError::BadRequest("video upload cannot be empty"));
        }

        return Ok((filename, bytes));
    }

    Err(RouteError::BadRequest("missing video upload field"))
}

pub async fn start_chunked_upload(
    State(state): State<AppState>,
    Json(input): Json<StartChunkedUpload>,
) -> Response {
    let filename = input.filename;
    let size = input.size;
    match state.chunked_uploads().start(filename.clone(), size).await {
        Ok(response) => {
            state
                .metrics()
                .increment("leo_upload_sessions_total", &[("result", "started")]);
            eprintln!(
                "{}",
                json!({
                    "level": "info",
                    "component": "upload",
                    "event": "session_started",
                    "upload_id": response.upload_id,
                    "filename": filename,
                    "declared_size": size,
                    "chunk_size": response.chunk_size,
                    "max_size": response.max_size,
                })
            );
            Json(response).into_response()
        }
        Err(error) => {
            state
                .metrics()
                .increment("leo_upload_sessions_total", &[("result", "failed")]);
            eprintln!(
                "{}",
                json!({
                    "level": "error",
                    "component": "upload",
                    "event": "session_failed",
                    "error": error.to_string(),
                })
            );
            upload_error_response(error)
        }
    }
}

pub async fn upload_chunk(
    State(state): State<AppState>,
    Path((upload_id, chunk_index)): Path<(String, u64)>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Response {
    let chunk_attempt = headers
        .get("X-Upload-Chunk-Attempt")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    let total_chunks = headers
        .get("X-Upload-Total-Chunks")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let chunk_bytes = bytes.len();

    match state
        .chunked_uploads()
        .append_chunk(&upload_id, chunk_index, &bytes)
        .await
    {
        Ok(()) => {
            state
                .metrics()
                .increment("leo_upload_chunks_total", &[("result", "accepted")]);
            eprintln!(
                "{}",
                json!({
                    "level": "info",
                    "component": "upload",
                    "event": "chunk_accepted",
                    "upload_id": upload_id,
                    "chunk_index": chunk_index,
                    "client_attempt": chunk_attempt,
                    "total_chunks": total_chunks,
                    "chunk_bytes": chunk_bytes,
                })
            );
            StatusCode::OK.into_response()
        }
        Err(error) => {
            state
                .metrics()
                .increment("leo_upload_chunks_total", &[("result", "failed")]);
            eprintln!(
                "{}",
                json!({
                    "level": "error",
                    "component": "upload",
                    "event": "chunk_failed",
                    "upload_id": upload_id,
                    "chunk_index": chunk_index,
                    "client_attempt": chunk_attempt,
                    "total_chunks": total_chunks,
                    "chunk_bytes": chunk_bytes,
                    "error": error.to_string(),
                })
            );
            upload_error_response(error)
        }
    }
}

pub async fn complete_chunked_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Response {
    let (filename, bytes) = match state.chunked_uploads().complete(&upload_id).await {
        Ok(upload) => upload,
        Err(error) => return upload_error_response(error),
    };

    if let Err(error) = db::video::Video::upload(state.db(), filename, bytes).await {
        state
            .metrics()
            .increment("leo_upload_sessions_total", &[("result", "failed")]);
        return RouteError::from(error).into_response();
    }
    state
        .metrics()
        .increment("leo_upload_sessions_total", &[("result", "completed")]);
    eprintln!(
        "{}",
        json!({
            "level": "info",
            "component": "upload",
            "event": "session_completed",
            "upload_id": upload_id,
        })
    );

    match db::video::Video::list(state.db()).await {
        Ok(videos) => UploadVideoView { videos }.render_fragment(&state),
        Err(error) => RouteError::from(error).into_response(),
    }
}

pub async fn cancel_chunked_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Response {
    match state.chunked_uploads().cancel(&upload_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => upload_error_response(error),
    }
}

fn upload_error_response(error: ChunkedUploadError) -> Response {
    let message = error.to_string();

    match error {
        ChunkedUploadError::NotFound => (StatusCode::NOT_FOUND, message).into_response(),
        ChunkedUploadError::EmptyUpload
        | ChunkedUploadError::MissingFilename
        | ChunkedUploadError::EmptyChunk => (StatusCode::BAD_REQUEST, message).into_response(),
        ChunkedUploadError::UploadTooLarge | ChunkedUploadError::ChunkTooLarge => {
            (StatusCode::PAYLOAD_TOO_LARGE, message).into_response()
        }
        ChunkedUploadError::OutOfOrder { .. } => (StatusCode::CONFLICT, message).into_response(),
        ChunkedUploadError::TooManyBytes
        | ChunkedUploadError::IncompleteUpload
        | ChunkedUploadError::ByteCountMismatch
        | ChunkedUploadError::SizeOverflow => (StatusCode::BAD_REQUEST, message).into_response(),
        ChunkedUploadError::Io(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chunked upload failure: {error}"),
        )
            .into_response(),
    }
}
