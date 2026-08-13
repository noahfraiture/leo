# Host Recording Operator Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a complete two-camera desktop workflow that records RTSP feeds into session-local MKV segments, survives temporary camera disconnects, and analyzes those local files directly without Synology or NAS code.

**Architecture:** Extract reusable session, recording, and analysis code into `backend`. A dedicated recorder runtime thread owns one reconnecting FFmpeg supervisor per camera and finalizes session-local MKV segments; the analysis facade discovers those files, skips physical recording gaps, and extracts frames directly. Dioxus owns only shared workflow state and root-scoped commands, while the desktop event-loop owner retains and shuts down RecorderRuntime, MediaMTX, and the log writer.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7.9, Tokio 1.53, Tailwind CSS 4, daisyUI, FFmpeg/FFprobe, MediaMTX 1.18.2, `ffmpeg-sidecar` 2.5.2, Rig 0.41, Serde/JSON, tracing, Matroska, SHA-256.

**Specification:** `docs/superpowers/specs/2026-08-12-host-recording-operator-workflow-design.md`

## Global Constraints

- Remove the complete `synology` crate and app Synology client; do not preserve compatibility aliases, dormant features, or a generic recording-source abstraction.
- Record every configured camera only while a software session is active, independent of analysis participation.
- Require every configured camera to produce media before writing the session-start event.
- Use RTSP/TCP, checked `-timeout` microseconds, video-only stream copy, and Matroska; never transcode recorder media.
- Do not pass `-nostats`; parsed FFmpeg progress is the readiness and media-timeline source.
- Retry a post-readiness disconnect every one second until Stop; later recovered segments must be analyzed.
- Default `LEO_CAMERA_CONFIG=./cameras.json`, `LEO_DATA_DIR=./data`, and `LEO_RECORDER_TIMEOUT_SECS=10`.
- Store the complete portable session under `<data>/sessions/<UTC-ms>/` with `events.jsonl`, `recordings/`, `recording-complete`, and optional `analysis.json`.
- A session is discoverable only when both its ended event log and zero-byte `recording-complete` marker are valid regular no-follow files.
- Ignore `.partial.mkv` during analysis; retain invalid non-empty attempts for diagnosis and remove empty attempts.
- Skip uncovered camera samples, preserve partial frame sets, and continue after gaps; fail only on overlapping segments or a completely empty frame plan.
- Persist one `RecordingGap` warning for every contiguous camera gap, including gaps during disabled analysis participation.
- Analyzer batches exactly five frame sets and saves an initial zero-response checkpoint before provider construction.
- Checkpoint fingerprinting includes stable segment UTC bounds but excludes absolute paths so sessions can move to an SSD.
- Keep one analysis job at a time; recording and analysis do not run concurrently in this increment.
- Use native semantic controls, Tailwind utilities, and daisyUI; keep presentation sparse and responsive.
- Never log RTSP URLs or credentials, API keys, prompts/checklists, image bytes, or model request bodies.
- Emit sanitized `tracing` events at the owning boundary for configuration, preview startup, recorder lifecycle, session lifecycle, catalogue skips, segment/gap discovery, and analysis planning/batches/failure/completion.
- Paid model execution requires feature `paid-openai-test`, exact ignored-test filtering, `LEO_RUN_PAID_OPENAI_TEST=1`, and explicit approval.
- Do not implement crash recovery, orphan cleanup after hard termination, continuous all-day recording, retention, deletion, export, settings, discovery, playback, cancellation, or concurrent sessions/analyses.
- For reorganized modules use private items or plain `pub`; keep child modules private and re-export only documented APIs.
- Keep `mod.rs` files declaration/re-export-only and keep non-trivial module errors in `error.rs` using `thiserror`.
- Do not touch unrelated worktree changes, especially `app/src/analysis/video/video.rs` changes that predate implementation, unless the implementation task necessarily incorporates them.
- Do not run blanket ignored tests or the paid test without approval.
- Implementation commits are optional suggestions only; create them only with explicit authorization beyond the already-authorized specification/plan commit.

## Locked File Structure

```text
app/
|-- AGENTS.md
|-- Cargo.toml
|-- cameras.json
|-- justfile
|-- backend/
|   |-- Cargo.toml
|   `-- src/
|       |-- lib.rs
|       |-- session/
|       |   |-- mod.rs
|       |   |-- controller.rs
|       |   |-- session.rs
|       |   |-- catalog.rs
|       |   `-- error.rs
|       |-- recording/
|       |   |-- mod.rs
|       |   |-- recorder.rs
|       |   |-- segment.rs
|       |   `-- error.rs
|       `-- analysis/
|           |-- mod.rs
|           |-- facade.rs
|           |-- error.rs
|           |-- agent/
|           |-- analyzer/
|           `-- video/
|-- app/
|   |-- Cargo.toml
|   `-- src/
|       |-- main.rs
|       |-- lib.rs
|       |-- analysis_task.rs
|       |-- camera_config.rs
|       |-- logging.rs
|       |-- paid_openai_workflow.rs
|       |-- session_task.rs
|       |-- workflow/
|       |-- preview/
|       |-- components/
|       `-- views/
`-- camera/
    |-- fixtures/
    `-- tests/
```

---

### Task 1: Extract The Reusable Backend Crate

**Files:**
- Create: `backend/Cargo.toml`
- Create: `backend/src/lib.rs`
- Move: `app/src/session/` -> `backend/src/session/`
- Move: `app/src/recording/` -> `backend/src/recording/`
- Move: `app/src/analysis/` -> `backend/src/analysis/`
- Modify: `Cargo.toml`
- Modify: `app/Cargo.toml`
- Modify: `app/src/main.rs`
- Modify: `AGENTS.md`

**Interfaces:**
- Consumes: the complete merged backend and its existing 115 normal app tests.
- Produces: `backend::{session, recording, analysis}` with identical behavior before local-recording changes.

- [ ] **Step 1: Record the baseline**

Run:

```bash
cargo fmt --all --check
cargo test --workspace --all-targets
```

Expected: formatting passes; 176 normal tests pass, seven ignored tests remain ignored, and no paid request runs.

- [ ] **Step 2: Add shared workspace dependencies**

Add `backend` to workspace members and declare dependencies shared by app/backend in root `Cargo.toml`:

```toml
[workspace.dependencies]
axum = "0.8.9"
clap = { version = "=4.6.1", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
thiserror = "2"
tokio = "1.53.0"
tower = { version = "0.5.3", features = ["util"] }
tracing = "0.1"
url = "2"
uuid = { version = "1.24.0", features = ["serde", "v4"] }
```

Keep camera dependencies on these workspace entries.

- [ ] **Step 3: Create the backend manifest and root module**

Create `backend/Cargo.toml` with the dependencies currently used by moved code:

```toml
[package]
name = "backend"
version = "0.1.0"
edition = "2024"

[dependencies]
base64 = "0.22"
ffmpeg-sidecar = { version = "=2.5.2", default-features = false }
reqwest = { version = "0.13.4", features = ["json", "stream"] }
rig-core = "0.41.0"
serde = { workspace = true }
serde_json = { workspace = true }
tempfile = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true, features = ["fs", "io-util", "rt", "sync"] }
tracing = { workspace = true }
url = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
axum = { workspace = true }
futures-util = "0.3"
rig-core = { version = "0.41.0", features = ["test-utils"] }
tokio = { workspace = true, features = ["macros", "rt"] }

[features]
paid-openai-test = []
test-support = []
```

Create:

```rust
//! Reusable local session, recording, and video-analysis backend.

pub mod analysis;
pub mod recording;
pub mod session;
```

- [ ] **Step 4: Move the modules mechanically**

Move all three directories and update moved imports from `crate::{analysis, recording, session}` to the backend crate root. Remove their `mod` declarations from `app/src/main.rs` and add to app dependencies:

```toml
backend = { path = "../backend" }
```

Do not change assertions or Synology behavior in this step.

- [ ] **Step 5: Gate the existing paid test before normal verification**

Add `#[cfg(feature = "paid-openai-test")]` to the existing paid Analyzer test and imports used only by it. Keep `#[ignore]` and the `LEO_RUN_PAID_OPENAI_TEST` assertion. This feature is temporary until Task 6 deletes the old paid test.

- [ ] **Step 6: Expose only the session API required by later tasks**

In `backend/src/session/mod.rs` re-export:

```rust
pub use controller::{OperatorAction, SessionController};
pub use error::Error;
pub use session::{Session, SessionCamera};
```

Make `OperatorAction`, `Session`, `SessionCamera`, and their data fields plain `pub`. Make `SessionController` and its `create`, `apply`, and `elapsed` methods plain `pub`, but keep its `SessionLog` field and all persistence internals private. Make `Session::load` plain `pub`. Add:

```rust
/// Returns monotonic elapsed time since the session-start event was written.
pub fn elapsed(&self) -> Duration {
    self.log.started_at.elapsed()
}
```

Keep persisted event DTOs and helpers private inside private child modules.

- [ ] **Step 7: Update repository rules for the approved reorganization**

Replace the conflicting narrow-visibility rule in `AGENTS.md` with:

```markdown
- In new or substantially reorganized modules, use private items or plain `pub`; do not use restricted `pub(...)` visibility. Keep child modules private and expose only documented module APIs.
- Never set `LEO_RUN_PAID_OPENAI_TEST=1` or run a paid model test without explicit user approval. Compile-only feature checks are allowed. Never run blanket ignored tests; filter approved ignored tests by exact name.
```

- [ ] **Step 8: Verify the mechanical extraction**

Run:

```bash
cargo fmt --all --check
cargo test -p backend
cargo test -p app
cargo test --workspace --all-targets
```

Expected: moved tests pass without behavior changes; app compiles against backend; no duplicate backend modules remain in app.

Suggested commit if separately authorized: `refactor(backend): extract reusable backend`

---

### Task 2: Add Marker-Gated Session Discovery

**Files:**
- Create: `backend/src/session/catalog.rs`
- Modify: `backend/src/session/mod.rs`
- Modify: `backend/src/session/error.rs`

**Interfaces:**
- Consumes: strict `Session::load` replay.
- Produces: `StoredSession`, `list_sessions`, and `mark_recording_complete`.

