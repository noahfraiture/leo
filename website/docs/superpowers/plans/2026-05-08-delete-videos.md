# Delete Videos Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a right-aligned delete button beside each uploaded video that removes both the metadata row and the SurrealDB bucket file, then refreshes the video picker fragment.

**Architecture:** Keep the existing `Video::delete(&self, db)` model method as the single operation that deletes metadata and file bytes. Add a small lookup by bucket key, a page-oriented UI delete route similar to `upload.rs`, and render each delete control as an HTMX `type="button"` because `video_selection()` is embedded inside other forms.

**Tech Stack:** Rust, axum UI route contract, axum `Path`, HTMX `hx-delete`, hypertext, SurrealDB file buckets, Taskfile workflow through `nix develop -c task ...`.

---

## File Structure

- Modify `backend/src/db/models/video.rs`: add `Video::find_by_file_key()` so the HTTP route can resolve the submitted bucket key to the full metadata record before calling `Video::delete()`.
- Create `backend/src/http/ui/features/delete.rs`: route and view for `DELETE /videos/{video_key}`; the fragment response reuses `home::video_selection()`.
- Modify `backend/src/http/ui/features/mod.rs`: register and re-export `DeleteVideoRoute`.
- Modify `backend/src/http/router.rs`: mount `DELETE /videos/{video_key}` and add focused integration coverage.
- Modify `backend/src/http/ui/features/home.rs`: restructure `video_option()` to place a delete button on the right without nesting a `<form>` inside the upload or analysis forms.
- Do not modify `AGENTS.md`; this plan follows the current repo conventions.

---

### Task 1: Add DB Lookup By File Key

**Files:**
- Modify: `backend/src/db/models/video.rs`

- [ ] **Step 1: Write the failing model test**

Add this test inside the existing `#[cfg(test)] mod tests` block:

```rust
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
```

- [ ] **Step 2: Run the model tests to verify they fail**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests::find_by_file_key
```

Expected: FAIL because `Video::find_by_file_key` does not exist.

- [ ] **Step 3: Add the query parameter type**

In `backend/src/db/models/video.rs`, near `UploadVideo` and `DeleteVideo`, add:

```rust
#[derive(SurrealValue)]
struct FindVideo {
    file: File,
}
```

- [ ] **Step 4: Implement the lookup method**

In the `impl Video` block, place this after `list()` and before `delete()`:

```rust
/// Returns one uploaded video metadata record by its bucket file key.
pub async fn find_by_file_key(
    db: &Database,
    key: &str,
) -> Result<Option<Video>, VideoError> {
    let mut response = db
        .query("SELECT * FROM video WHERE file = $file LIMIT 1;")
        .bind(FindVideo {
            file: File::new(VIDEO_BUCKET, key),
        })
        .await?;

    let mut videos: Vec<Video> = response.take(0)?;
    Ok(videos.pop())
}
```

- [ ] **Step 5: Run the model tests to verify they pass**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml db::models::video::tests::find_by_file_key
```

Expected: PASS.

- [ ] **Step 6: Commit**

Run:

```bash
git add backend/src/db/models/video.rs
git commit -m "feat: add video lookup by file key"
```

---

### Task 2: Add The Delete UI Route

**Files:**
- Create: `backend/src/http/ui/features/delete.rs`
- Modify: `backend/src/http/ui/features/mod.rs`

- [ ] **Step 1: Create the route module**

Create `backend/src/http/ui/features/delete.rs`:

