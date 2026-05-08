# Serve Videos Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve uploaded SurrealDB bucket videos from `GET /video/<name>` so HTML video players can use the stored `Video.path` directly.

**Architecture:** Keep upload at `POST /videos`; add a separate raw media endpoint at `GET /video/{key}`. Treat `{key}` as the generated bucket key (`video.file.key()`), not the original display filename, because original filenames can collide. The DB model owns resolving metadata plus bytes; the HTTP layer owns status codes and response headers.

**Tech Stack:** Rust, axum `Path`/`State` extractors, SurrealDB `file::get`, axum `Body`, focused router/model tests through `nix develop -c ...`.

---

## File Structure

- Modify `backend/src/db/models/video.rs`: change public paths from `/public/videos/<key>` to `/video/<key>` and add a method that loads one video's metadata plus bytes by bucket key.
- Create `backend/src/http/video.rs`: raw axum handler for `GET /video/{key}`; this is not a UI `Route` because it returns media bytes, not a document or HTMX fragment.
- Modify `backend/src/http/mod.rs`: expose the new private HTTP video module to `router.rs`.
- Modify `backend/src/http/router.rs`: mount `GET /video/{key}` and add an integration test.
- No change to `backend/src/http/ui/features/home.rs` is required once `Video.path` changes, but future player markup should use `video.path`.

---

### Task 1: Change Stored Public Video Paths

**Files:**
- Modify: `backend/src/db/models/video.rs`

- [ ] **Step 1: Write the failing model test**

Add a path assertion to `upload_persists_metadata_with_file_size`:

```rust
assert!(video.path.starts_with("/video/"));
assert_eq!(video.path, format!("/video/{}", video.file.key()));
```

Replace the old assertion:

```rust
assert!(video.path.starts_with("/public/videos/"));
```

- [ ] **Step 2: Run the model test to verify it fails**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests::upload_persists_metadata_with_file_size
```

Expected: FAIL because `public_video_path()` still returns `/public/videos/<key>`.

- [ ] **Step 3: Update the path helper**

In `backend/src/db/models/video.rs`, change:

```rust
fn public_video_path(key: &str) -> String {
    format!("/public/videos/{}", key.trim_start_matches('/'))
}
```

to:

```rust
fn public_video_path(key: &str) -> String {
    format!("/video/{}", key.trim_start_matches('/'))
}
```

- [ ] **Step 4: Run the model test to verify it passes**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests::upload_persists_metadata_with_file_size
```

Expected: PASS.

---

### Task 2: Add A DB Read Method For Video Bytes

**Files:**
- Modify: `backend/src/db/models/video.rs`

- [ ] **Step 1: Write the failing DB read test**

In the `tests` module in `backend/src/db/models/video.rs`, add:

```rust
#[tokio::test]
async fn read_by_key_returns_metadata_and_bytes() {
    let db = crate::test::database::init()
        .await
        .expect("test database should initialize");
    let bytes = b"video bytes".to_vec();
    let video = Video::upload(&db, "sample.mp4", bytes.clone())
        .await
        .expect("video should upload");

    let stored = Video::read_by_key(&db, video.file.key())
        .await
        .expect("video should read")
        .expect("video should exist");

    assert_eq!(stored.video.name, "sample.mp4");
    assert_eq!(stored.video.file.key(), video.file.key());
    assert_eq!(stored.bytes.as_ref(), bytes.as_slice());
}

#[tokio::test]
async fn read_by_key_returns_none_for_missing_metadata() {
    let db = crate::test::database::init()
        .await
        .expect("test database should initialize");

    let stored = Video::read_by_key(&db, "missing.mp4")
        .await
        .expect("missing lookup should complete");

    assert!(stored.is_none());
}
```

- [ ] **Step 2: Run the DB tests to verify they fail**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests::read_by_key
```

Expected: FAIL because `Video::read_by_key` and the returned asset type do not exist.

- [ ] **Step 3: Add the asset and query parameter types**

Near `Video`, add:

```rust
pub struct VideoAsset {
    pub video: Video,
    pub bytes: Vec<u8>,
}
```

Near the existing `UploadVideo`/`DeleteVideo` parameter structs, add:

```rust
#[derive(SurrealValue)]
struct ReadVideo {
    file: File,
}
```

- [ ] **Step 4: Implement `Video::read_by_key`**

Add this method to `impl Video`:

```rust
pub async fn read_by_key(
    db: &Database,
    key: &str,
) -> Result<Option<VideoAsset>, VideoError> {
    let file = File::new(VIDEO_BUCKET, key);
    let mut response = db
        .query(
            r#"
            SELECT * FROM video WHERE file = $file LIMIT 1;
            RETURN file::get($file);
            "#,
        )
        .bind(ReadVideo { file })
        .await?;

    let mut videos: Vec<Video> = response.take(0)?;
    let Some(video) = videos.pop() else {
        return Ok(None);
    };
    let bytes: Bytes = response.take(1)?;

    Ok(Some(VideoAsset {
        video,
        bytes: bytes.into_inner().to_vec(),
    }))
}
```

Keep the existing `use surrealdb::types::{Bytes, File, RecordId, SurrealValue};`.

- [ ] **Step 5: Run the DB tests to verify they pass**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests
```