```rust
#[derive(Debug)]
pub struct StoredSession {
    pub directory: PathBuf,
    pub session: Session,
}

pub fn list_sessions(root: &Path) -> Result<Vec<StoredSession>, Error>;
pub fn mark_recording_complete(directory: &Path) -> Result<(), Error>;
```

- [ ] **Step 1: Write failing catalog and public-boundary tests**

Create `catalog.rs`, declare `mod catalog;` in `session/mod.rs`, and reuse the session test JSON builders to add these tests:

```rust
#[test]
fn missing_root_returns_no_sessions();

#[test]
fn catalogue_requires_ended_log_and_completion_marker();

#[test]
fn catalogue_skips_active_malformed_nested_and_unrelated_entries();

#[cfg(unix)]
#[test]
fn catalogue_rejects_symlinked_directories_events_and_markers();

#[test]
fn catalogue_sorts_newest_first_by_start_and_uuid();

#[test]
fn mark_recording_complete_uses_create_new_and_zero_bytes();

#[test]
fn regular_file_root_returns_io_error();

#[test]
fn session_controller_rejects_empty_and_zero_id_camera_lists();

#[cfg(unix)]
#[test]
fn session_load_rejects_a_symlinked_events_file();
```

The valid fixtures must contain direct child directories with newline-terminated start/end logs and zero-byte markers. Assert nested valid logs are not discovered.

- [ ] **Step 2: Run the focused tests and verify red**

Run:

```bash
cargo test -p backend session::catalog -- --nocapture
```

Expected: compilation fails because the declared `catalog` module imports APIs that do not exist yet.

- [ ] **Step 3: Implement no-follow discovery**

Use `std::fs::symlink_metadata` for the root, each direct child, `events.jsonl`, and `recording-complete`. A valid row requires:

```rust
child_metadata.file_type().is_dir()
    && events_metadata.file_type().is_file()
    && marker_metadata.file_type().is_file()
    && marker_metadata.len() == 0
```

Scan only direct child directories. Missing root returns `Ok(vec![])`. Emit sanitized `tracing::warn!` events for invalid/active child skips and one `tracing::debug!` event for unrelated entries; include paths but never event contents. Sort descending by `(session.start_utc_ms, session.id)`.

Make `Session::load` reject anything except a direct regular no-follow `events.jsonl`, and make `SessionController::create` reject an empty camera list or camera ID zero before opening the destination. Existing duplicate-ID and cadence validation remains unchanged.

- [ ] **Step 4: Implement atomic marker creation**

Create `<directory>/recording-complete` with:

```rust
let file = OpenOptions::new().write(true).create_new(true).open(&path)?;
file.sync_all()?;
File::open(directory)?.sync_all()?;
```

Reject a symlinked/non-directory session path before creation. Re-export all three public catalog items from `session/mod.rs`.

- [ ] **Step 5: Verify session discovery**

Run:

```bash
cargo fmt --all --check
cargo test -p backend session::catalog -- --nocapture
cargo test -p backend session:: -- --nocapture
```

Expected: catalog and existing session durability tests pass.

Suggested commit if separately authorized: `feat(backend): discover completed sessions`

---

### Task 3: Add Local MKV Segment Discovery

**Files:**
- Create: `backend/src/recording/segment.rs`
- Modify: `backend/src/recording/mod.rs`
- Modify: `backend/src/recording/error.rs`

**Interfaces:**
- Produces: finalized segment discovery and one shared FFprobe parser used by Task 7.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSegment {
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
    pub path: PathBuf,
}

pub fn list_segments(
    recordings_root: &Path,
    camera_ids: &[u32],
) -> Result<Vec<RecordingSegment>, Error>;
```

Private sibling API shared with recorder finalization:

```rust
pub struct ProbedMedia {
    pub start_time_ms: i64,
    pub media_span_ms: i64,
}

pub fn probe_media(
    ffprobe: &Path,
    path: &Path,
    timeout: Duration,
    shutdown: &AtomicBool,
) -> Result<ProbedMedia, Error>;
```

`segment` remains a private child module and `probe_media` is not re-exported, so this plain `pub` is only a sibling-module implementation boundary.

- [ ] **Step 1: Declare the module and write fake-FFprobe parsing tests**

Create `segment.rs`, declare `mod segment;` in `recording/mod.rs`, and add a Unix test helper that writes an executable shell script whose stdout is supplied JSON. Add:

```rust
#[test]
fn probe_rounds_start_down_and_duration_up() {
    // start_time 0.0679, duration 2.0001 -> start 67, span 1934.
}

#[test]
fn probe_rejects_non_finite_non_positive_and_multiple_video_streams();

#[test]
fn successful_malformed_probe_json_is_fatal();

#[test]
fn unsuccessful_probe_is_invalid_media();

#[test]
fn hanging_probe_is_killed_reaped_and_times_out();

#[test]
fn shutdown_kills_and_reaps_an_active_probe();
```

Assert unknown top-level FFprobe fields such as `programs` and `stream_groups` are accepted.

- [ ] **Step 2: Write failing discovery tests**

Create camera directories and fake numeric MKVs, then add:

```rust
#[test]
fn list_segments_ignores_partial_and_unrelated_files();

#[test]
fn list_segments_accepts_an_existing_empty_camera_directory();

#[test]
fn list_segments_rejects_missing_camera_directory();

#[cfg(unix)]
#[test]
fn list_segments_rejects_symlinked_roots_directories_and_entries();

#[test]
fn list_segments_rejects_duplicate_camera_ids();

#[test]
fn list_segments_rejects_empty_and_zero_camera_ids();

#[test]
fn list_segments_rejects_overlapping_intervals();

#[test]
fn list_segments_sorts_by_camera_and_start();
```

- [ ] **Step 3: Run tests and verify red**

Run:

```bash
cargo test -p backend recording::segment -- --nocapture
```

Expected: compilation fails because the declared module's local segment types and discovery functions do not exist.

- [ ] **Step 4: Implement exact FFprobe parsing**

Run:

```text
ffprobe -v error
  -select_streams v
  -show_entries stream=index:format=start_time,duration
  -of json
  <segment>
```

Deserialize strings without `deny_unknown_fields`:

```rust
#[derive(Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}

#[derive(Deserialize)]
struct ProbeStream { index: u32 }

#[derive(Deserialize)]
struct ProbeFormat {
    start_time: String,
    duration: String,
}
```

Require exactly one video stream, finite `start >= 0`, finite `duration > 0`, and checked bounds. Compute:

```rust
let start_time_ms = checked_floor_millis(start)?;
let duration_ms = checked_ceil_millis(duration)?;
let media_span_ms = duration_ms
    .checked_sub(start_time_ms)
    .filter(|span| *span > 0)
    .ok_or(Error::InvalidMediaDuration)?;
```

A nonzero probe exit is `Error::InvalidMedia`; successful malformed JSON is `Error::ProbeJson`. Do not use `ffmpeg_sidecar::ffprobe::ffprobe` because its blocking `output()` cannot be interrupted. Spawn `ffprobe` with piped stdout, drain stdout on one reader thread, and poll `try_wait()` every 50 ms. On timeout or `shutdown`, kill and wait for the child, then join the reader before returning the first error. Never return while the child or reader remains live.

- [ ] **Step 5: Implement direct no-follow discovery**

Production `list_segments` uses `ffmpeg_sidecar::ffprobe::ffprobe_path`; tests call private `list_segments_with_ffprobe`. Reject an empty camera list, ID zero, and duplicate IDs. Validate a regular no-follow root and direct `camera-<id>` directories. Reject any symlink direct entry. Accept only regular files whose exact extension is `.mkv` and whose stem parses as `i64`; ignore `.partial.mkv`, unrelated files, and nested directories. Analysis probes use a fixed ten-second timeout and a local never-set shutdown flag; analysis cancellation remains out of scope.

For every accepted file:

```rust
let probe = probe_media(ffprobe, &path, Duration::from_secs(10), &shutdown)?;
let end_utc_ms = start_utc_ms
    .checked_add(probe.media_span_ms)
    .ok_or(Error::TimestampOverflow)?;
```

Sort by `(camera_id, start_utc_ms)`. Reject same-camera overlaps; adjacency is valid. Re-export `RecordingSegment` and `list_segments` only.

Emit one sanitized discovery event with camera/segment counts and one warning event for each invalid finalized file. Log paths and stable camera IDs only, never source URLs.

- [ ] **Step 6: Verify local discovery**

Run:

```bash
cargo fmt --all --check
cargo test -p backend recording::segment -- --nocapture
```

Expected: all probe, no-follow, ordering, and overlap tests pass.

Suggested commit if separately authorized: `feat(backend): discover local recording segments`

---

### Task 4: Replace Synology Frames With Gap-Tolerant Local Frames

**Files:**
- Modify: `backend/src/analysis/video/video.rs`
- Modify: `backend/src/analysis/video/error.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Modify: `backend/src/analysis/analyzer/error.rs`

**Interfaces:**
- Consumes: `RecordingSegment` from Task 3.
- Produces: path-backed frames, physical recording-gap warnings, partial frame sets, and direct JPEG extraction.

- [ ] **Step 1: Write failing gap and frame tests**

Replace `sequences_reject_missing_recording_coverage` and Synology `Video` fixtures with segment fixtures. Add:

```rust
#[test]
fn sequence_skips_missing_samples_and_resumes_after_the_gap() {
    // Schedule offsets 0,1,2,3; segments cover 0 and 3.
    // Frames retain sample_index 0 and 3.
}

#[test]
fn missing_camera_frame_keeps_the_other_camera_frame_set();

#[test]
fn sequence_still_rejects_overlapping_segments();

#[test]
fn gaps_before_between_and_after_segments_are_coalesced();

#[test]
fn disabled_participation_does_not_hide_a_physical_recording_gap();

#[test]
fn camera_without_segments_gets_one_full_session_gap();
```

- [ ] **Step 2: Run focused tests and verify red**

Run:

```bash
cargo test -p backend analysis::video::video -- --nocapture
```

Expected: old `Video`/recording-ID planning cannot satisfy the segment and gap assertions.

- [ ] **Step 3: Replace frame identity**

