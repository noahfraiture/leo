# Synology Module Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Refactor the Synology crate into small Axum-oriented modules without changing its HTTP behavior.

**Architecture:** `server.rs` mounts an `api::router`; typed endpoint request structs feed responsibility-specific handlers; `api::error` converts failures to Synology JSON. The single entry request accumulates all supported entry parameters with typed values.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio 1, Serde 1, thiserror 2, Tower 0.5

## Global Constraints

- Preserve all current routes, JSON payloads, error codes, error precedence, reachability behavior, and recording mutation.
- Use typed Axum `Query` extraction; do not pass request parameters as `HashMap<String, String>`.
- Keep one accumulated `EntryRequest` schema for all `entry.cgi` parameters.
- Hold no mutex guard across an await.
- Delete production-only test seams and redundant wrappers rather than replacing them with abstractions.
- Add no dependency.
- Make exactly one final commit using `refactor(synology): organize API modules`.

---

### Task 1: Organize Synology API Modules

**Files:**
- Modify: `AGENTS.md`
- Modify: `synology/src/lib.rs`
- Modify: `synology/src/server.rs`
- Modify: `synology/src/camera.rs`
- Modify: `synology/Cargo.toml`
- Delete: `synology/src/api.rs`
- Delete: `synology/src/error.rs`
- Delete: `synology/src/cli.rs`
- Create: `synology/src/api/mod.rs`
- Create: `synology/src/api/error.rs`
- Create: `synology/src/api/info.rs`
- Create: `synology/src/api/entry.rs`
- Create: `synology/src/api/camera.rs`
- Create: `synology/src/api/external_recording.rs`
- Test: API module test blocks and `synology/src/server.rs`

**Interfaces:**
- `server::start(Vec<Camera>, SocketAddr) -> std::io::Result<()>`
- `server::app(Vec<Camera>) -> Router`
- `api::router() -> Router<CameraState>`
- `CameraState = Arc<Mutex<Vec<Camera>>>`
- `info::handle(Result<Query<InfoRequest>, QueryRejection>) -> Result<Response, ApiError>`
- `entry::handle(State<CameraState>, Result<Query<EntryRequest>, QueryRejection>) -> Result<Response, ApiError>`

- [ ] **Step 1: Strengthen request-parsing regression coverage**

Add tests proving missing common fields return 101, unknown APIs return 102, unknown methods return 103, unsupported string versions return 104, malformed camera IDs and actions return 401, and camera reachability/recording behavior remains unchanged.

- [ ] **Step 2: Run the Synology tests before refactoring**

Run: `cargo test -p synology`

Expected: all baseline tests pass; newly added typed-structure tests fail to compile until the API modules exist.

- [ ] **Step 3: Add the error ownership rule**

Append to `AGENTS.md`:

```text
When a module is large enough to become a directory, keep its non-trivial error types and response conversions in that module's `error.rs`. Do not create a crate-level `error.rs` solely to wrap a standard-library error.
```

- [ ] **Step 4: Implement the API module tree**

Create `api/mod.rs` with private API submodules, `CameraState`, the `/query.cgi` and `/entry.cgi` router, and the shared success envelope. Create `api/error.rs` with `ApiError`, response DTOs, `From<QueryRejection>`, and `IntoResponse`.

Create typed endpoint schemas:

```rust
struct InfoRequest {
    api: String,
    method: String,
    version: String,
    query: String,
}

struct EntryRequest {
    api: String,
    method: String,
    version: String,
    camera_id: Option<CameraId>,
    action: Option<RecordingAction>,
}
```

`CameraId` must distinguish a valid `u32` from malformed input so malformed IDs still produce 401. `RecordingAction` must distinguish `start`, `stop`, and invalid values. Add future API parameters directly to `EntryRequest`.

Move information discovery to `info.rs`, entry dispatch to `entry.rs`, camera-list DTOs and handling to `api/camera.rs`, and recording DTOs/handling to `api/external_recording.rs`.

- [ ] **Step 5: Simplify server and camera responsibilities**

Reduce `server.rs` to listener lifecycle, API nesting, and state construction. Return `std::io::Result<()>` directly and delete `app_with_state`.

Move the shared 250 ms TCP reachability probe to `Camera::reachable`. Ensure handlers clone required camera data while locked and release the lock before awaiting.

- [ ] **Step 6: Remove obsolete modules and exports**

Delete crate-level `api.rs`, `error.rs`, empty `cli.rs`, `Error`, and `Result`. Keep only `mod api`, `pub mod camera`, and `pub mod server` in `lib.rs` plus necessary tests that are not better colocated.

- [ ] **Step 7: Verify**

Run: `cargo fmt --all -- --check`

Run: `cargo test -p synology`

Run: `cargo clippy -p synology --all-targets -- -D warnings`

Run: `cargo test --workspace`

Run: `git diff --check`

Expected: all commands succeed with no Synology warnings and no changed HTTP behavior.

- [ ] **Step 8: Review and commit once**

Inspect `git status`, `git diff`, and `git log --oneline -10`. Stage `AGENTS.md`, the approved spec and plan, and only Synology crate changes. Do not stage unrelated workspace changes.

Commit:

```bash
git commit -m "refactor(synology): organize API modules"
```
