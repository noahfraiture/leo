//! Chunked upload session state and staging-file assembly.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use thiserror::Error;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

pub const MAX_VIDEO_UPLOAD_SIZE_BYTES: usize = 4 * 1024 * 1024 * 1024;
pub const MAX_VIDEO_UPLOAD_SIZE_LABEL: &str = "4 GiB";
pub const VIDEO_UPLOAD_CHUNK_SIZE_BYTES: usize = 64 * 1024 * 1024;
pub const VIDEO_UPLOAD_CHUNK_REQUEST_LIMIT_BYTES: usize =
    VIDEO_UPLOAD_CHUNK_SIZE_BYTES + 1024 * 1024;

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

#[derive(Serialize)]
pub struct StartChunkedUploadResponse {
    pub upload_id: String,
    pub chunk_size: u64,
    pub max_size: u64,
}

#[derive(Debug, Error)]
pub enum ChunkedUploadError {
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

impl ChunkedUploadStore {
    pub fn new(staging_dir: PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&staging_dir)?;

        Ok(Self {
            staging_dir: Arc::new(staging_dir),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn start(
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

    pub async fn append_chunk(
        &self,
        upload_id: &str,
        chunk_index: u64,
        bytes: &[u8],
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
        file.write_all(bytes).await?;

        session.received_bytes = received_bytes;
        session.next_chunk_index += 1;

        Ok(())
    }

    pub async fn complete(&self, upload_id: &str) -> Result<(String, Vec<u8>), ChunkedUploadError> {
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

    pub async fn cancel(&self, upload_id: &str) -> Result<(), ChunkedUploadError> {
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