```rust
use async_trait::async_trait;
use axum::extract::Path;
use hypertext::prelude::*;

use crate::{
    db,
    http::{
        router::AppState,
        ui::{Public, Route, RouteContext, RouteError, RouteView, document},
    },
};

use super::home::video_selection;

pub struct DeleteVideoRoute;

pub struct DeleteVideoView {
    videos: Vec<db::video::Video>,
}

#[async_trait]
impl Route for DeleteVideoRoute {
    type Input = Path<String>;
    type Authz = Public;
    type View = DeleteVideoView;

    async fn handle(
        context: &RouteContext,
        _granted: (),
        Path(video_key): Self::Input,
    ) -> Result<Self::View, RouteError> {
        let video = db::video::Video::find_by_file_key(context.state().db(), &video_key)
            .await?
            .ok_or(RouteError::BadRequest("video does not exist"))?;

        video.delete(context.state().db()).await?;
        let videos = db::video::Video::list(context.state().db()).await?;

        Ok(DeleteVideoView { videos })
    }
}

impl RouteView for DeleteVideoView {
    fn document(&self, state: &AppState) -> impl Renderable {
        document(
            state,
            "Video analysis | Videos",
            rsx! {
                <main class="mx-auto max-w-4xl space-y-8 p-6 lg:py-10">
                    <section class="space-y-6 rounded-box border border-base-300 bg-base-100 p-5 shadow-sm">
                        <h1 class="text-2xl font-semibold text-base-content">"Uploaded videos"</h1>
                        (video_selection(&self.videos))
                        <a class="btn btn-primary" href="/">"Back to analysis"</a>
                    </section>
                </main>
            },
        )
    }

    fn fragment(&self, _state: &AppState) -> impl Renderable {
        video_selection(&self.videos)
    }
}
```

- [ ] **Step 2: Export the route**

Update `backend/src/http/ui/features/mod.rs` to include the module and export:

```rust
mod analyze;
mod delete;
mod home;
mod upload;

// Pages
pub use analyze::AnalyzeRoute;
pub use delete::DeleteVideoRoute;
pub use home::{HomePage, healthz};
pub use upload::UploadVideoRoute;
```

- [ ] **Step 3: Check compilation for the new route module**

Run:

```bash
nix develop -c cargo check --manifest-path backend/Cargo.toml
```

Expected: PASS after Task 1 exists.

- [ ] **Step 4: Commit**

Run:

```bash
git add backend/src/http/ui/features/delete.rs backend/src/http/ui/features/mod.rs
git commit -m "feat: add video delete route"
```

---

### Task 3: Mount And Test The Delete Route

**Files:**
- Modify: `backend/src/http/router.rs`

- [ ] **Step 1: Mount the route**

In `app()`, add this route after the existing `POST /videos` route:

```rust
.route(
    "/videos/{video_key}",
    ui::route::<ui::features::DeleteVideoRoute>(MethodFilter::DELETE),
)
```

- [ ] **Step 2: Write the failing route test**

Inside `#[cfg(test)] mod tests`, add:

```rust
#[tokio::test]
async fn video_delete_route_removes_video_and_returns_updated_picker() {
    let state = AppState::for_test().await;
    let video = db::video::Video::upload(state.db(), "sample.mp4", b"video bytes".to_vec())
        .await
        .expect("video should upload");

    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("DELETE")
                .uri(format!("/videos/{}", video.file.key()))
                .header("HX-Request", "true")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let status = response.status();
    let html = response_text(response).await;

    assert_eq!(status, StatusCode::OK);
    assert!(html.contains(r#"id="video-selection""#));
    assert!(html.contains("No videos have been uploaded yet."));
    assert!(!html.contains("sample.mp4"));

    let videos = db::video::Video::list(state.db())
        .await
        .expect("videos should list");
    assert!(videos.is_empty());
}
```

