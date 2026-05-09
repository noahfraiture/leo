use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use surrealdb::types::{Bytes, File, RecordId, SurrealValue};
use thiserror::Error;

use crate::db::Database;

const VIDEO_TABLE: &str = "video";
const VIDEO_BUCKET: &str = "videos";

/// Uploaded video metadata stored in SurrealDB.
///
/// The actual bytes live in the `videos` file bucket. This record keeps the
/// user-facing filename, public playback path, file size, and bucket reference
/// that other routes use to render and manage uploaded videos.
#[derive(Clone, Debug, PartialEq, SurrealValue)]
pub struct Video {
    /// SurrealDB record id in the `video` table.
    pub id: RecordId,
    /// Original filename supplied at upload time.
    pub name: String,
    /// Public URL path used by HTML video players.
    pub path: String,
    /// Uploaded file size in bytes.
    pub size: u64,
    /// SurrealDB file bucket pointer for the stored video bytes.
    pub file: File,
}

#[derive(Clone)]
pub struct VideoAsset {
    pub video: Video,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("uploaded video is too large to record its size")]
    SizeOverflow,
    #[error("database did not return the created video record")]
    MissingCreatedRecord,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Surreal(#[from] surrealdb::Error),
}

#[derive(SurrealValue)]
struct UploadVideo {
    name: String,
    path: String,
    size: u64,
    file: File,
    bytes: Bytes,
}

#[derive(SurrealValue)]
struct FindVideo {
    file: File,
}

#[derive(SurrealValue)]
struct FindVideoByName {
    name: String,
}

#[derive(SurrealValue)]
struct DeleteVideo {
    id: RecordId,
    file: File,
}

impl Video {
    /// Creates the `videos` bucket and `video` metadata table if they do not exist.
    ///
    /// `bucket_path` is the local filesystem backend directory used by the
    /// embedded SurrealDB file bucket.
    pub async fn init(db: &Database, bucket_path: impl AsRef<Path>) -> Result<(), VideoError> {
        define_video_bucket(db, bucket_path.as_ref()).await?;
        define_video_table(db).await?;
        Ok(())
    }

    /// Returns all uploaded video metadata records, newest first.
    pub async fn list(db: &Database) -> Result<Vec<Video>, VideoError> {
        let mut response = db
            .query("SELECT * FROM video ORDER BY created_at DESC;")
            .await?;

        Ok(response.take(0)?)
    }

    /// Returns one uploaded video metadata record by its bucket file key.
    pub async fn find_by_file_key(db: &Database, key: &str) -> Result<Option<Video>, VideoError> {
        let mut response = db
            .query("SELECT * FROM video WHERE file = $file LIMIT 1;")
            .bind(FindVideo {
                file: File::new(VIDEO_BUCKET, key),
            })
            .await?;

        let mut videos: Vec<Video> = response.take(0)?;
        Ok(videos.pop())
    }

    /// Returns one uploaded video and its stored bucket bytes by file key.
    pub async fn read_by_file_key(
        db: &Database,
        key: &str,
    ) -> Result<Option<VideoAsset>, VideoError> {
        let mut response = db
            .query(
                r#"
                SELECT * FROM video WHERE file = $file LIMIT 1;
                RETURN file::get($file);
                "#,
            )
            .bind(FindVideo {
                file: File::new(VIDEO_BUCKET, key),
            })
            .await?;

        let mut videos: Vec<Video> = response.take(0)?;
        let Some(video) = videos.pop() else {
            return Ok(None);
        };
        let bytes = Bytes::from_value(response.take::<surrealdb::types::Value>(1)?)?;

        Ok(Some(VideoAsset {
            video,
            bytes: bytes.into_inner().to_vec(),
        }))
    }