Delete `Video` and define internal frames:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub camera_id: u32,
    pub segment_start_utc_ms: i64,
    pub segment_end_utc_ms: i64,
    pub sample_index: usize,
    pub session_offset: Duration,
    pub recording_offset: Duration,
    pub path: PathBuf,
}
```

`SampleSequence::from_segments` enumerates the complete scheduled offsets. Zero covering segments skips only that sample; one adds a frame; multiple returns overlap. Use checked `sample_utc_ms - segment.start_utc_ms` for the recording offset. Continue through every later sample.

- [ ] **Step 4: Add physical gap derivation**

Define and document:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisWarning {
    RecordingGap {
        camera_id: u32,
        start_offset_ms: u64,
        end_offset_ms: u64,
    },
}
```

For every session camera, clip its sorted segment coverage to the half-open session UTC interval and take the complement. Do not inspect sampling participation. Sort warnings by `(camera_id, start_offset_ms, end_offset_ms)`.

Emit one sanitized event for every derived physical gap with only session ID, camera ID, and offset bounds.

- [ ] **Step 5: Convert Analyzer to local direct materialization**

Change Analyzer planning to accept `Vec<RecordingSegment>` rather than a `SynologyClient`. Delete duplicate recording-ID checks, `DownloadWindow`, `DownloadedVideo`, `batch_windows`, `download_batch`, and local download-offset translation. Store warnings beside frame sets.

For each frame:

```rust
let path = frame.path.clone();
let offset = frame.recording_offset;
let jpeg = tokio::task::spawn_blocking(move || extract_jpeg(&path, offset)).await??;
```

Keep temporary JPEG behavior. Return `NoAnalyzableFrames` only when merged frame sets are empty.

- [ ] **Step 6: Rewrite Analyzer tests without HTTP**

Delete Axum List/Download helpers and HTTP query assertions. Keep prompt order, batch ordering, model failure, save rollback, completion, and resume-context assertions with local segment paths. Replace download failure coverage with:

```rust
#[tokio::test]
async fn extraction_failure_precedes_model_invocation();
```

Assert the mock model receives no request when a local segment is invalid.

- [ ] **Step 7: Verify local gap-tolerant planning**

Run:

```bash
cargo fmt --all --check
cargo test -p backend analysis::video -- --nocapture
cargo test -p backend analysis::analyzer::analyzer -- --nocapture
```

Expected: post-gap frames are present, partial sets survive, overlap remains fatal, and no HTTP process is used.

Suggested commit if separately authorized: `feat(backend): analyze local segments across gaps`

---

### Task 5: Add Checkpoint V2 And Path-Independent Plan Identity

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/analysis/analyzer/progress.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Move: `backend/src/analysis/analyzer/error.rs` -> `backend/src/analysis/error.rs`
- Modify: `backend/src/analysis/analyzer/mod.rs`
- Modify: `backend/src/analysis/agent/agent.rs`
- Modify: `backend/src/analysis/agent/mod.rs`
- Modify: `backend/src/analysis/video/mod.rs`
- Modify: `backend/src/analysis/mod.rs`

**Interfaces:**
- Produces: public model DTOs, `AnalysisCheckpoint::read`, initial checkpoint persistence, stable SHA-256 plan validation, and lazy provider use.

- [ ] **Step 1: Add SHA-256 and failing v2 tests**

Add `sha2 = "0.10"` to backend. Replace v1 checkpoint tests with:

```rust
#[test]
fn checkpoint_v2_round_trips_checklist_fingerprint_warnings_and_responses();

#[test]
fn read_rejects_wrong_schema_session_empty_identity_and_excess_responses();

#[cfg(unix)]
#[test]
fn read_rejects_a_symlinked_checkpoint();

#[test]
fn resume_rejects_changed_checklist_plan_or_warnings();

#[test]
fn fingerprint_is_independent_of_absolute_paths();

#[test]
fn fingerprint_encoding_is_stable();

#[tokio::test]
async fn initial_checkpoint_exists_before_provider_or_extraction_failure();
```

The golden frame uses batch size `5`, frame-set offset `1000`, camera `2`, segment `1786204800000..1786204805000`, sample index `3`, and recording offset `250`.

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cargo test -p backend analysis::analyzer::progress -- --nocapture
cargo test -p backend fingerprint_encoding_is_stable -- --nocapture
```

Expected: v1 lacks checklist, warnings, fingerprint, and direct response fields.

- [ ] **Step 3: Replace the checkpoint DTO**

Implement schema `2`:

```rust
pub const ANALYSIS_SCHEMA_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCheckpoint {
    pub schema_version: u8,
    pub session_id: Uuid,
    pub checklist: String,
    pub plan_fingerprint: String,
    pub total_batches: usize,
    pub warnings: Vec<AnalysisWarning>,
    pub responses: Vec<AnalysisResponse>,
}

impl AnalysisCheckpoint {
    pub fn read(path: &Path, expected_session_id: Uuid) -> Result<Self, Error>;
}
```

Delete `CompletedBatch`; response vector position is the batch index. `read` first requires `symlink_metadata(path).file_type().is_file()`, then rejects wrong schema/UUID, empty checklist/fingerprint, and `responses.len() > total_batches`. Private `load_or_new` additionally validates exact checklist, fingerprint, batch count, and warnings.

- [ ] **Step 4: Implement canonical plan hashing**

Hash this exact byte stream:

```text
ASCII "leo-analysis-plan-v2\0"
batch size                 u64 little-endian
frame-set count            u64 little-endian
for each frame set:
  session offset ms        u64 little-endian
  frame count              u64 little-endian
  for each frame:
    camera ID               u32 little-endian
    segment start UTC ms    i64 little-endian
    segment end UTC ms      i64 little-endian
    sample index            u64 little-endian
    recording offset ms     u64 little-endian
```

Use checked conversions and lowercase hex. Never hash paths, warnings, checklist, JPEGs, or responses. The golden digest must equal:

```text
2e61898616fe0b02dda021e2bc83131bd38ec7e2fb1681f051934ee9a3ef288a
```

- [ ] **Step 5: Save initial state and make Analyzer provider-independent**

Make `Analyzer` non-generic. Its `resume` plans frames, computes identity, loads/creates checkpoint, and immediately saves a missing zero-response checkpoint before returning. Existing checkpoints are not rewritten during planning.

Move the model parameter to:

```rust
async fn analyze_next<M: CompletionModel>(
    &mut self,
    agent: &Agent<M>,
) -> Result<&AnalysisResponse, Error>;
```

Push one response, atomically save, and pop on save failure. Update old tests that asserted no checkpoint after model/download failures to assert a readable zero-response v2 checkpoint.

Emit sanitized analysis planning, batch-start, checkpoint-save, failure, resume, and completion events. Include session ID and numeric batch counts only; never checklist/prompt text, image data, provider bodies, or credentials.

- [ ] **Step 6: Expose legal public analysis DTOs and errors**

Make `Observation`, `ChecklistProgress`, `AnalysisResponse`, their fields, `AnalysisWarning`, and `AnalysisCheckpoint` plain `pub`. Keep Agent/Analyzer private. Move the aggregate public `thiserror::Error` to `analysis/error.rs`, and re-export from `analysis/mod.rs`:

```rust
pub use agent::{AnalysisResponse, ChecklistProgress, Observation};
pub use analyzer::{AnalysisCheckpoint, AnalysisWarning};
pub use error::Error;
```

Keep `analysis/mod.rs` declaration/re-export-only.

- [ ] **Step 7: Verify v2 durability and rollback**

Run:

```bash
cargo fmt --all --check
cargo test -p backend analysis::analyzer::progress -- --nocapture
cargo test -p backend analysis::analyzer::analyzer -- --nocapture
```

Expected: v2 identity validation, initial save, model failure, save rollback, completion, and resume all pass.

Suggested commit if separately authorized: `feat(backend): checkpoint local analysis plans`

---

### Task 6: Add The Local Analysis Facade And Delete Synology

**Files:**
- Create: `backend/src/analysis/facade.rs`
- Modify: `backend/src/analysis/mod.rs`
- Modify: `backend/src/analysis/error.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Delete: `backend/src/recording/synology.rs`
- Delete: Synology-only portions of `backend/src/recording/error.rs`
- Delete: `synology/`
- Modify: `Cargo.toml`
- Modify: `backend/Cargo.toml`
- Modify: `app/Cargo.toml`
- Modify: `justfile`

**Interfaces:**
- Produces: directory-based `analyze_session`, direct local extraction, and a three-crate workspace.

```rust
pub struct AnalyzeSession {
    pub directory: PathBuf,
    pub checklist: String,
}

pub async fn analyze_session(
    request: AnalyzeSession,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint, Error>;
```

- [ ] **Step 1: Write failing facade-order tests**

Inside private facade tests, add:

```rust
#[tokio::test]
async fn empty_checklist_fails_before_filesystem_access();

#[tokio::test]
async fn missing_or_symlinked_marker_is_rejected();

#[cfg(unix)]
#[tokio::test]
async fn symlinked_events_or_checkpoint_is_rejected();

#[tokio::test]
async fn no_analyzable_frames_fails_before_agent_construction();

#[tokio::test]
async fn callback_receives_zero_then_each_saved_response_snapshot();

#[tokio::test]
async fn completed_checkpoint_returns_without_agent_construction();

#[tokio::test]
async fn failed_save_never_emits_an_unsaved_response();
```

Use a private generic helper with a closure that increments when an Agent is constructed.

- [ ] **Step 2: Run facade tests and verify red**

Run:

```bash
cargo test -p backend analysis::facade -- --nocapture
```

Expected: compilation fails because `analysis::facade` and `AnalyzeSession` do not exist.

- [ ] **Step 3: Implement the facade in exact order**

Implement:

```rust
async fn analyze_session_with<M, F>(
    request: AnalyzeSession,
    make_agent: F,
    mut on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint, Error>
where
    M: CompletionModel,
    F: FnOnce() -> Result<Agent<M>, Error>,
```

Order operations exactly:

```text
trim/reject checklist
validate regular no-follow session directory and zero-byte marker
validate direct regular no-follow events.jsonl and load it
spawn_blocking list_segments for every session camera
build warnings/frame plan and reject an empty plan
reject a symlinked/non-regular analysis.json when it exists, then load or save it
emit initial complete checkpoint snapshot
return if responses == total_batches
construct Agent
run remaining five-frame-set batches
save then emit each snapshot
return final checkpoint
```