- [ ] **Step 3: Run the route test**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml http::router::tests::video_delete_route_removes_video_and_returns_updated_picker
```

Expected: PASS once Tasks 1 and 2 are complete.

- [ ] **Step 4: Commit**

Run:

```bash
git add backend/src/http/router.rs
git commit -m "feat: mount video delete endpoint"
```

---

### Task 4: Render The Delete Button Beside Each Video

**Files:**
- Modify: `backend/src/http/ui/features/home.rs`
- Modify: `backend/src/http/router.rs`

- [ ] **Step 1: Extend the upload route test to capture the UI contract**

In `video_upload_route_returns_updated_video_picker`, add these assertions after the existing `name="video_keys"` assertion:

```rust
assert!(html.contains("Delete"));
assert!(html.contains(r#"type="button""#));
assert!(html.contains(r#"hx-delete="/videos/"#));
assert!(html.contains(r#"hx-target="#video-selection""#));
```

- [ ] **Step 2: Run the test to verify it fails**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml http::router::tests::video_upload_route_returns_updated_video_picker
```

Expected: FAIL because the rendered video option has no delete button.

- [ ] **Step 3: Replace `video_option()` markup**

Replace the current `video_option()` function in `backend/src/http/ui/features/home.rs` with:

```rust
pub(super) fn video_option(video: &db::video::Video) -> impl Renderable {
    let delete_path = format!("/videos/{}", video.file.key());
    let delete_label = format!("Delete {}", video.name);
    let delete_confirm = format!("Delete {}?", video.name);

    rsx! {
        <div class="flex items-center gap-2 rounded-box border border-base-300 hover:bg-base-200">
            <label class="flex min-w-0 flex-1 cursor-pointer items-center gap-3 p-3">
                <input
                    class="checkbox checkbox-primary"
                    type="checkbox"
                    name="video_keys"
                    value=(video.file.key()) />
                <span class="min-w-0 flex-1">
                    <span class="block truncate text-sm font-medium text-base-content">
                        (video.name.as_str())
                    </span>
                    <span class="block text-xs text-base-content/60">
                        (format!("{} bytes", video.size))
                    </span>
                </span>
            </label>

            <button
                class="btn btn-ghost btn-sm mr-2 text-error hover:bg-error hover:text-error-content"
                type="button"
                aria-label=(delete_label)
                hx-delete=(delete_path)
                hx-target="#video-selection"
                hx-swap="outerHTML"
                hx-confirm=(delete_confirm)>
                "Delete"
            </button>
        </div>
    }
}
```

- [ ] **Step 4: Run the UI route test**

Run:

```bash
nix develop -c cargo test --manifest-path backend/Cargo.toml http::router::tests::video_upload_route_returns_updated_video_picker
```

Expected: PASS.

- [ ] **Step 5: Commit**

Run:

```bash
git add backend/src/http/ui/features/home.rs backend/src/http/router.rs
git commit -m "feat: add video delete button"
```

---

### Task 5: Final Verification

**Files:**
- Verify all changed files.

- [ ] **Step 1: Run formatting**

Run:

```bash
nix develop -c cargo fmt --manifest-path backend/Cargo.toml
```

Expected: command exits successfully and formats Rust files.

- [ ] **Step 2: Run the project test task**

Run:

```bash
nix develop -c task test
```

Expected: all backend tests pass.

- [ ] **Step 3: Run the project check task**

Run:

```bash
nix develop -c task check
```

Expected: build and tests pass.

- [ ] **Step 4: Inspect the final diff**

Run:

```bash
git diff --stat
git diff -- backend/src/db/models/video.rs backend/src/http/ui/features/delete.rs backend/src/http/ui/features/mod.rs backend/src/http/router.rs backend/src/http/ui/features/home.rs
```

Expected: diff only contains the planned lookup method, delete route, route mount/tests, and right-aligned delete button.

- [ ] **Step 5: Commit verification fixes if needed**

If formatting changed files or verification exposed small fixes, run:

```bash
git add backend/src/db/models/video.rs backend/src/http/ui/features/delete.rs backend/src/http/ui/features/mod.rs backend/src/http/router.rs backend/src/http/ui/features/home.rs
git commit -m "fix: polish video delete flow"
```

If no files changed after verification, do not create this commit.

---

## Self-Review

- Spec coverage: The plan adds a delete button on the right, creates a delete feature similar to `upload.rs`, and uses the existing `Video::delete()` method that removes both the metadata row and bucket file.
- HTML correctness: The delete button is not wrapped in a nested form, because `video_selection()` appears inside both upload and analysis forms.
- Tests: Existing model coverage already proves bucket-file deletion. New tests cover lookup, route behavior, fragment refresh, and button rendering.
- Repository conventions: The route uses native axum `Path`, follows page-oriented UI feature files, keeps shared rendering on the existing `video_selection()` fragment, and runs through `nix develop -c task ...`.