    /// Returns one uploaded video and its stored bucket bytes by original file name.
    pub async fn read_by_name(db: &Database, name: &str) -> Result<Option<VideoAsset>, VideoError> {
        let mut response = db
            .query(
                r#"
                SELECT * FROM video WHERE name = $name LIMIT 1;
                RETURN file::get((SELECT VALUE file FROM video WHERE name = $name LIMIT 1)[0]);
                "#,
            )
            .bind(FindVideoByName {
                name: name.to_owned(),
            })
            .await?;

        let mut videos: Vec<Video> = response.take(0)?;
        let Some(video) = videos.pop() else {
            return Ok(None);
        };
        let bytes = Bytes::from_value(response.take::<surrealdb::types::Value>(1)?)?;

        Ok(Some(VideoAsset {
            video,
            bytes: bytes.into_inner().to_vec(),
        }))
    }

    /// Deletes this video's metadata record and its stored bucket file.
    pub async fn delete(&self, db: &Database) -> Result<(), VideoError> {
        db.query(
            r#"
            file::delete($file);
            DELETE $id;
            "#,
        )
        .bind(DeleteVideo {
            id: self.id.clone(),
            file: self.file.clone(),
        })
        .await?
        .check()?;

        Ok(())
    }

    /// Stores video bytes in the `videos` bucket and creates its metadata record.
    ///
    /// The returned record includes the generated public path and byte size.
    pub async fn upload(
        db: &Database,
        name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<Video, VideoError> {
        let name = name.into();
        let bytes = bytes.into();
        let size = u64::try_from(bytes.len()).map_err(|_| VideoError::SizeOverflow)?;
        let file = File::new(VIDEO_BUCKET, video_file_key(&name));
        let path = public_video_path(&name);

        let mut response = db
            .query(
                r#"
                file::put($file, $bytes);
                CREATE video CONTENT {
                    name: $name,
                    path: $path,
                    size: $size,
                    file: $file,
                    created_at: time::now(),
                };
                "#,
            )
            .bind(UploadVideo {
                name,
                path,
                size,
                file,
                bytes: Bytes::from(bytes),
            })
            .await?;

        let mut created: Vec<Video> = response.take(1)?;
        created.pop().ok_or(VideoError::MissingCreatedRecord)
    }
}

async fn define_video_bucket(db: &Database, bucket_path: &Path) -> Result<(), VideoError> {
    let path = prepare_video_bucket_path(bucket_path)?;
    let backend = format!("file:{}?lowercase_paths=false", path.display());
    let backend = serde_json::to_string(&backend)?;
    let query = format!("DEFINE BUCKET IF NOT EXISTS {VIDEO_BUCKET} BACKEND {backend};");

    db.query(query).await?.check()?;
    Ok(())
}

async fn define_video_table(db: &Database) -> Result<(), VideoError> {
    db.query(
        r#"
        DEFINE TABLE IF NOT EXISTS video SCHEMAFULL;
        DEFINE FIELD IF NOT EXISTS name ON TABLE video TYPE string;
        DEFINE FIELD IF NOT EXISTS path ON TABLE video TYPE string;
        DEFINE FIELD IF NOT EXISTS size ON TABLE video TYPE int ASSERT $value >= 0;
        DEFINE FIELD IF NOT EXISTS file ON TABLE video TYPE file<videos>;
        DEFINE FIELD IF NOT EXISTS created_at ON TABLE video TYPE datetime;
        DEFINE INDEX IF NOT EXISTS video_path ON TABLE video FIELDS path UNIQUE;
        DEFINE INDEX IF NOT EXISTS video_name ON TABLE video FIELDS name UNIQUE;
        "#,
    )
    .await?
    .check()?;

    Ok(())
}

fn prepare_video_bucket_path(path: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(path)?;
    let path = path.canonicalize()?;

    if env::var_os("SURREAL_BUCKET_FOLDER_ALLOWLIST").is_none() {
        // SurrealDB reads this process-wide allowlist when file buckets are
        // used, so it must be set before the first bucket operation.
        unsafe {
            env::set_var("SURREAL_BUCKET_FOLDER_ALLOWLIST", &path);
        }
    }

    Ok(path)
}

fn video_file_key(name: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{now}-{}", sanitize_file_name(name))
}

fn sanitize_file_name(name: &str) -> String {
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

fn public_video_path(key: &str) -> String {
    format!("/video/{}", key.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use surrealdb::types::{File, SurrealValue};

    use super::{VIDEO_BUCKET, Video, VideoError};
    use crate::db::Database;

    #[derive(SurrealValue)]
    struct FileParam {
        file: File,
    }

    async fn file_exists(db: &Database, key: &str) -> Result<bool, VideoError> {
        let mut response = db
            .query("RETURN file::exists($file);")
            .bind(FileParam {
                file: File::new(VIDEO_BUCKET, key),
            })
            .await?;

        Ok(bool::from_value(
            response.take::<surrealdb::types::Value>(0)?,
        )?)
    }

    #[tokio::test]
    async fn upload_persists_metadata_with_file_size() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let bytes = b"video bytes".to_vec();

        let video = Video::upload(&db, "sample.mp4", bytes.clone())
            .await
            .expect("video should upload");

        assert_eq!(video.name, "sample.mp4");
        assert_eq!(video.size, bytes.len() as u64);
        assert!(video.path.starts_with("/video/"));
        assert_eq!(video.path, "/video/sample.mp4");

        let videos = Video::list(&db).await.expect("videos should list");
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].name, "sample.mp4");
        assert_eq!(videos[0].size, bytes.len() as u64);
        assert_eq!(videos[0].path, video.path);
    }

