use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use axum::{
    Json,
    body::Bytes,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hypertext::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

use crate::{
    db,
    http::{
        router::{AppState, MAX_VIDEO_UPLOAD_SIZE_BYTES, VIDEO_UPLOAD_CHUNK_SIZE_BYTES},
        ui::{Public, Route, RouteContext, RouteError, RouteView, document},
    },
};

use super::home::video_workspace;

pub struct UploadVideoRoute;

pub struct UploadVideoView {
    videos: Vec<db::video::Video>,
}

#[derive(Clone)]
pub struct ChunkedUploadStore {
    staging_dir: Arc<PathBuf>,
    sessions: Arc<Mutex<HashMap<String, ChunkedUploadSession>>>,
}

#[derive(Clone)]
struct ChunkedUploadSession {
    filename: String,
    expected_size: u64,
    received_bytes: u64,
    next_chunk_index: u64,
    partial_path: PathBuf,
}

#[derive(Deserialize)]
pub struct StartChunkedUpload {
    filename: String,
    size: u64,
}

#[derive(Serialize)]
pub struct StartChunkedUploadResponse {
    upload_id: String,
    chunk_size: u64,
    max_size: u64,
}

#[derive(Debug, Error)]
enum ChunkedUploadError {
    #[error("upload was not found")]
    NotFound,
    #[error("video upload cannot be empty")]
    EmptyUpload,
    #[error("video upload exceeds the 4 GiB limit")]
    UploadTooLarge,
    #[error("video upload requires a file name")]
    MissingFilename,
    #[error("chunk cannot be empty")]
    EmptyChunk,
    #[error("chunk exceeds the 64 MiB limit")]
    ChunkTooLarge,
    #[error("expected chunk {expected}, received chunk {received}")]
    OutOfOrder { expected: u64, received: u64 },
    #[error("chunk exceeds the declared upload size")]
    TooManyBytes,
    #[error("upload is incomplete")]
    IncompleteUpload,
    #[error("uploaded bytes did not match the declared upload size")]
    ByteCountMismatch,
    #[error("uploaded chunk is too large to record")]
    SizeOverflow,
    #[error(transparent)]
    Io(#[from] std::io::Error),
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

impl ChunkedUploadStore {
    pub fn new(staging_dir: PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&staging_dir)?;

        Ok(Self {
            staging_dir: Arc::new(staging_dir),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    async fn create(
        &self,
        filename: String,
        expected_size: u64,
    ) -> Result<StartChunkedUploadResponse, ChunkedUploadError> {
        if filename.trim().is_empty() {
            return Err(ChunkedUploadError::MissingFilename);
        }

        if expected_size == 0 {
            return Err(ChunkedUploadError::EmptyUpload);
        }

        if expected_size > MAX_VIDEO_UPLOAD_SIZE_BYTES as u64 {
            return Err(ChunkedUploadError::UploadTooLarge);
        }

        self.cleanup_stale_partials().await?;

        let upload_id = self.allocate_upload_id(&filename).await;
        let partial_path = self.staging_dir.join(format!("{upload_id}.part"));
        fs::File::create(&partial_path).await?;

        let session = ChunkedUploadSession {
            filename,
            expected_size,
            received_bytes: 0,
            next_chunk_index: 0,
            partial_path,
        };
        self.sessions
            .lock()
            .await
            .insert(upload_id.clone(), session);

        Ok(StartChunkedUploadResponse {
            upload_id,
            chunk_size: VIDEO_UPLOAD_CHUNK_SIZE_BYTES as u64,
            max_size: MAX_VIDEO_UPLOAD_SIZE_BYTES as u64,
        })
    }

    async fn append_chunk(
        &self,
        upload_id: &str,
        chunk_index: u64,
        bytes: Bytes,
    ) -> Result<(), ChunkedUploadError> {
        if bytes.is_empty() {
            return Err(ChunkedUploadError::EmptyChunk);
        }

        if bytes.len() > VIDEO_UPLOAD_CHUNK_SIZE_BYTES {
            return Err(ChunkedUploadError::ChunkTooLarge);
        }

        let chunk_size =
            u64::try_from(bytes.len()).map_err(|_| ChunkedUploadError::SizeOverflow)?;
        let mut sessions = self.sessions.lock().await;
        let session = sessions
            .get_mut(upload_id)
            .ok_or(ChunkedUploadError::NotFound)?;

        if chunk_index < session.next_chunk_index {
            return Ok(());
        }

        if chunk_index != session.next_chunk_index {
            return Err(ChunkedUploadError::OutOfOrder {
                expected: session.next_chunk_index,
                received: chunk_index,
            });
        }

        let received_bytes = session
            .received_bytes
            .checked_add(chunk_size)
            .ok_or(ChunkedUploadError::SizeOverflow)?;
        if received_bytes > session.expected_size {
            return Err(ChunkedUploadError::TooManyBytes);
        }

        let mut file = OpenOptions::new()
            .append(true)
            .open(&session.partial_path)
            .await?;
        file.write_all(&bytes).await?;

        session.received_bytes = received_bytes;
        session.next_chunk_index += 1;

        Ok(())
    }

    async fn complete(&self, upload_id: &str) -> Result<(String, Vec<u8>), ChunkedUploadError> {
        let session = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .get(upload_id)
                .ok_or(ChunkedUploadError::NotFound)?;

            if session.received_bytes != session.expected_size {
                return Err(ChunkedUploadError::IncompleteUpload);
            }

            sessions
                .remove(upload_id)
                .ok_or(ChunkedUploadError::NotFound)?
        };

        let bytes = fs::read(&session.partial_path).await?;
        if bytes.len() as u64 != session.expected_size {
            let _ = fs::remove_file(&session.partial_path).await;
            return Err(ChunkedUploadError::ByteCountMismatch);
        }

        remove_file_if_exists(&session.partial_path).await?;
        Ok((session.filename, bytes))
    }

    async fn cancel(&self, upload_id: &str) -> Result<(), ChunkedUploadError> {
        let session = self
            .sessions
            .lock()
            .await
            .remove(upload_id)
            .ok_or(ChunkedUploadError::NotFound)?;
        remove_file_if_exists(&session.partial_path).await?;
        Ok(())
    }

    async fn allocate_upload_id(&self, filename: &str) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let filename = sanitize_upload_name(filename);
        let sessions = self.sessions.lock().await;

        for attempt in 0.. {
            let upload_id = format!("{now}-{attempt}-{filename}");
            if !sessions.contains_key(&upload_id) {
                return upload_id;
            }
        }

        unreachable!("upload id allocation should find an unused id")
    }

    async fn cleanup_stale_partials(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&*self.staging_dir).await?;

        let stale_before = SystemTime::now()
            .checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
        let mut entries = fs::read_dir(&*self.staging_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            if !metadata.is_file() {
                continue;
            }

            let Ok(modified) = metadata.modified() else {
                continue;
            };

            if modified < stale_before {
                let _ = fs::remove_file(entry.path()).await;
            }
        }

        Ok(())
    }
}

pub async fn start_chunked_upload(
    State(state): State<AppState>,
    Json(input): Json<StartChunkedUpload>,
) -> Response {
    match state
        .chunked_uploads()
        .create(input.filename, input.size)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => error.into_response(),
    }
}

pub async fn upload_chunk(
    State(state): State<AppState>,
    Path((upload_id, chunk_index)): Path<(String, u64)>,
    bytes: Bytes,
) -> Response {
    match state
        .chunked_uploads()
        .append_chunk(&upload_id, chunk_index, bytes)
        .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => error.into_response(),
    }
}