Expected: all video model tests pass.

---

### Task 3: Add The Raw Video HTTP Handler

**Files:**
- Create: `backend/src/http/video.rs`
- Modify: `backend/src/http/mod.rs`

- [ ] **Step 1: Create the handler module**

Create `backend/src/http/video.rs`:

```rust
use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        header::{CONTENT_LENGTH, CONTENT_TYPE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
};

use crate::{db, http::router::AppState};

pub async fn serve(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    match db::video::Video::read_by_key(state.db(), &key).await {
        Ok(Some(asset)) => video_response(asset),
        Ok(None) => (StatusCode::NOT_FOUND, "video not found").into_response(),
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
        HeaderValue::from_static(content_type_for(&asset.video.name)),
    );
    if let Ok(length) = HeaderValue::from_str(&asset.bytes.len().to_string()) {
        headers.insert(CONTENT_LENGTH, length);
    }

    (headers, Body::from(asset.bytes)).into_response()
}

fn content_type_for(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase()) {
        Some(extension) if extension == "mp4" => "video/mp4",
        Some(extension) if extension == "webm" => "video/webm",
        Some(extension) if extension == "mov" => "video/quicktime",
        Some(extension) if extension == "m4v" => "video/x-m4v",
        Some(extension) if extension == "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}
```

- [ ] **Step 2: Expose the module inside `http`**

In `backend/src/http/mod.rs`, change:

```rust
pub mod router;
mod ui;
```

to:

```rust
pub mod router;
mod ui;
mod video;
```

- [ ] **Step 3: Run a compile check and expect failure until route is mounted**

Run:

```bash
nix develop -c cargo check --manifest-path backend/Cargo.toml
```

Expected: if `Video::read_by_key` is not implemented yet, compilation fails there. If Task 2 is complete, compilation should pass even before mounting because `mod video` compiles the handler.

---

### Task 4: Mount And Test `GET /video/{key}`

**Files:**
- Modify: `backend/src/http/router.rs`

- [ ] **Step 1: Write the failing router tests**

In `backend/src/http/router.rs`, add these tests inside `mod tests`:

```rust
#[tokio::test]
async fn video_route_serves_uploaded_video_bytes() {
    let state = AppState::for_test().await;
    let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
        .await
        .expect("video should upload");

    let response = app(state)
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/video/{}", video.file.key()))
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body should read");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("video/mp4"));
    assert_eq!(body.as_ref(), b"video bytes");
}

#[tokio::test]
async fn video_route_returns_not_found_for_missing_video() {
    let response = test_app()
        .await
        .oneshot(
            HttpRequest::builder()
                .uri("/video/missing.mp4")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run the router tests to verify they fail**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml http::router::tests::video_route
```

Expected: FAIL with 404 from the router because `/video/{key}` is not mounted yet.

- [ ] **Step 3: Mount the route**

In `backend/src/http/router.rs`, add `get` route mounting:

```rust
.route("/video/{key}", get(crate::http::video::serve))
```

Place it near the other public routes:

```rust
Router::new()
    .route("/", ui::route::<ui::features::HomePage>(MethodFilter::GET))
    .route("/healthz", get(ui::features::healthz))
    .route("/video/{key}", get(crate::http::video::serve))
    .route(
        "/analysis",
        ui::route::<ui::features::AnalyzeRoute>(MethodFilter::POST),
    )
```

- [ ] **Step 4: Run the router tests to verify they pass**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml http::router::tests::video_route
```

Expected: both video route tests pass.

---

### Task 5: Final Verification

**Files:**
- Verify: whole repository

- [ ] **Step 1: Format**

Run:

```bash
nix develop -c cargo fmt --manifest-path backend/Cargo.toml
```

Expected: command exits 0.

- [ ] **Step 2: Run full project check**

Run:

```bash
nix develop -c task check
```

Expected: build passes and all tests pass.

- [ ] **Step 3: Inspect the focused diff**

Run:

```bash
git diff -- backend/src/db/models/video.rs backend/src/http/mod.rs backend/src/http/router.rs backend/src/http/video.rs
```

Expected: diff only contains the model read method, `/video/{key}` handler, route mount, and tests.