    #[tokio::test]
    async fn delete_removes_metadata_and_file() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let video = Video::upload(&db, "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        video.delete(&db).await.expect("video should delete");

        let videos = Video::list(&db).await.expect("videos should list");
        assert!(videos.is_empty());
        assert!(
            !file_exists(&db, video.file.key())
                .await
                .expect("file existence should be checked")
        );
    }

    #[tokio::test]
    async fn find_by_file_key_returns_uploaded_video() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let video = Video::upload(&db, "sample.mp4", b"video bytes".to_vec())
            .await
            .expect("video should upload");

        let found = Video::find_by_file_key(&db, video.file.key())
            .await
            .expect("lookup should complete")
            .expect("video should exist");

        assert_eq!(found.id, video.id);
        assert_eq!(found.file.key(), video.file.key());
        assert_eq!(found.name, "sample.mp4");
    }

    #[tokio::test]
    async fn find_by_file_key_returns_none_for_missing_video() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let found = Video::find_by_file_key(&db, "missing.mp4")
            .await
            .expect("lookup should complete");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn read_by_file_key_returns_uploaded_video_and_bytes() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let bytes = b"video bytes".to_vec();
        let video = Video::upload(&db, "sample.mp4", bytes.clone())
            .await
            .expect("video should upload");

        let found = Video::read_by_file_key(&db, video.file.key())
            .await
            .expect("lookup should complete")
            .expect("video should exist");

        assert_eq!(found.video.id, video.id);
        assert_eq!(found.video.file.key(), video.file.key());
        assert_eq!(found.bytes, bytes);
    }

    #[tokio::test]
    async fn read_by_file_key_returns_none_for_missing_video() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let found = Video::read_by_file_key(&db, "missing.mp4")
            .await
            .expect("lookup should complete");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn upload_rejects_duplicate_file_names() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");

        Video::upload(&db, "sample.mp4", b"first".to_vec())
            .await
            .expect("first upload should succeed");
        let duplicate = Video::upload(&db, "sample.mp4", b"second".to_vec()).await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn read_by_name_returns_uploaded_video_and_bytes() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");
        let bytes = b"video bytes".to_vec();
        let video = Video::upload(&db, "sample.mp4", bytes.clone())
            .await
            .expect("video should upload");

        let found = Video::read_by_name(&db, "sample.mp4")
            .await
            .expect("lookup should complete")
            .expect("video should exist");

        assert_eq!(found.video.id, video.id);
        assert_eq!(found.video.name, "sample.mp4");
        assert_eq!(found.bytes, bytes);
    }

    #[tokio::test]
    async fn read_by_name_returns_none_for_missing_video() {
        let db = crate::test::database::init()
            .await
            .expect("test database should initialize");

        let found = Video::read_by_name(&db, "missing.mp4")
            .await
            .expect("lookup should complete");

        assert!(found.is_none());
    }
}