Production calls `OpenAiAgent::from_env` only at the lazy construction step.

- [ ] **Step 4: Remove all Synology code and dependencies**

Delete the client, login/list/download schemas, HTTP tests, old paid Analyzer test, simulator crate, fixtures, and recipes. Remove `synology` from workspace members. Remove direct backend Reqwest, `futures-util`, and Axum dev dependencies and unneeded Tokio `fs`/`io-util` features. Keep root Axum/Tower because camera uses them. Transitive Reqwest in `Cargo.lock` is allowed through Rig/Dioxus.

- [ ] **Step 5: Verify the three-crate workspace**

Run:

```bash
cargo fmt --all --check
cargo test -p backend
cargo test --workspace --all-targets
cargo metadata --no-deps --format-version 1
```

Expected: workspace packages are only `app`, `backend`, and `camera`; no test starts an HTTP recording server.

Run current-source removal check:

```bash
rg -n 'Synology|synology|LEO_SYNOLOGY_URL|SynologyClient|download_batch|DownloadedVideo' \
  backend/src app/src Cargo.toml app/Cargo.toml backend/Cargo.toml justfile
```

Expected: no matches.

Suggested commit if separately authorized: `refactor(backend): replace synology with local analysis`

---

### Task 7: Implement One FFmpeg Recording Attempt

**Files:**
- Create: `backend/src/recording/recorder.rs`
- Modify: `backend/src/recording/mod.rs`
- Modify: `backend/src/recording/error.rs`
- Modify: `backend/Cargo.toml`

**Interfaces:**
- Consumes: shared `probe_media` from Task 3.
- Produces: exact command construction, responsive progress parsing, graceful/forced cleanup, and no-clobber segment finalization.

- [ ] **Step 1: Write exact command and secrecy tests**

Use a fake executable that writes each argument to `$FAKE_FFMPEG_ARGS`, creates its last argument, emits one prefixed progress line to stderr, and waits for `q`. Add:

```rust
#[test]
fn ffmpeg_command_uses_tcp_timeout_video_copy_and_matroska();

#[test]
fn timeout_microseconds_are_checked_before_command_creation();

#[test]
fn recorder_errors_and_events_never_expose_rtsp_credentials();
```

Assert the argument order after `ffmpeg-sidecar`'s automatic `-loglevel level+info` is:

```text
-hide_banner -n
-rtsp_transport tcp
-timeout <checked-microseconds>
-i <url>
-map 0:v:0 -an -c:v copy
-avoid_negative_ts make_zero
-f matroska
<partial-path>
```

Do not add `-nostats` or a duplicate log-level flag.

- [ ] **Step 2: Write progress and cleanup tests**

Add:

```rust
#[test]
fn first_qualifying_progress_freezes_media_timeline_zero();

#[test]
fn later_progress_does_not_move_media_timeline_zero();

#[test]
fn readiness_requires_a_frame_and_nonempty_regular_output();

#[test]
fn graceful_stop_sends_q_and_reaps();

#[test]
fn stop_timeout_kills_and_reaps();
```

The fake scripts must `exec` long-running children so inherited stderr cannot keep the parser pump alive.

- [ ] **Step 3: Run attempt tests and verify red**

Run:

```bash
cargo test -p backend recording::recorder -- --nocapture
```

Expected: compilation fails because `recording::recorder` and the attempt runner do not exist.

- [ ] **Step 4: Implement a responsive parser pump**

Do not use blocking `FfmpegChild::iter()`. Take stderr and run `FfmpegLogParser::parse_next_event()` on a short-lived pump thread. Forward only:

```rust
enum PumpEvent {
    Progress(FfmpegProgress),
    Failed,
    Eof,
}
```

Never forward raw FFmpeg lines. Take/drop stdout because no media is written there. The supervisor receives pump events with `recv_timeout`, so stop/shutdown tokens remain responsive.

Parse `progress.time` with `FfmpegTimeDuration::from_str`; reject negative or invalid values. On the first qualifying progress only:

```rust
let media_zero_utc_ms = observed_utc_ms
    .checked_sub(progress_time.as_micros().div_euclid(1_000))
    .ok_or(Error::TimestampOverflow)?;
```

- [ ] **Step 5: Implement stop, kill, and reap without early returns**

Use `child.as_inner_mut().try_wait()` at 50 ms intervals. Cleanup order is:

```text
poll already-exited child
send q
poll until stop timeout
kill on timeout or failed q
wait after kill
join parser pump
return the first error after every cleanup step ran
```

Never use `?` before the child is reaped and pump joined.

- [ ] **Step 6: Write finalization tests**

Use fake FFprobe scripts from Task 3 and add:

```rust
#[test]
fn positive_container_start_adjusts_segment_bounds();

#[test]
fn reconnect_start_is_clamped_to_previous_end();

#[test]
fn valid_attempt_is_promoted_without_overwrite();

#[test]
fn empty_attempt_is_removed();

#[test]
fn invalid_nonempty_attempt_is_retained();
```

- [ ] **Step 7: Run finalization tests and verify red**

Run:

```bash
cargo test -p backend recording::recorder -- --nocapture
```

Expected: the new finalization assertions fail because attempts are not yet probed, clamped, or promoted.

- [ ] **Step 8: Implement no-clobber finalization**

After child reaping and pump drain, remove an empty attempt. Keep nonempty media with no frozen timeline or invalid semantic probe under its partial name. For valid media:

```rust
let candidate_start = media_zero_utc_ms
    .checked_add(probe.start_time_ms)
    .ok_or(Error::TimestampOverflow)?;
let start_utc_ms = candidate_start.max(previous_end_utc_ms.unwrap_or(i64::MIN));
let end_utc_ms = start_utc_ms
    .checked_add(probe.media_span_ms)
    .ok_or(Error::TimestampOverflow)?;
```

Promote without overwrite using existing `tempfile` support, not another dependency:

```rust
let mut temporary = tempfile::TempPath::try_from_path(partial_path)?;
temporary.disable_cleanup(true);
temporary.persist_noclobber(&final_path)?;
```

On persist error, retain the partial path for diagnosis and return a fatal error.

- [ ] **Step 9: Verify one-attempt behavior**

Run:

```bash
cargo fmt --all --check
cargo test -p backend recording::recorder -- --nocapture
```

Expected: command, progress, secrecy, graceful/forced cleanup, probe, clamping, and no-overwrite tests pass.

Suggested commit if separately authorized: `feat(backend): finalize host recording attempts`

---

### Task 8: Implement Recorder Runtime Supervision

**Files:**
- Modify: `backend/src/recording/recorder.rs`
- Modify: `backend/src/recording/error.rs`
- Modify: `backend/src/recording/mod.rs`