pub async fn complete_chunked_upload(
    State(state): State<AppState>,
    Path(upload_id): Path<String>,
) -> Response {
    let (filename, bytes) = match state.chunked_uploads().complete(&upload_id).await {
        Ok(upload) => upload,
        Err(error) => return error.into_response(),
    };

    if let Err(error) = db::video::Video::upload(state.db(), filename, bytes).await {
        return RouteError::from(error).into_response();
    }

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
        Err(error) => error.into_response(),
    }
}

impl IntoResponse for ChunkedUploadError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, self.to_string()).into_response(),
            Self::EmptyUpload | Self::MissingFilename | Self::EmptyChunk => {
                (StatusCode::BAD_REQUEST, self.to_string()).into_response()
            }
            Self::UploadTooLarge | Self::ChunkTooLarge => {
                (StatusCode::PAYLOAD_TOO_LARGE, self.to_string()).into_response()
            }
            Self::OutOfOrder { .. } => (StatusCode::CONFLICT, self.to_string()).into_response(),
            Self::TooManyBytes
            | Self::IncompleteUpload
            | Self::ByteCountMismatch
            | Self::SizeOverflow => (StatusCode::BAD_REQUEST, self.to_string()).into_response(),
            Self::Io(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("chunked upload failure: {error}"),
            )
                .into_response(),
        }
    }
}

async fn remove_file_if_exists(path: &PathBuf) -> Result<(), std::io::Error> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn sanitize_upload_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.is_empty() {
        "video".to_owned()
    } else {
        sanitized
    }
}