**Interfaces:**
- Produces: the complete public recorder API.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingCamera {
    pub id: u32,
    pub rtsp_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecorderSettings {
    pub io_timeout: Duration,
    pub retry_delay: Duration,
    pub stop_timeout: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderStatus { Starting, Recording, Reconnecting, Stopped }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderEvent {
    Status {
        camera_id: u32,
        status: RecorderStatus,
        message: Option<String>,
    },
    Faulted {
        camera_id: Option<u32>,
        message: String,
    },
}

#[derive(Clone)]
pub struct RecorderHandle { /* private command sender and shutdown token */ }

pub struct RecorderRuntime { /* private runtime thread */ }

impl RecorderRuntime {
    pub fn spawn(
        settings: RecorderSettings,
    ) -> Result<(
        Self,
        RecorderHandle,
        tokio::sync::mpsc::UnboundedReceiver<RecorderEvent>,
    ), Error>;

    pub fn shutdown(self) -> Result<(), Error>;
}

impl RecorderHandle {
    pub async fn start(
        &self,
        cameras: Vec<RecordingCamera>,
        recordings_root: PathBuf,
    ) -> Result<(), Error>;

    pub async fn stop(&self) -> Result<Vec<RecordingSegment>, Error>;
}
```

- [ ] **Step 1: Write preflight and backend-validation tests**

Keep the alternate-executable constructor private in normal builds. Expose one concrete wrapper only under the non-default `test-support` feature; this avoids a recorder trait while allowing app tests to construct a handle without host FFmpeg:

```rust
fn spawn_with_executables(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<(
    RecorderRuntime,
    RecorderHandle,
    UnboundedReceiver<RecorderEvent>,
), Error>;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn spawn_for_test(
    settings: RecorderSettings,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
) -> Result<(
    RecorderRuntime,
    RecorderHandle,
    UnboundedReceiver<RecorderEvent>,
), Error> {
    spawn_with_executables(settings, ffmpeg, ffprobe)
}
```

Add:

```rust
#[test]
fn spawn_rejects_missing_or_failing_ffmpeg_and_ffprobe();

#[test]
fn hanging_preflight_is_killed_reaped_and_times_out();

#[test]
fn spawn_rejects_zero_or_unrepresentable_settings();

#[tokio::test]
async fn start_rejects_empty_duplicate_zero_and_non_rtsp_cameras();

#[tokio::test]
async fn start_rejects_missing_symlinked_and_non_directory_output_paths();
```

Preflight spawns each executable with `-version` and null stdout/stderr, polls it for at most `settings.io_timeout`, and requires successful exit. After spawn, never return early: timeout, `try_wait`, or status errors all attempt kill when the child may still run and always call `wait`; return the first error only after reap. Do not expose executable paths in normal builds.

- [ ] **Step 2: Run validation tests and verify red**

Run:

```bash
cargo test -p backend recording::recorder -- --nocapture
```

Expected: the new tests fail because runtime spawning, bounded preflight, URL parsing, and output validation do not exist.

- [ ] **Step 3: Implement bounded preflight, validation, and command topology**

Use standard channels internally and Tokio oneshots at the async API boundary:

```rust
enum RecorderCommand {
    Start {
        cameras: Vec<RecordingCamera>,
        recordings_root: PathBuf,
        reply: oneshot::Sender<Result<(), Error>>,
    },
    Stop {
        reply: oneshot::Sender<Result<Vec<RecordingSegment>, Error>>,
    },
    Shutdown,
}
```

One management thread owns an optional active `RecorderSet`; one supervisor thread owns each current FFmpeg child and reconnect state. Create shared runtime/session shutdown atomics before blocking work. `RecorderHandle::start/stop` send commands and await one-shot replies.

Use `url::Url::parse` and require exact `rtsp` scheme. Before sending Start to supervisors, require `recordings_root` itself and every expected direct `camera-<id>` child to be existing regular no-follow directories; reject missing paths, symlinks, files, extra path traversal, zero IDs, duplicate IDs, and an empty camera list.

- [ ] **Step 4: Write all-camera startup tests**

Use fake FFmpeg scripts parameterized per camera and add:

```rust
#[tokio::test]
async fn start_waits_for_every_camera();

#[tokio::test]
async fn one_startup_failure_stops_and_reaps_ready_cameras();

#[tokio::test]
async fn duplicate_start_and_stop_commands_are_rejected();
```

Use one shared initial deadline computed before spawning supervisors. Before first readiness, child exit/timeout is startup failure, not unbounded reconnect. Initial failures are returned only through the Start reply after every started child is cleaned up; do not emit `Faulted` before Start succeeds, because Workflow must roll back to Idle.

- [ ] **Step 5: Run startup tests and verify red**

Run:

```bash
cargo test -p backend recording::recorder -- --nocapture
```

Expected: all-camera startup and rollback tests fail because the management thread does not yet gate readiness or coordinate cleanup.

- [ ] **Step 6: Implement all-camera startup and post-readiness reconnect loops**

After a camera's first qualifying progress:

```text
emit Recording
on ordinary exit/RTSP timeout: finalize valid attempt
emit Reconnecting with sanitized status
wait retry_delay interruptibly
write, sync, and remove one-byte storage probe
create a new UUID partial path
spawn another FFmpeg attempt
emit Recording after qualifying progress
repeat until Stop
```

After the all-camera Start reply succeeds, spawn, parser, storage, FFprobe process/JSON, promotion, quit, kill, wait, or reap failures emit exactly one `Faulted` event and return a fatal result. Before that boundary, the same errors fail Start without an event. Pass `settings.io_timeout` and the runtime shutdown flag to every FFprobe call so Stop finalization is bounded and runtime shutdown can interrupt, kill, reap, and join a hanging probe.

Emit sanitized tracing events for attempt spawn, readiness, ordinary exit, reconnect, finalization, Stop, forced kill, and fatal supervision. Fields may include camera ID, attempt UUID, numeric timing, and output path, but never RTSP URL or raw FFmpeg lines.

- [ ] **Step 7: Write reconnect and fatal tests**

Add:

```rust
#[tokio::test]
async fn ordinary_exit_finalizes_and_emits_reconnecting();

#[tokio::test]
async fn retry_uses_a_new_partial_path_and_returns_to_recording();

#[tokio::test]
async fn storage_probe_failure_emits_faulted();

#[tokio::test]
async fn stop_finalizes_and_reaps_every_camera_concurrently();

#[tokio::test]
async fn hanging_stop_probe_times_out_without_leaking_a_process();
```

Signal every supervisor before joining the first so two five-second stop deadlines do not serialize to ten seconds.

- [ ] **Step 8: Run reconnect tests and verify red**

Run:

```bash
cargo test -p backend recording::recorder -- --nocapture
```

Expected: reconnect, concurrent finalization, and hanging-probe assertions fail before supervision is completed.

- [ ] **Step 9: Implement runtime shutdown ownership**

`RecorderRuntime::shutdown` and Drop fallback must:

```text
set runtime shutdown atomic
send Shutdown to wake management thread
management sets active-session shutdown and signals every supervisor
supervisors stop/kill/reap current children
management joins every supervisor
caller joins management thread
return panic/cleanup errors after all cleanup attempts
```

Cloned handles may keep channels alive, so channel closure is not shutdown.

- [ ] **Step 10: Test shutdown during every blocking phase**

Add:

```rust
#[test]
fn shutdown_interrupts_initial_readiness_and_reaps_children();

#[test]
fn shutdown_interrupts_reconnect_delay();

#[test]
fn shutdown_during_stop_finalization_still_joins_runtime();

#[test]
fn shutdown_interrupts_ffprobe_and_reaps_it();
```

- [ ] **Step 11: Verify supervision**

Run:

```bash
cargo fmt --all --check
cargo test -p backend recording::recorder -- --nocapture
cargo test -p backend recording::
```

Expected: no fake child or pump thread remains after any success/failure/shutdown test.

In `recording/mod.rs`, keep child modules private and publish the documented boundary only:

```rust
pub use error::Error;
pub use recorder::{
    RecorderEvent, RecorderHandle, RecorderRuntime, RecorderSettings, RecorderStatus,
    RecordingCamera,
};
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use recorder::spawn_for_test;
pub use segment::{RecordingSegment, list_segments};
```

Suggested commit if separately authorized: `feat(backend): supervise host recorders`

---

### Task 9: Add App Configuration, Logging, And Runtime Ownership

**Files:**
- Create: `cameras.json`
- Create: `app/src/lib.rs`
- Create: `app/src/camera_config.rs`
- Create: `app/src/logging.rs`
- Modify: `app/src/main.rs`
- Modify: `app/src/preview/bridge.rs`
- Modify: `app/src/preview/config.rs`
- Modify: `app/src/preview/mod.rs`
- Modify: `app/Cargo.toml`
- Modify: `.gitignore`

**Interfaces:**
- Produces: validated startup configuration, stable preview identity, structured logs, and process owners retained outside VDOM state.

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CameraConfig {
    pub id: u32,
    pub name: String,
    pub rtsp_url: String,
    pub enabled: bool,
    pub sample_every_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartupConfig {
    pub cameras: Vec<CameraConfig>,
    pub data_root: PathBuf,
    pub sessions_root: PathBuf,
    pub logs_root: PathBuf,
    pub recorder_settings: RecorderSettings,
}

pub fn load_startup_config() -> Result<StartupConfig, Error>;
```

- [ ] **Step 1: Write startup-config validation tests**

Keep `load_startup_config` as a thin environment wrapper over a private loader accepting explicit camera path, data path, and timeout text. Test that loader for exact two-row parsing, one/three rows, unknown fields, zero/duplicate IDs, blank names/URLs, non-RTSP schemes, zero/non-whole-second cadence, missing/malformed files, invalid data root, zero/overflow timeout, and default/override values. Do not mutate process-global environment in unit tests.

- [ ] **Step 2: Run configuration tests and verify red**

Run:

```bash
cargo test -p app camera_config -- --nocapture
```

Expected: compilation fails because `camera_config` and `StartupConfig` do not exist.

- [ ] **Step 3: Add the checked-in two-camera config**

Create `cameras.json` exactly:

```json
[
  {
    "id": 1,
    "name": "Salon 1",
    "rtspUrl": "rtsp://127.0.0.1:8554/axis-media/media.amp",
    "enabled": true,
    "sampleEveryMs": 1000
  },
  {
    "id": 2,
    "name": "Salon 2",
    "rtspUrl": "rtsp://127.0.0.1:8555/axis-media/media.amp",
    "enabled": true,
    "sampleEveryMs": 1000
  }
]
```

- [ ] **Step 4: Implement startup loading**

Add `url = { workspace = true }` to app dependencies. Load `LEO_CAMERA_CONFIG`/`./cameras.json`, `LEO_DATA_DIR`/`./data`, and `LEO_RECORDER_TIMEOUT_SECS`/`10`. Parse URLs with `url::Url` and require exact `rtsp` scheme. Create and validate regular no-follow `<data>/sessions` and `<data>/logs`. Build settings with fixed one-second retry and five-second stop. Check timeout conversion to `i64` microseconds.

- [ ] **Step 5: Carry stable IDs through preview**

Change:

```rust
pub struct CameraSource {
    pub id: u32,
    pub name: String,
    pub rtsp_url: String,
}

pub struct PreviewFeed {
    pub camera_id: u32,
    pub name: String,
    pub video_id: String,
    pub whep_url: String,
}
```

Keep MediaMTX paths based on vector index. Add a regression proving camera IDs 26/41 still use `camera-0`/`camera-1` while metadata preserves 26/41.

- [ ] **Step 6: Add structured logging**

Add workspace tracing plus app `tracing-subscriber` (`env-filter`, `json`) and `tracing-appender`. Implement:

```rust
pub struct LogGuard {
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init(directory: &Path) -> Result<LogGuard, Error>;
```

Use `RUST_LOG` or `info`, compact console output, and daily JSON `leo.jsonl.<date>`. A temp-dir test emits one structured event, drops the guard, and asserts JSON level/message/field output.

Replace preview startup/shutdown `eprintln!` calls with sanitized tracing and emit configuration/data-root and preview ready/unavailable/stop events. Log camera IDs/counts and local paths, never RTSP URLs. Recorder, session, catalogue, gap, and analysis events remain in their owning backend/app tasks rather than one logging facade.

- [ ] **Step 7: Create the app library and thin executable**

Move routes/startup/root component from main to `lib.rs`. Make main exactly:

```rust
fn main() {
    app::launch();
}
```

Use Dioxus 0.7.9 without its duplicate logger:

```toml
dioxus = {
    version = "0.7.9",
    default-features = false,
    features = ["desktop", "lib", "launch", "router"]
}
```

Remove unused `dioxus-icons` and Git `dioxus-primitives`.

- [ ] **Step 8: Retain recorder, preview, and log owners until shutdown**

After config/log init, call `RecorderRuntime::spawn` and preview bridge startup. Define cloneable bootstrap context:

```rust
#[derive(Clone)]
struct RecorderBootstrap {
    handle: RecorderHandle,
    events: Arc<Mutex<Option<UnboundedReceiver<RecorderEvent>>>>,
}

#[derive(Clone)]
enum Bootstrap {
    Ready {
        config: StartupConfig,
        preview: PreviewState,
        recorder: RecorderBootstrap,
    },
    Unavailable { message: String },
}
```

The Tao custom event handler owns `Option<RecorderRuntime>`, `Option<Bridge>`, and `Option<LogGuard>`. On `LoopDestroyed`, call recorder shutdown first, then bridge stop, then drop log guard. Recorder Drop is fallback. Invalid config/log/preflight launches Unavailable with no Start; preview failure remains Ready with unavailable preview state.

- [ ] **Step 9: Ignore runtime/brainstorm artifacts and verify**

Add:

```gitignore
/data/
/.superpowers/
```

Run:

```bash
cargo fmt --all --check
cargo test -p app camera_config -- --nocapture
cargo test -p app logging -- --nocapture
cargo test -p app preview -- --nocapture
```

Expected: startup, logging, and stable preview identity pass without launching real recorder media.

Suggested commit if separately authorized: `feat(app): configure host recording runtime`

---

### Task 10: Add Workflow State And Root Session Tasks

**Files:**
- Create: `app/src/workflow/mod.rs`
- Create: `app/src/workflow/workflow.rs`
- Create: `app/src/workflow/error.rs`
- Create: `app/src/session_task.rs`
- Modify: `app/src/lib.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**
- Produces: one session state machine and route-independent recorder start/stop/fault orchestration.

```rust
pub struct CameraState {
    pub config: CameraConfig,
    pub participating: bool,
    pub recorder_status: RecorderStatus,
}

pub enum SessionRunState {
    Idle,
    Starting { directory: PathBuf },
    Active {
        directory: PathBuf,
        controller: SessionController,
    },
    Stopping { directory: PathBuf },
    Faulted { directory: PathBuf, message: String },
}

pub struct SessionListItem {
    pub stored: StoredSession,
    pub checkpoint: std::result::Result<Option<AnalysisCheckpoint>, String>,
}

pub struct Workflow {
    pub cameras: Vec<CameraState>,
    pub selected_camera_id: Option<u32>,
    pub session: SessionRunState,
    pub sessions: Vec<SessionListItem>,
    pub selected_session_id: Option<Uuid>,
    pub running_analysis_id: Option<Uuid>,
    pub analysis_error: Option<(Uuid, String)>,
    pub model_config_error: Option<String>,
    pub message: Option<String>,
    pub session_root: PathBuf,
    recorder: RecorderHandle,
}

impl Workflow {
    pub fn new(
        cameras: Vec<CameraConfig>,
        session_root: PathBuf,
        recorder: RecorderHandle,
        model_config_error: Option<String>,
    ) -> Result<Self, Error>;
}
```

- [ ] **Step 1: Write pure transition tests**

Create the private `workflow` module, declare it from `app/src/lib.rs`, and add in-module named tests for construction, selection, Idle/Starting/Active/Stopping/Faulted transitions, duplicate Start/Stop rejection, Start rejection while analysis runs, write-before-state participation/cadence, marker-gated refresh, invalid checkpoint row errors, and older-session preservation. Add `backend = { path = "../backend", features = ["test-support"] }` to app dev-dependencies. Tests call `backend::recording::spawn_for_test` with one tiny successful fake executable used for both preflight paths, retain the returned runtime, and shut it down before returning.

- [ ] **Step 2: Run transition tests and verify red**

Run:

```bash
cargo test -p app workflow -- --nocapture
```

Expected: compilation fails because Workflow transitions and request types are not implemented.

- [ ] **Step 3: Implement state transition requests**

Keep filesystem/process awaits in `session_task.rs`. Workflow methods synchronously own transitions:

```rust
pub fn begin_start(&mut self, utc_ms: i64) -> Result<StartSessionRequest, Error>;
pub fn finish_start(&mut self, directory: PathBuf, controller: SessionController);
pub fn fail_start(&mut self, message: String);
pub fn begin_stop(&mut self) -> Result<StopSessionRequest, Error>;
pub fn finish_stop(&mut self) -> Result<(), Error>;
pub fn begin_fault(
    &mut self,
    message: String,
    append_end: bool,
) -> Option<FaultSessionRequest>;
pub fn finish_fault(&mut self, directory: PathBuf, message: String);
pub fn apply_recorder_event(&mut self, event: &RecorderEvent);
pub fn refresh_sessions(&mut self) -> Result<(), Error>;
```

Define the request structs exactly inside the private workflow module; they are not re-exported from app root:

```rust
pub struct StartSessionRequest {
    pub directory: PathBuf,
    pub events_path: PathBuf,
    pub recording_cameras: Vec<RecordingCamera>,
    pub session_cameras: Vec<SessionCamera>,
    pub recorder: RecorderHandle,
}

pub struct StopSessionRequest {
    pub directory: PathBuf,
    pub controller: SessionController,
    pub recorder: RecorderHandle,
}

pub struct FaultSessionRequest {
    pub directory: PathBuf,
    pub controller: Option<SessionController>,
    pub recorder: RecorderHandle,
    pub message: String,
}
```

`begin_fault(..., true)` retains the controller in the request so the task attempts `EndSession`; `begin_fault(..., false)` drops the uncertain controller and only requests recorder cleanup. Both move state to Faulted immediately so duplicate fatal events cannot launch duplicate cleanup.

Emit sanitized tracing for session start requests/results, durable actions, Stop/finalization, and fault cleanup. Include session UUID/directory and camera IDs where available, but never checklist text or RTSP URLs.

Expose these crate-internal task entry points from private `session_task`:

```rust
pub fn spawn_start_session(mut workflow: Signal<Workflow>, utc_ms: i64);
pub fn spawn_stop_session(mut workflow: Signal<Workflow>);
pub fn spawn_fault_cleanup(
    mut workflow: Signal<Workflow>,
    request: FaultSessionRequest,
);
```

- [ ] **Step 4: Implement root-scoped Start**

`spawn_start_session(workflow, utc_ms)` first calls `begin_start`, which uses `create_dir_all(session_root)`, then `create_dir(<utc-ms>)`, `create_dir(recordings)`, and direct `camera-<id>` directories. It sets Starting and returns `RecordingCamera` plus `SessionCamera` snapshots.

The forever task awaits `recorder.start`. Only after all cameras are ready, call `SessionController::create(events.jsonl, session_cameras)` and set Active. If recorder start or controller creation fails, call recorder stop when needed; remove the staging directory only after successful cleanup; return Idle with a live-region error.

- [ ] **Step 5: Implement root-scoped Stop**

`begin_stop` moves the Active controller out and sets Stopping. The task calls `controller.apply(EndSession)` synchronously, always awaits `recorder.stop`, and creates `recording-complete` only when both succeeded. On complete success set Idle and refresh sessions. On any uncertain event write or fatal cleanup failure, preserve the directory without marker and set Faulted.

- [ ] **Step 6: Keep metadata writes write-before-state**

For participation and cadence, call `SessionController::apply` before changing camera state. On error, leave displayed values unchanged, call `begin_fault(message, false)`, and spawn recorder cleanup. Do not attempt another EndSession after an uncertain append.

- [ ] **Step 7: Consume recorder events once at root**

Take the `UnboundedReceiver` once from `RecorderBootstrap` in the root component and launch one forever task. `Status` updates only that camera. `Reconnecting` never faults. `Faulted` calls `begin_fault(message, true)` exactly once and awaits recorder stop; duplicate fatal events while Stopping/Faulted are ignored.

Never hold `workflow.write()` across an await.

- [ ] **Step 8: Add durable integration coverage**

Keep recorder calls at the edge of `session_task.rs`. Add private generic `run_start_session_with` and `run_stop_session_with` helpers that accept the already-created request plus lazy Start/Stop futures; production passes `RecorderHandle::start`/`stop`, while tests pass ready futures and one atomic "stop polled" flag. This is a test seam for task ordering, not a recorder trait or alternate implementation.

In `session_task.rs`'s test module, use those helpers, two configs, and a temp data root to Start, change camera 2 cadence/participation, Stop, reload events, assert marker ordering, create a second Workflow, and rediscover the UUID. Add failure cases for all-camera startup rollback and EndSession failure still polling cleanup, and for EndSession failure still polling Stop. Keeping this in-crate avoids publishing Dioxus workflow internals solely for tests or requiring FFmpeg in normal app unit tests.

- [ ] **Step 9: Verify workflow and root tasks**

Run:

```bash
cargo fmt --all --check
cargo test -p app workflow -- --nocapture
cargo test -p app session_task -- --nocapture
```

Expected: lifecycle, durable ordering, reconnect status, and fatal cleanup behavior pass without real FFmpeg.

Suggested commit if separately authorized: `feat(app): orchestrate recording sessions`

---

### Task 11: Add Root-Scoped Background Analysis

**Files:**
- Create: `app/src/analysis_task.rs`
- Modify: `app/src/workflow/workflow.rs`
- Modify: `app/src/workflow/error.rs`
- Modify: `app/src/lib.rs`

**Interfaces:**
- Produces: one explicit model job that survives route navigation and projects persisted checkpoints.

- [ ] **Step 1: Write failing analysis-state tests**

Build marker-complete stored sessions and v2 snapshots. Add:

```rust
#[test]
fn empty_checklist_missing_model_active_session_and_second_job_are_rejected();

#[test]
fn existing_checkpoint_locks_its_checklist();

#[test]
fn checkpoint_snapshots_replace_instead_of_append();

#[test]
fn all_observations_and_latest_summary_checklist_are_projected();

#[test]
fn final_snapshot_and_failure_clear_the_matching_running_id();

#[test]
fn retry_preserves_the_saved_checkpoint();
```

- [ ] **Step 2: Run analysis-state tests and verify red**

Run:

```bash
cargo test -p app analysis_task -- --nocapture
```

Expected: compilation fails because model state and analysis transitions do not exist.

- [ ] **Step 3: Add model-configuration presentation state**

Set `Workflow.model_config_error: Option<String>` at startup to a fixed sanitized message when `OPENAI_API_KEY` or `ANALYSIS_MODEL` is absent. `OPENAI_BASE_URL` remains optional and Rig validates it during lazy provider construction. Never store, log, or render secret values.

- [ ] **Step 4: Implement analysis Workflow methods**

```rust
pub fn begin_analysis(&mut self, checklist: String) -> Result<AnalyzeSession, Error>;
pub fn apply_checkpoint(&mut self, checkpoint: AnalysisCheckpoint);
pub fn analysis_failed(&mut self, session_id: Uuid, message: String);
```

Require selected completed session, Idle recorder state, no running job, valid checkpoint, available model config, and non-empty new/persisted checklist. Set running ID only after validation. Replace complete row snapshots. Clear running ID on a final `responses.len() == total_batches` snapshot or matching failure.

- [ ] **Step 5: Implement the root forever task**

```rust
pub fn spawn_analysis(
    mut workflow: Signal<Workflow>,
    request: AnalyzeSession,
    session_id: Uuid,
) {
    dioxus::dioxus_core::spawn_forever(async move {
        let mut checkpoint_workflow = workflow;
        let result = backend::analysis::analyze_session(request, move |checkpoint| {
            checkpoint_workflow.write().apply_checkpoint(checkpoint);
        })
        .await;

        if let Err(error) = result {
            workflow
                .write()
                .analysis_failed(session_id, error.to_string());
        }
    });
}
```

Signal write guards exist only inside short callback/error blocks.

- [ ] **Step 6: Add navigation-independent integration coverage**

In `analysis_task.rs`'s test module, feed zero/one/two-response checkpoints with one RecordingGap through the same Workflow callback. Recreate route presentation state between updates and assert progress continued, warning persisted, all observations remain ordered, latest summary/checklist win, and retry uses persisted text.

- [ ] **Step 7: Verify analysis state**

Run:

```bash
cargo fmt --all --check
cargo test -p app analysis_task -- --nocapture
```

Expected: no test constructs a real model or sends a request.

The root task emits sanitized analysis start/failure/completion events keyed by session UUID and batch counts; backend analysis owns per-batch/save events.

Suggested commit if separately authorized: `feat(app): run analysis outside route lifetimes`

---

### Task 12: Implement Monitor Recording Controls

**Files:**
- Modify: `app/src/components/camera/feed.rs`
- Modify: `app/src/views/monitor/monitor.rs`
- Modify: `app/src/views/monitor/sidebar.rs`
- Modify: `app/src/views/navbar.rs`
- Modify: `app/src/views/sidebar.rs`
- Modify: `app/src/views/mod.rs`
- Create: `app/src/views/render_tests.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**
- Produces: exactly two previews, camera selection, analysis participation/cadence, recorder health, and Start/Stop lifecycle presentation.

- [ ] **Step 1: Add SSR render test infrastructure**

Add `dioxus-ssr = "0.7.9"` as a dev dependency and declare `#[cfg(test)] mod render_tests;` in `views/mod.rs`. A test root takes prepared Workflow into `use_signal`, provides it plus PreviewState, and renders Monitor sidebar and body together. Build with `VirtualDom`/`NoOpMutations`, then call `dioxus_ssr::render`.

- [ ] **Step 2: Write failing semantic-state renders**

Render Idle, Starting, Active Recording, Active Reconnecting, Stopping, Faulted, preview unavailable, and invalid startup. Assert:

```text
both stable video IDs remain mounted while excluded/reconnecting
one reader.js script exists
Included/Excluded is separate from recorder status
interval input is type=number min=1
Start/Stop disabled states match lifecycle
status/error live regions exist
LIVE, 14:42:18, CAM 04, Camera options are absent
```

- [ ] **Step 3: Run Monitor renders and verify red**

Run:

```bash
cargo test -p app views::render_tests -- --nocapture
```

Expected: semantic lifecycle assertions fail against the current static Monitor UI.

- [ ] **Step 4: Narrow CameraFeed props and remove fake claims**

```rust
#[component]
pub fn CameraFeed(
    feed: PreviewFeed,
    selected: bool,
    participating: bool,
    recorder_status: RecorderStatus,
    on_select: EventHandler<u32>,
) -> Element;
```

Render one native `script { src: script_url, defer: true }` in Monitor, not `document::Script` per card. Key each feed by camera ID. Add semantic Select and labelled badges; keep both videos mounted.

- [ ] **Step 5: Implement all sidebar states**

Idle shows Start/root/selected camera. Starting shows per-camera readiness and no Cancel. Active shows elapsed time, recording/reconnecting statuses, selected camera, Include/Exclude, integer cadence input/Apply, Stop, and directory. Stopping disables controls and shows finalization. Faulted shows blocking guidance and directory.

Key selected-camera controls by camera ID so draft cadence does not leak. Route-local ticker rerenders once per second but displays `SessionController::elapsed`; route unmount never affects recording.

- [ ] **Step 6: Route every action through root tasks**

Start and Stop call `spawn_start_session`/`spawn_stop_session`; write errors trigger `spawn_fault_cleanup`. Selection is synchronous. Store any action error in shared `Workflow.message` and clear it on success. Render the shared message once as `role="alert"` above routed content.

- [ ] **Step 7: Keep layout sparse and responsive**

Use semantic buttons/labels/inputs and existing `btn`, `input`, `badge`, `alert`, grid/flex utilities. Remove inert Settings. Avoid gradients, animation, decorative shadows, or a new component system.

- [ ] **Step 8: Verify Monitor**

Run:

```bash
cargo fmt --all --check
cargo test -p app views::monitor -- --nocapture
cargo test -p app components::camera -- --nocapture
cargo test -p app views::render_tests -- --nocapture
```

Expected: every lifecycle and reconnect state renders semantically and both previews remain mounted.

Suggested commit if separately authorized: `feat(app): add recording monitor controls`

---

### Task 13: Implement Analyze Session Results

**Files:**
- Modify: `app/src/views/analyze/sidebar.rs`
- Modify: `app/src/views/analyze/analyze.rs`
- Modify: `app/src/views/render_tests.rs`
- Modify: `app/src/views/navbar.rs`
- Modify: `justfile`
- Regenerate: `app/assets/tailwind.css`

**Interfaces:**
- Produces: completed-session selection, explicit analysis, persisted progress, gap warnings, and model results.

- [ ] **Step 1: Write failing Analyze SSR cases**

Render no sessions, two sessions, invalid checkpoint, missing model config, zero-response checkpoint, partial, complete, complete-with-warning, running, and failed states. Assert exact row status, recap paths, read-only checklist, disabled actions, progress values, RecordingGap text, ordered observations, and latest summary/checklist.

- [ ] **Step 2: Run Analyze renders and verify red**

Run:

```bash
cargo test -p app views::render_tests -- --nocapture
```

Expected: Analyze cases fail against the placeholder route.

- [ ] **Step 3: Implement session list and refresh**

Newest-first buttons show UTC start and one derived state: `Not started`, `Running`, `In progress`, `Complete`, `Complete with warning`, `Failed`, or `Invalid checkpoint`. Refresh/selection never start analysis and route errors into shared message.

- [ ] **Step 4: Implement selected-session recap**

Display UUID, UTC start, duration, camera count, directory, `events.jsonl`, `recordings/`, `recording-complete`, and `analysis.json`. Show checkpoint errors without replacing the file. Add no preview/playback/raw JSON.

- [ ] **Step 5: Implement checklist and explicit Analyze/Resume**

Key the selected body by session UUID. Use local textarea state only when no checkpoint exists. Existing checkpoint text is prefilled/read-only. Disable action while recording, running, complete, invalid, or model config is missing. On click call `begin_analysis`, then `spawn_analysis`; do not use `?` in the Dioxus listener.

- [ ] **Step 6: Render warnings, progress, and results**

Render `<progress value=responses.len() max=total_batches>`. Show every `RecordingGap { camera_id, start_offset_ms, end_offset_ms }` before model content. Aggregate every response's observations in order. Use only the latest response for cumulative summary/checklist state.

- [ ] **Step 7: Generate final CSS deterministically**

Add:

```just
css:
    nix develop --command tailwindcss -i app/tailwind.css -o app/assets/tailwind.css
```

Run:

```bash
cargo fmt --all --check
cargo test -p app views::analyze -- --nocapture
cargo test -p app views::render_tests -- --nocapture
just css
```

Expected: all states render; generated CSS includes final Monitor/Analyze classes and is not hand-edited.

Suggested commit if separately authorized: `feat(app): render persisted analysis results`

---

### Task 14: Add Real Two-Camera Recording And Reconnect Coverage

**Files:**
- Create: `camera/fixtures/salon-1.mp4`
- Create: `camera/fixtures/salon-2.mp4`
- Modify: `camera/tests/rtsp_stream.rs`
- Modify: `camera/Cargo.toml`
- Modify: `backend/src/analysis/facade.rs`
- Modify: `justfile`

**Interfaces:**
- Produces: distinct local fixtures and exact ignored checks for playable MKV, reconnect segments, post-gap extraction, and one mock-model analysis batch.

- [ ] **Step 1: Generate deterministic fixtures**

Run:

```bash
just prepare-camera-video ../videos/salon-1-synced.mp4 camera/fixtures/salon-1.mp4
just prepare-camera-video ../videos/salon-2-synced.mp4 camera/fixtures/salon-2.mp4
```

Probe each and require H.264 Constrained Baseline level 3.1, 1280x720, 15 FPS, no audio, no B-frames, one-second keyframes, and at least 24 seconds.

- [ ] **Step 2: Add two-camera launch recipes**

```just
camera-1:
    nix develop --command cargo run -p camera -- --address 127.0.0.1:8080 --rtsp-address 127.0.0.1:8554 --video camera/fixtures/salon-1.mp4

camera-2:
    nix develop --command cargo run -p camera -- --address 127.0.0.1:8081 --rtsp-address 127.0.0.1:8555 --video camera/fixtures/salon-2.mp4
```

Remove obsolete `camera`/Synology recipes after replacing callers.

- [ ] **Step 3: Add playable-MKV ignored coverage**

Add backend as a camera dev dependency and extend existing `rtsp_stream.rs` to reuse ProcessGuard. Add:

```rust
#[test]
#[ignore = "requires MediaMTX, FFmpeg, and FFprobe from the Nix development shell"]
fn host_recorder_records_playable_mkv();
```

Start one virtual camera, start RecorderRuntime, wait Recording, stop, require one finalized MKV, probe H.264 packets, and assert no recorder/camera child remains.

- [ ] **Step 4: Add reconnect/post-gap ignored coverage**

```rust
#[test]
#[ignore = "requires MediaMTX, FFmpeg, and FFprobe from the Nix development shell"]
fn host_recorder_reconnects_into_a_second_segment();
```

Start camera/recorder, stop camera, wait Reconnecting, wait at least two seconds, restart on the same ports, wait Recording, then Stop. Assert two ordered non-overlapping finalized MKVs with a positive gap and successful JPEG extraction from both.

- [ ] **Step 5: Add one ignored local analysis pipeline test**

Inside facade tests, build a completed session whose two local segments lie before/after a gap, use a Rig mock response, call the private generic facade, and assert callbacks `[0, 1]`, persisted gap warning, JPEG-containing prompt, final checkpoint equality, and no session-local temporary JPEG/MP4.

- [ ] **Step 6: Add exact media recipes and run them**

```just
test-host-recording:
    nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture

test-host-reconnect:
    nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture

test-local-analysis:
    nix develop --command cargo test -p backend analysis::facade::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
```

Run all three plus the existing exact two-reader and JPEG tests. Never run blanket ignored tests.

Suggested commit if separately authorized: `test(camera): cover host recorder reconnects`

---

### Task 15: Add Paid Gate, Current Documentation, And Final Verification

**Files:**
- Create: `app/src/paid_openai_workflow.rs`
- Modify: `app/src/lib.rs`
- Modify: `app/Cargo.toml`
- Modify: `README.md`
- Modify: `app/README.md`
- Rewrite: `docs/architecture.md`
- Modify: `justfile`

**Interfaces:**
- Produces: one safely gated paid workflow test, current host-recording docs, complete automated evidence, and a manual acceptance record.

- [ ] **Step 1: Add the only paid test behind three gates**

Add app feature `paid-openai-test = []`, declare `#[cfg(all(test, feature = "paid-openai-test"))] mod paid_openai_workflow;` in `app/src/lib.rs`, and create this in-crate test so it can apply snapshots through the private real Workflow without exporting UI internals:

```rust
#![cfg(feature = "paid-openai-test")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use backend::{
    analysis::{AnalysisCheckpoint, analyze_session},
    recording::{RecorderRuntime, RecorderSettings},
    session::mark_recording_complete,
};
use serde_json::json;
use uuid::Uuid;

use crate::{camera_config::CameraConfig, workflow::Workflow};

#[tokio::test]
#[ignore = "costs money; requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"]
async fn paid_openai_analyzes_one_local_application_session() {
    assert_eq!(
        std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(),
        Ok("1"),
        "paid test requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"
    );

    let temporary = tempfile::tempdir().expect("temporary data root should be created");
    let sessions_root = temporary.path().join("sessions");
    fs::create_dir(&sessions_root).expect("sessions root should be created");
    let session_id = Uuid::new_v4();
    let start_utc_ms = 1_786_552_800_000_i64;
    let session_directory = create_local_session(
        &sessions_root,
        session_id,
        start_utc_ms,
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../camera/fixtures/salon-1.mp4")
            .as_path(),
    );

    let (runtime, handle, _events) = RecorderRuntime::spawn(RecorderSettings {
        io_timeout: Duration::from_secs(10),
        retry_delay: Duration::from_secs(1),
        stop_timeout: Duration::from_secs(5),
    })
    .expect("paid test requires ffmpeg and ffprobe");
    let mut workflow = Workflow::new(
        camera_configs(),
        sessions_root,
        handle,
        None,
    )
    .expect("workflow should initialize");
    workflow.refresh_sessions().expect("session should be discovered");
    workflow.selected_session_id = Some(session_id);
    let request = workflow
        .begin_analysis("Describe the visible exercise in order.".into())
        .expect("analysis should start");
    let mut callback_counts = Vec::new();

    let checkpoint = analyze_session(request, |snapshot| {
        callback_counts.push(snapshot.responses.len());
        workflow.apply_checkpoint(snapshot);
    })
    .await
    .expect("paid analysis should complete");

    assert_eq!(callback_counts, [0, 1]);
    assert_eq!(checkpoint.responses.len(), 1);
    assert_eq!(workflow.running_analysis_id, None);
    assert_eq!(
        AnalysisCheckpoint::read(&session_directory.join("analysis.json"), session_id)
            .expect("saved checkpoint should reload"),
        checkpoint
    );
    assert_no_temporary_media(&session_directory);
    runtime.shutdown().expect("recorder runtime should shut down");
}

fn camera_configs() -> Vec<CameraConfig> {
    [1_u32, 2]
        .into_iter()
        .map(|id| CameraConfig {
            id,
            name: format!("Salon {id}"),
            rtsp_url: format!("rtsp://127.0.0.1:855{}/axis-media/media.amp", id + 3),
            enabled: id == 1,
            sample_every_ms: 1_000,
        })
        .collect()
}

fn create_local_session(
    sessions_root: &Path,
    session_id: Uuid,
    start_utc_ms: i64,
    fixture: &Path,
) -> PathBuf {
    let directory = sessions_root.join(start_utc_ms.to_string());
    let camera_1 = directory.join("recordings/camera-1");
    let camera_2 = directory.join("recordings/camera-2");
    fs::create_dir_all(&camera_1).expect("camera 1 directory should be created");
    fs::create_dir(&camera_2).expect("camera 2 directory should be created");
    let events = [
        json!({
            "schema_version": 1,
            "sequence": 0,
            "session_id": session_id,
            "utc_ms": start_utc_ms,
            "session_offset_ms": 0,
            "action": {
                "type": "session_started",
                "cameras": [
                    {"camera_id": 1, "name": "Salon 1", "enabled": true, "sample_every_ms": 1_000},
                    {"camera_id": 2, "name": "Salon 2", "enabled": false, "sample_every_ms": 1_000}
                ]
            }
        }),
        json!({
            "schema_version": 1,
            "sequence": 1,
            "session_id": session_id,
            "utc_ms": start_utc_ms + 5_000,
            "session_offset_ms": 5_000,
            "action": {"type": "session_ended"}
        }),
    ]
    .into_iter()
    .map(|event| serde_json::to_string(&event).expect("event should serialize"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    fs::write(directory.join("events.jsonl"), events)
        .expect("event log should be written");

    let segment = camera_1.join(format!("{start_utc_ms}.mkv"));
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-ss", "0", "-t", "5", "-i"])
        .arg(fixture)
        .args(["-map", "0:v:0", "-an", "-c:v", "copy", "-f", "matroska"])
        .arg(&segment)
        .status()
        .expect("ffmpeg should start");
    assert!(status.success(), "ffmpeg should create the local MKV");
    mark_recording_complete(&directory).expect("session should be marked complete");
    directory
}

fn assert_no_temporary_media(directory: &Path) {
    for entry in fs::read_dir(directory).expect("session directory should be readable") {
        let path = entry.expect("session entry should be readable").path();
        if path.is_dir() {
            assert_no_temporary_media(&path);
        } else {
            assert!(!matches!(path.extension().and_then(|value| value.to_str()), Some("jpg" | "mp4")));
        }
    }
}
```

The runtime assertion is the first executable statement, before provider construction. Use one short local MKV. Assert callbacks response counts `[0, 1]`, persisted reload, cleared running ID, and no temporary media in the session directory.

- [ ] **Step 2: Rewrite current architecture and setup docs**

Document:

```text
three-crate workspace
session-scoped direct RTSP recording
MKV stream copy and reconnect attempts
data-root/session layout and completion marker
preview vs recorder independence
gap-tolerant direct local analysis
root runtime/task ownership
environment variables
two-camera and exact media recipes
logging and paid-test prohibition
crash/SSD/retention limitations
```

Remove all current Synology/NAS claims and recipes. Leave historical spec/plan files as history.

- [ ] **Step 3: Run all normal verification**

Run:

```bash
cargo fmt --all --check
cargo test -p backend
cargo test -p camera
cargo test -p app
cargo test --workspace --all-targets
cargo test --workspace --all-targets --all-features --no-run
cargo test --doc -p backend
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check
```

Expected: all normal tests pass; all-feature paid code compiles but no paid/ignored test runs; Clippy has zero warnings.

- [ ] **Step 4: Run exact non-paid media verification**

Run only:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
nix develop --command cargo test -p backend analysis::facade::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture
```

- [ ] **Step 5: Verify complete Synology removal**

Run:

```bash
rg -n 'Synology|synology|LEO_SYNOLOGY_URL|SynologyClient|download_batch|DownloadedVideo' \
  backend/src app/src Cargo.toml app/Cargo.toml backend/Cargo.toml \
  justfile README.md app/README.md docs/architecture.md
```

Expected: no matches. Then inspect `cargo metadata --no-deps --format-version 1` and confirm packages are exactly app/backend/camera.

- [ ] **Step 6: Perform local desktop acceptance**

Execute all non-paid steps 1-9 and 12-13 from the approved specification: two previews, all-camera readiness, cadence/participation, camera-2 disconnect/reconnect, Stop/finalization, marker/MKVs, gap warning, restart discovery, and temporary-media absence. Provider progress/results steps run only with an approved endpoint.

- [ ] **Step 7: Run paid acceptance only after explicit approval**

If approved, run exactly:

```bash
nix develop --command env LEO_RUN_PAID_OPENAI_TEST=1 \
  cargo test -p app \
  --features paid-openai-test \
  paid_openai_workflow::paid_openai_analyzes_one_local_application_session \
  -- --ignored --exact --nocapture
```

Record model name, request count, duration, and result without printing credentials.

- [ ] **Step 8: Report implementation**

Report behavior changes, deleted Synology code, exact automated/manual checks, paid-test status, physical-camera checks not run, crash/SSD/retention limits, and implementation difficulties.

Suggested commits if separately authorized:

```text
test(app): gate paid local analysis workflow
docs(app): document host recording workflow
```

## Dependency Order

```text
1 -> 2
1 -> 3
3 -> 4 -> 5 -> 6
3 -> 7 -> 8
6 + 8 -> 9
2 + 9 -> 10 -> 11
10 -> 12
11 + 12 -> 13
6 + 8 -> 14
all prior tasks -> 15
```

## Stop Conditions

- Stop and rework the plan with the user if reliable recorder cleanup would require a daemon/service, OS-specific process manager, or persistent manifest not described by the approved design.
- Stop before changing the storage format away from MKV stream copy.
- Stop before weakening all-camera initial readiness or post-gap analysis semantics.
- Stop before adding automatic deletion/retention, active-session crash recovery, or a generic recording-provider abstraction.
- Never solve a process leak by increasing sleeps/timeouts; establish ownership, interruption, kill, reap, and join evidence.
