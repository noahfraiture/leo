# Local Operator Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a complete local two-camera operator workflow from preview and durable session actions through background analysis, persisted progress, and visible results.

**Architecture:** Move the merged session/recording/analysis backend into one reusable library crate, then expose one concrete `analyze_session` facade to the Dioxus app. The app loads two stable camera definitions, stores completed sessions under `./sessions`, discovers them from JSONL, and runs one root-scoped analysis task that survives route navigation. The Synology simulator keeps fixed behavior by default and gains an explicit request-alignment mode for local finite fixtures.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7.9, Tailwind CSS 4, daisyUI, Tokio 1.53, Serde/JSON, Rig 0.41, Reqwest 0.13, FFmpeg 8 through `ffmpeg-sidecar` 2.5.2, Axum 0.8, `tracing`, SHA-256, Synology Recording List v5 and Download v6.

## Global Constraints

- Reuse commit `698ce32`; do not reimplement its session durability, List v5 client, Download v6 client, sampling, FFmpeg, Agent, Analyzer, or checkpoint tests.
- Cameras and Synology record independently from software sessions. Participation/cadence actions affect analysis only.
- Use one stable camera ID from `cameras.json` through preview, JSONL, Synology, frame metadata, and UI state. MediaMTX vector indices remain private path IDs.
- The user explicitly supersedes the old narrow-visibility rule for new and moved code: use private items or plain `pub`, never restricted `pub(...)`. Keep child implementation modules private and re-export only the documented API; do not normalize unrelated existing modules.
- Keep public backend data fields public instead of adding trivial getters.
- Keep every `mod.rs` declaration/re-export-only and keep non-trivial module errors in `error.rs` with `thiserror`.
- List v5 supplies UTC recording boundaries. Download v6 supplies bounded MP4 bytes. Do not parse v6 metadata as timestamped events.
- Query alignment is simulator-only, defaults off, and never appears in fixture JSON or Synology HTTP fields.
- Missing coverage at the first scheduled sample is an error. A later first gap truncates every camera at the same offset and persists a typed warning. Overlapping recordings remain errors.
- Checkpoint schema v2 persists checklist, SHA-256 plan fingerprint, warnings, total batches, and ordered responses. Save an initial zero-response checkpoint before the first model request.
- One analysis may run at a time; it must continue when Monitor/Analyze routes mount or unmount.
- Default camera cadence is one sample every 1,000 ms. UI cadence controls accept whole seconds. Analyzer batches exactly five frame sets.
- UI uses native semantic controls, Tailwind layout utilities, and daisyUI classes. Keep styling sparse: no gradients, animation, decorative shadows, or bespoke component system.
- Store sessions under `./sessions` and structured logs under `./logs`; ignore both in Git.
- Do not log credentials, SIDs, prompts/checklists, image data, or credential-bearing URLs.
- Paid OpenAI coverage requires Cargo feature `paid-openai-test`, `#[ignore]`, exact-name filtering, and runtime `LEO_RUN_PAID_OPENAI_TEST=1`. Compile-only feature checks are safe; never set the runtime gate, run the paid test, or run blanket ignored tests without explicit user approval.
- Do not add active-session recovery, multiple analyses, cancellation, session deletion/search, camera discovery, Settings UI, video playback on Analyze, permanent extracted media, physical authentication UI, or packaged sidecars.
- Do not commit this plan. Do not create implementation commits unless the user explicitly authorizes commits for the execution session.

## Locked File Structure

```text
app/
|-- AGENTS.md
|-- Cargo.toml
|-- cameras.json
|-- justfile
|-- backend/
|   |-- Cargo.toml
|   \-- src/
|       |-- lib.rs
|       |-- session/
|       |   |-- mod.rs
|       |   |-- controller.rs
|       |   |-- session.rs
|       |   |-- catalog.rs
|       |   \-- error.rs
|       |-- recording/
|       |   |-- mod.rs
|       |   |-- synology.rs
|       |   |-- video.rs
|       |   \-- error.rs
|       \-- analysis/
|           |-- mod.rs
|           |-- facade.rs
|           |-- error.rs
|           |-- agent/
|           |-- analyzer/
|           \-- video/
|-- app/
|   |-- Cargo.toml
|   \-- src/
|       |-- main.rs
|       |-- lib.rs
|       |-- analysis_task.rs
|       |-- camera_config.rs
|       |-- logging.rs
|       |-- workflow/
|       |   |-- mod.rs
|       |   |-- workflow.rs
|       |   \-- error.rs
|       |-- preview/
|       |-- components/
|       \-- views/
|-- camera/fixtures/
|   |-- salon-1.mp4
|   \-- salon-2.mp4
\-- synology/
    |-- fixtures/recordings.json
    \-- src/
```

---

### Task 1: Extract The Reusable Backend Library

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
- Modify: `app/src/analysis/analyzer/analyzer.rs`

**Interfaces:**
- Consumes: the complete merged backend and its current tests.
- Produces: `backend::{session, recording, analysis}` as a library dependency of `app` with no behavior change.

- [ ] **Step 1: Record the clean baseline**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
```

Expected: formatting passes; all current normal tests pass with only the existing ignored/gated media tests skipped.

- [ ] **Step 2: Add the backend package and workspace dependency**

Add `"backend"` to `[workspace].members`. Move dependencies shared by app/backend to the root manifest:

```toml
[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
thiserror = "2"
tokio = "1.53.0"
tracing = "0.1"
url = "2"
uuid = { version = "1.24.0", features = ["serde", "v4"] }
```

Create:

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
tokio = { workspace = true, features = ["fs", "io-util", "rt"] }
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
```

Create:

```rust
//! Reusable session recording and video-analysis backend.

pub mod analysis;
pub mod recording;
pub mod session;
```

Add to app dependencies:

```toml
backend = { path = "../backend" }
url = { workspace = true }
uuid = { workspace = true }
```

Immediately add `#[cfg(feature = "paid-openai-test")]` to the existing paid test and any imports/helpers used only by it. The feature may be compiled without approval, but its ignored test must not run.

- [ ] **Step 3: Mechanically move modules and tests**

Move the three directories without changing assertions. Remove `mod analysis;`, `mod recording;`, and `mod session;` from app. Update app imports from `crate::{analysis, recording, session}` to `backend::{analysis, recording, session}`.

Move `Video` and its documentation from `backend/src/analysis/video/video.rs` to `backend/src/recording/video.rs`, declare `mod video;` in `recording/mod.rs`, then update:

```rust
use crate::recording::{SynologyClient, Video};
```

inside Analyzer. `SynologyClient::list_videos` returns `Vec<Video>` from its own module. Preserve every field and test assertion.

- [ ] **Step 4: Normalize only moved/new visibility**

Use private child modules and plain-`pub` sibling APIs. Make these exact declarations/re-exports:

```rust
// backend/src/session/mod.rs
mod controller;
mod error;
mod session;

pub use controller::{OperatorAction, SessionController};
pub use error::Error;
pub use session::{Session, SessionCamera};

// backend/src/recording/mod.rs
mod error;
mod synology;
mod video;

pub use error::Error;
pub use synology::SynologyClient;
pub use video::Video;

// backend/src/analysis/mod.rs, until the facade is added in Task 4
mod agent;
mod analyzer;
mod video;
```

Change the re-exported types and their entry-point methods to plain `pub`, while leaving persisted event DTOs, prompt builders, extraction helpers, and child modules private. Add to `SessionController`:

```rust
/// Returns elapsed monotonic time since the session-start event was written.
pub fn elapsed(&self) -> Duration {
    self.log.started_at.elapsed()
}
```

Document every re-exported type and entry-point method. Plain `pub` items inside private `agent`, `analyzer`, and `video` modules remain unreachable outside `backend::analysis` unless Task 4 re-exports them.

Update `AGENTS.md`:

```markdown
- In new or substantially reorganized modules, use private items or plain `pub`; do not use restricted `pub(...)` visibility. Keep child modules private and expose only documented module APIs.
- Never set `LEO_RUN_PAID_OPENAI_TEST=1` or run a paid OpenAI test without explicit user approval. Compile-only feature checks are allowed. Never run blanket ignored tests; filter approved ignored tests by exact name.
```

Remove the conflicting narrowest-visibility rule.

- [ ] **Step 5: Remove dependencies no longer used by app**

Remove base64, `ffmpeg-sidecar`, Rig, Reqwest, and backend-only dev dependencies from `app/Cargo.toml`. Keep workspace UUID and URL because Workflow uses both directly, and keep workspace Axum as an app dev dependency because Task 12's single paid workflow test serves fixture HTTP media.

- [ ] **Step 6: Verify the mechanical extraction**

Run:

```bash
cargo fmt --all --check
cargo test -p backend
cargo test -p app
cargo test --workspace
```

Expected: every moved test passes with unchanged behavior; app compiles against backend; no duplicate app backend modules remain.

---

### Task 2: Checkpoint V2, Checklist Locking, And Plan Identity

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/analysis/analyzer/progress.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Move: `backend/src/analysis/analyzer/error.rs` -> `backend/src/analysis/error.rs`
- Modify: `backend/src/analysis/analyzer/mod.rs`
- Modify: `backend/src/analysis/mod.rs`
- Modify: `backend/src/analysis/agent/agent.rs`

**Interfaces:**
- Consumes: current checkpoint schema v1 and rebuilt frame-set plan.
- Produces: public `AnalysisCheckpoint::read(path, session_id)`, checklist and SHA-256 plan validation, ordered response vector, and initial pre-model checkpoint save.

- [ ] **Step 1: Write failing checkpoint-v2 tests**

In `progress.rs`, retain the existing `response()` helper and replace the v1 tests with these exact cases:

```rust
#[test]
fn checkpoint_v2_round_trips_all_resume_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("analysis.json");
    let expected = AnalysisCheckpoint {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        session_id: Uuid::from_u128(1),
        checklist: "Open the valve".into(),
        plan_fingerprint: "ab".repeat(32),
        total_batches: 2,
        warnings: vec![AnalysisWarning::RecordingCoverageEnded {
            camera_id: 2,
            session_offset_ms: 9_000,
        }],
        responses: vec![response("Batch zero is complete.")],
    };
    expected.save(&path).unwrap();
    assert_eq!(AnalysisCheckpoint::read(&path, expected.session_id).unwrap(), expected);
}

#[test]
fn read_rejects_a_checkpoint_for_another_session() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("analysis.json");
    AnalysisCheckpoint {
        schema_version: ANALYSIS_SCHEMA_VERSION,
        session_id: Uuid::from_u128(1),
        checklist: "Open the valve".into(),
        plan_fingerprint: "ab".repeat(32),
        total_batches: 0,
        warnings: vec![],
        responses: vec![],
    }.save(&path).unwrap();
    assert!(matches!(
        AnalysisCheckpoint::read(&path, Uuid::from_u128(2)),
        Err(Error::CheckpointSession { .. })
    ));
}
```

In `analyzer.rs`, adapt the existing `resume_analyzer` helper to accept checklist text and recording JSON, then add `resume_rejects_changed_checklist` and `resume_rejects_changed_plan_with_same_batch_count`. Each first run saves a zero-response checkpoint; each second run keeps two batches but changes exactly checklist text or one recording ID. Assert the second `Analyzer::resume` returns `CheckpointChecklist` or `CheckpointPlan` before the model has any requests. Rename the current model-failure test to `planning_saves_zero_response_checkpoint_before_model_failure` and assert `Analyzer::resume` creates a readable zero-response checkpoint before `submit_prompt` returns the provider error. In `progress.rs`, save a checkpoint with one warning and call private full-plan validation with an empty expected warning vector; assert `CheckpointWarnings`. Task 3 adds the generated-warning integration case after truncation exists.

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cargo test -p backend analysis::analyzer::progress -- --nocapture
cargo test -p backend planning_saves_zero_response_checkpoint_before_model_failure -- --nocapture
```

Expected: compilation/assertions fail because schema v2 fields, fingerprint validation, and initial save do not exist.

- [ ] **Step 3: Replace the checkpoint DTO**

Implement:

```rust
pub const ANALYSIS_SCHEMA_VERSION: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalysisWarning {
    RecordingCoverageEnded {
        camera_id: u32,
        session_offset_ms: u64,
    },
}

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
    pub fn read(path: &Path, expected_session_id: Uuid) -> Result<Self>;
}
```

Delete `CompletedBatch`; response index is vector position. `read` opens an existing file and rejects wrong schema/session, empty checklist/fingerprint, and `responses.len() > total_batches` without rebuilding media. Keep a private `load_or_new` used by Analyzer that returns `(checkpoint, checkpoint_was_missing)` and additionally validates expected checklist, fingerprint, total batches, and warnings.

Move the Analyzer error to `analysis/error.rs`, make it plain `pub`, and have agent, analyzer, progress, facade, and video code convert into that one top-level analysis error. Re-export `analysis::Error`; because its transparent source variants mention child errors, make those child error types plain `pub` and re-export them as `analysis::AgentError` and `analysis::VideoError`. This keeps every public `Result` signature legal while private child modules still hide implementation functions.

- [ ] **Step 4: Add stable plan fingerprinting**

Add `sha2 = "0.10"` to backend. Hash this exact byte sequence:

```text
ASCII "leo-analysis-plan-v1\0"
frame_sets_per_batch as checked u64 little-endian
frame_set_count as checked u64 little-endian
for each frame set:
  session_offset_ms as checked u64 little-endian
  frame_count as checked u64 little-endian
  for each frame:
    camera_id as u32 little-endian
    recording_id as u64 little-endian
    sample_index as checked u64 little-endian
    recording_offset_ms as checked u64 little-endian
```

Use checked conversions and lowercase hexadecimal SHA-256 output. Do not hash JPEG bytes, filesystem paths, checklist, warnings, or model responses. Add `fingerprint_encoding_is_stable` using batch size 5, one frame set at 1,000 ms, and one frame `(camera=2, recording=9, sample_index=3, recording_offset=250 ms)`; assert exactly `462a1eba19e2b1db0efde71d49726e22d9e0f15de71aa1eae083b7dba2183b65`.

- [ ] **Step 5: Save initial planning state before model work**

After plan reconstruction and checkpoint validation:

```rust
if checkpoint_was_missing {
    checkpoint.save(&progress_path)?;
}
```

The saved checkpoint already contains checklist, fingerprint, warnings, and total batches with an empty response vector. Existing checkpoint resume must not rewrite the file during planning.

Save before constructing any prompt or calling the model. Update `analyze_next` to push/pop `AnalysisResponse` directly. In `download_failure_happens_before_model_invocation`, `model_failure_does_not_modify_progress`, ignored `concrete_ffmpeg_extraction_failure_precedes_model_and_checkpoint`, and `resume_rebuilds_the_canonical_plan_and_fixed_batches`, replace `!checkpoint.exists()` with `AnalysisCheckpoint::read(...).responses.is_empty()`. For `failed_checkpoint_save_rolls_back_the_completed_batch`, let `resume` save to a valid path, then set the private `progress_path` to a missing-parent path before `submit_prompt`; assert the original checkpoint still has zero responses and in-memory state rolls back.

- [ ] **Step 6: Verify checkpoint behavior**

Run:

```bash
cargo fmt --all --check
cargo test -p backend analysis::analyzer::progress -- --nocapture
cargo test -p backend analysis::analyzer::analyzer -- --nocapture
```

Expected: all checkpoint and Analyzer tests pass; model/save rollback behavior remains intact.

---

### Task 3: Truncate At The First Missing Recording Sample

**Files:**
- Modify: `backend/src/analysis/video/video.rs`
- Modify: `backend/src/analysis/video/error.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Modify: `backend/src/analysis/error.rs`
- Modify: `backend/src/analysis/analyzer/progress.rs`

**Interfaces:**
- Consumes: schedules and strict recording matching.
- Produces: typed `AnalysisWarning::RecordingCoverageEnded`, globally truncated frame sets, and `NoAnalyzableFrames` when no initial coverage exists.

- [ ] **Step 1: Write failing sequence and Analyzer tests**

Replace the current `sequences_reject_missing_recording_coverage` test with:

```rust
#[test]
fn sequence_stops_at_its_first_uncovered_sample() {
    let exercise = session(true, Duration::from_secs(2), Duration::from_secs(7), vec![]);
    let schedule = SamplingSchedule::from_session(&exercise, 1).unwrap();
    let videos = vec![Video {
        recording_id: 10,
        camera_id: 1,
        start_utc_ms: SESSION_START_UTC_MS,
        end_utc_ms: SESSION_START_UTC_MS + 4_000,
    }];
    let sequence = SampleSequence::from_videos(SESSION_START_UTC_MS, &schedule, &videos).unwrap();

    assert_eq!(sequence.frames.len(), 2);
    assert_eq!(sequence.frames[0].session_offset, Duration::ZERO);
    assert_eq!(sequence.frames[1].session_offset, Duration::from_secs(2));
    assert_eq!(sequence.first_uncovered, Some(Duration::from_secs(4)));
}

```

Keep the existing `sequences_reject_overlapping_recording_coverage` test unchanged. In Analyzer tests, use the existing Axum recording helper and `MockCompletionModel`: camera 1 covers 0/1/2/3 seconds and camera 2 covers 0/1 seconds. Assert the resulting frame sets contain only offsets 0 and 1 for both cameras, warnings equal one `RecordingCoverageEnded { camera_id: 2, session_offset_ms: 2_000 }`, and the initial checkpoint persists that warning. Add a second case where neither camera covers offset zero; assert `Error::NoAnalyzableFrames`, zero model requests, and no checkpoint file.

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cargo test -p backend analysis::video::video -- --nocapture
cargo test -p backend analyzer_truncates_every_camera_at_the_earliest_gap -- --nocapture
```

Expected: current missing-coverage errors fail the new expectations.

- [ ] **Step 3: Record first uncovered sample in sequence planning**

Extend the internal sequence with:

```rust
struct SampleSequence {
    pub camera_id: u32,
    pub frames: Vec<Frame>,
    pub first_uncovered: Option<Duration>,
}
```

`SampleSequence` remains inside the private `analysis::video` module, so plain `pub` permits sibling Analyzer access without exposing the type from `backend::analysis`.

For each scheduled offset:

- zero matching recordings: set `first_uncovered`, stop that sequence;
- one match: append the frame;
- more than one match: return the existing overlap error.

- [ ] **Step 4: Apply one global truncation boundary**

In Analyzer planning:

```rust
let truncation = sequences
    .iter()
    .filter_map(|sequence| sequence.first_uncovered)
    .min();
```

Retain only frames with `session_offset < truncation`. Emit warnings only for sequences whose `first_uncovered == truncation`:

```rust
AnalysisWarning::RecordingCoverageEnded {
    camera_id: sequence.camera_id,
    session_offset_ms: u64::try_from(truncation.as_millis())
        .map_err(|_| Error::SessionOffsetOverflow)?,
}
```

Add `SessionOffsetOverflow` and `NoAnalyzableFrames` to the unified `analysis::Error` enum.

If merged frame sets are empty, return `Error::NoAnalyzableFrames` before creating `analysis.json` or calling the model.

- [ ] **Step 5: Verify truncation and unchanged overlap safety**

Run:

```bash
cargo fmt --all --check
cargo test -p backend analysis::video:: -- --nocapture
cargo test -p backend analysis::analyzer:: -- --nocapture
```

Expected: finite end truncates successfully, initial absence fails safely, and overlaps still fail.

---

### Task 4: Session Catalog And Concrete Analysis Facade

**Files:**
- Create: `backend/src/session/catalog.rs`
- Modify: `backend/src/session/mod.rs`
- Modify: `backend/src/session/session.rs`
- Modify: `backend/src/session/error.rs`
- Modify: `backend/src/analysis/mod.rs`
- Create: `backend/src/analysis/facade.rs`
- Modify: `backend/src/analysis/error.rs`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Modify: `backend/src/analysis/error.rs`

**Interfaces:**
- Produces: durable completed-session discovery and one public `analyze_session` entrypoint used by desktop and future CLI.

- [ ] **Step 1: Write failing catalog tests**

Reuse the session module's JSON event builders to create direct children `100/events.jsonl` and `200/events.jsonl` with completed sessions, `active/events.jsonl` without an end, `bad/events.jsonl` with malformed JSON, a root-level unrelated file, and `nested/child/events.jsonl`. Assert:

```rust
let sessions = list_sessions(root).unwrap();
assert_eq!(sessions.iter().map(|s| s.session.start_utc_ms).collect::<Vec<_>>(), newest_first);
assert_eq!(sessions[0].events_path, expected_events_path);
```

A missing root returns an empty vector. Passing a regular file as the root returns `session::Error::Io`; this avoids permission-dependent tests.

- [ ] **Step 2: Implement the minimal catalog**

```rust
#[derive(Debug)]
pub struct StoredSession {
    pub events_path: PathBuf,
    pub session: Session,
}

pub fn list_sessions(root: &Path) -> Result<Vec<StoredSession>>;
```

Scan direct directories only. Skip invalid/incomplete logs with `tracing::warn!` and sort by descending `(start_utc_ms, id)`. Derive directory and `analysis.json` path at call sites.

Declare `mod catalog;` and re-export `list_sessions` and `StoredSession` from `session/mod.rs`. Add a `CatalogEntry` context variant to public `session::Error` only if an underlying path needs to be included; otherwise reuse `Error::Io` and the existing parse variants.

- [ ] **Step 3: Write failing facade tests using existing mocks**

Move the existing ignored `full_http_ffmpeg_and_model_pipeline_uses_the_existing_fixture` test to `facade.rs`, keep its real Axum media server and Rig `MockCompletionModel`, and route it through:

```rust
async fn analyze_session_with<M: CompletionModel>(
    request: AnalyzeSession,
    agent: Agent<M>,
    synology: SynologyClient,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint>;
```

Collect callback values in `Arc<Mutex<Vec<AnalysisCheckpoint>>>`. For the existing one-batch fixture assert callback response lengths are `[0, 1]`, the returned checkpoint equals callback index 1, and the checkpoint file equals the returned value. Keep the test ignored because it invokes FFmpeg. Add a normal empty-checklist test against this helper and assert the model has zero requests and the events file is not opened.

- [ ] **Step 4: Implement the public concrete facade**

```rust
pub struct AnalyzeSession {
    pub events_path: PathBuf,
    pub checklist: String,
    pub synology_url: url::Url,
}

pub async fn analyze_session(
    request: AnalyzeSession,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint>;
```

Implement `facade.rs` in this order: trim/reject checklist; load `Session`; derive `analysis.json`; construct `OpenAiAgent::from_env`; construct `SynologyClient::new(request.synology_url)`; and call the private helper with `NonZeroUsize::new(5).expect("five is non-zero")`. The helper calls `Analyzer::resume`, emits `analyzer.checkpoint().clone()` after its initial save, loops while `next_batch_index < total_batches`, calls `analyze_next`, emits the newly saved complete snapshot, and returns `analyzer.into_checkpoint()`.

Extend the documented public `analysis::Error` with session and `EmptyChecklist` variants. In `analysis/mod.rs`, keep only declarations and these re-exports:

```rust
mod agent;
mod analyzer;
mod error;
mod facade;
mod video;

pub use agent::{AnalysisResponse, ChecklistProgress, Observation};
pub use analyzer::{AnalysisCheckpoint, AnalysisWarning};
pub use agent::Error as AgentError;
pub use error::Error;
pub use facade::{AnalyzeSession, analyze_session};
pub use video::Error as VideoError;
```

Make the re-exported DTOs and fields plain `pub`; keep `Agent`, `OpenAiAgent`, Analyzer, and helpers inside private child modules. Add structured events for catalog skips, planning, truncation, each batch start/save, failure, and completion without prompt/checklist contents or credential-bearing URLs.

- [ ] **Step 5: Verify the backend facade**

Run:

```bash
cargo fmt --all --check
cargo test -p backend session::catalog -- --nocapture
cargo test -p backend analysis::facade::tests::empty_checklist_fails_before_io_or_model -- --exact
cargo test -p backend
```

Expected: catalog and concrete-facade behavior pass while existing backend regressions remain green.

---

### Task 5: Two-Camera Simulator Alignment And Media

**Files:**
- Modify: `synology/src/cli.rs`
- Modify: `synology/src/main.rs`
- Modify: `synology/src/server.rs`
- Modify: `synology/src/api/mod.rs`
- Modify: `synology/src/api/entry.rs`
- Modify: `synology/src/api/recording.rs`
- Modify: `synology/src/api/external_recording.rs`
- Modify: `synology/Cargo.toml`
- Modify: `synology/fixtures/recordings.json`
- Create: `camera/fixtures/salon-1.mp4`
- Create: `camera/fixtures/salon-2.mp4`
- Modify: `justfile`

**Interfaces:**
- Produces: fixed-by-default simulator behavior, opt-in request-time UTC projection, two finite synchronized recordings, and two preview launch recipes.

- [ ] **Step 1: Write failing CLI/state/alignment tests**

Extend the existing Clap tests with:

```rust
let default = Args::try_parse_from([
    "synology", "--address", "127.0.0.1:5000",
    "--camera", "127.0.0.1:8080",
]).unwrap();
assert!(!default.align_recordings_to_query);

let aligned = Args::try_parse_from([
    "synology", "--address", "127.0.0.1:5000",
    "--camera", "127.0.0.1:8080", "--align-recordings-to-query",
]).unwrap();
assert!(aligned.align_recordings_to_query);
```

In Recording API tests, add two recordings with starts 100 and 105, durations 24 and 10, then request `fromTime=1_000`. Assert projected v5 bounds are `1_000..1_024` and `1_005..1_015`; relative bounds, sort, camera/DS/mount filters, and pagination stay intact. Assert v6 uses the projected overlap for selection but has no `startTime`/`stopTime`; `fromTime=0` returns fixed values; `fromTime=u64::MAX` returns error code 401. Keep every existing fixed-mode response test unchanged.

- [ ] **Step 2: Run tests and verify red**

Run:

```bash
cargo test -p synology cli:: -- --nocapture
cargo test -p synology api::recording:: -- --nocapture
```

Expected: flag/state/projection tests fail because alignment does not exist.

- [ ] **Step 3: Add immutable API state and projection**

```rust
#[derive(Clone)]
pub struct ApiState {
    pub cameras: CameraState,
    pub align_recordings_to_query: bool,
}
```

`api` remains a private crate module, so plain `pub` only permits sibling access. Change `api::router() -> Router<ApiState>`, `entry::handle(State(state): State<ApiState>, ...)`, and `server::start(cameras, address, align)`. Entry passes `state.cameras` to Camera and ExternalRecording; Recording receives both camera state and the alignment bool. Existing `tests::app(cameras)` wraps `ApiState { cameras, align_recordings_to_query: false }`; add `app_with_alignment(cameras, true)`. Update `external_recording::tests::starts_and_stops_one_camera` to call the helper/state wrapper rather than binding `CameraState` directly.

Before filtering, find the earliest catalogue start across all cameras. For each List row, compute:

```rust
fn projected_bounds(recording: &Recording, earliest: u64, from_time: u64, align: bool)
    -> Result<(u64, u64), ApiError>
{
    if !align || from_time == 0 {
        return Ok((recording.start_time, recording.stop_time));
    }
    let relative_start = recording.start_time.checked_sub(earliest)
        .ok_or(ApiError::InvalidRecordingParameters)?;
    let duration = recording.stop_time.checked_sub(recording.start_time)
        .ok_or(ApiError::InvalidRecordingParameters)?;
    let start = from_time.checked_add(relative_start)
        .ok_or(ApiError::InvalidRecordingParameters)?;
    let stop = start.checked_add(duration)
        .ok_or(ApiError::InvalidRecordingParameters)?;
    Ok((start, stop))
}
```

Use projected bounds for List filtering, sorting, pagination, and v5 serialization. V6 selection uses them but emits its existing metadata. Download receives no alignment value and remains unchanged. Log alignment mode once at startup and projected List bounds at debug level without request credentials.

- [ ] **Step 4: Prepare browser/range-safe salon fixtures**

Run the existing deterministic transform for each tracked synchronized source:

```bash
just prepare-camera-video ../videos/salon-1-synced.mp4 camera/fixtures/salon-1.mp4
just prepare-camera-video ../videos/salon-2-synced.mp4 camera/fixtures/salon-2.mp4
```

Probe both and require H.264 Constrained Baseline, 1280x720, 15 FPS, no audio, no B-frames, one-second GOPs, and duration at least 24 seconds.

Run for each output:

```bash
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,profile,level,width,height,r_frame_rate,has_b_frames -show_entries format=duration -of json camera/fixtures/salon-1.mp4
ffprobe -v error -select_streams a -show_entries stream=index -of csv=p=0 camera/fixtures/salon-1.mp4
ffprobe -v error -skip_frame nokey -select_streams v:0 -show_entries frame=best_effort_timestamp_time -of csv=p=0 camera/fixtures/salon-1.mp4
```

Expected: `h264`, `Constrained Baseline`, level `31`, `1280x720`, `15/1`, `has_b_frames=0`, duration at least `24.0`; the audio command prints nothing; keyframe timestamps increase by one second. Repeat with `salon-2.mp4`.

- [ ] **Step 5: Replace the local catalogue with two rows**

Use IDs/camera IDs 1 and 2, `dsId=0`, `mountId=0`, identical fixed starts, stops exactly 24 seconds later, private paths to the prepared files, codec 3, audio codec 0, dimensions 1280x720.

- [ ] **Step 6: Add two-camera recipes**

```just
camera-1:
    nix develop --command cargo run -p camera -- --address 127.0.0.1:8080 --rtsp-address 127.0.0.1:8554 --video camera/fixtures/salon-1.mp4

camera-2:
    nix develop --command cargo run -p camera -- --address 127.0.0.1:8081 --rtsp-address 127.0.0.1:8555 --video camera/fixtures/salon-2.mp4

synology:
    nix develop --command cargo run -p synology -- --address 127.0.0.1:5000 --camera 127.0.0.1:8080 --camera 127.0.0.1:8081 --recording-catalogue synology/fixtures/recordings.json --align-recordings-to-query
```

- [ ] **Step 7: Verify simulator and media**

Run:

```bash
cargo fmt --all --check
cargo test -p synology
nix develop --command cargo test -p synology api::recording::tests::downloads_partial_recording -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
```

Expected: all normal tests and exact approved non-paid media tests pass. Do not run blanket ignored tests.

---

### Task 6: App Library, Camera Configuration, And Stable Preview Identity

**Files:**
- Create: `cameras.json`
- Create: `app/src/lib.rs`
- Create: `app/src/camera_config.rs`
- Modify: `app/src/main.rs`
- Modify: `app/src/preview/bridge.rs`
- Modify: `app/src/preview/config.rs`
- Modify: `app/src/components/camera/feed.rs`
- Modify: `app/src/views/mod.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**
- Produces: importable app library, validated runtime camera definitions, and stable camera IDs in preview metadata.

- [ ] **Step 1: Write failing camera-config tests**

Write `parses_exact_two_camera_configuration` using the checked-in JSON values, then table-driven rejection tests for one/three rows, unknown fields, zero/duplicate IDs, blank names/URLs, zero interval, missing file, and malformed JSON. Assert concrete `camera_config::Error` variants rather than message substrings.

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

pub fn load_cameras(path: &Path) -> Result<Vec<CameraConfig>>;
```

Define a documented `thiserror::Error` in the same small module with `Read { path, source }`, `Parse { path, source }`, `CameraCount { actual }`, `ZeroId`, `DuplicateId { id }`, `EmptyName { id }`, `EmptyRtspUrl { id }`, and `ZeroInterval { id }`. Deserialize with `deny_unknown_fields`, require exactly two rows, and preserve file order for MediaMTX path assignment.

- [ ] **Step 2: Run tests and verify red**

Run `cargo test -p app camera_config -- --nocapture`.

Expected: module/types do not exist.

- [ ] **Step 3: Add app library and thin executable**

Move current startup/modules/routes/App into `app/src/lib.rs` and expose:

```rust
pub fn launch();
```

Replace main with:

```rust
fn main() {
    app::launch();
}
```

`LaunchBuilder::launch` returns `()`, so do not use a diverging return type. Keep bridge ownership and event-loop cleanup behavior unchanged. Define cloneable bootstrap data passed through `LaunchBuilder::with_context`:

```rust
#[derive(Clone)]
pub enum Bootstrap {
    Ready {
        cameras: Vec<CameraConfig>,
        preview: PreviewState,
    },
    Unavailable {
        message: String,
    },
}
```

Make root `App` plain `pub`. It reads `Bootstrap` and renders either a top-level `role="alert"` configuration error with no router/session controls or a `ReadyApp` child. `ReadyApp` provides `PreviewState` and renders the router. Task 8 extends only the Ready branch with Workflow state.

Keep implementation modules private but add these documented root re-exports for external integration tests and future executable callers:

```rust
pub use camera_config::CameraConfig;
pub use preview::{PreviewFeed, PreviewState};
pub use views::{Analyze, AnalyzeSidebar, Monitor, MonitorSidebar};
```

Keep `views/mod.rs` declaration/re-export-only and expose the aliases there first:

```rust
pub use analyze::{Analyze, Sidebar as AnalyzeSidebar};
pub use monitor::{Monitor, Sidebar as MonitorSidebar};
```

Make the re-exported components/types and required data fields plain `pub`; do not expose `Bridge`, `ConfigFile`, reader internals, or route implementation helpers.

- [ ] **Step 4: Implement camera loading and startup errors**

Load `LEO_CAMERA_CONFIG` or default `./cameras.json`. Map valid config into exactly two `CameraSource`s. On camera-config failure, launch `Bootstrap::Unavailable` rather than starting MediaMTX or panicking. A MediaMTX startup failure still uses `Bootstrap::Ready` with `PreviewState::Unavailable`, so valid session controls remain usable.

Add the approved `cameras.json` with IDs 1/2, Salon names, RTSP ports 8554/8555, enabled true, and 1,000 ms intervals.

- [ ] **Step 5: Carry stable identity through preview**

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

Keep MediaMTX paths/indexes unchanged. Add a regression using IDs 26 and 41 that still produces paths `camera-0` and `camera-1` while preserving IDs.

Also map the same configs to `SessionCamera` and assert IDs, names, initial `enabled`, and `Duration::from_millis(sample_every_ms)` are unchanged; this is the stable-identity regression required by Workflow.

- [ ] **Step 6: Remove unused UI dependencies**

Remove `dioxus-icons` and `dioxus-primitives`. Keep workspace Serde, Serde JSON, thiserror, URL, and UUID required by config/workflow; do not add a UI framework.

- [ ] **Step 7: Verify startup/config/preview**

Run:

```bash
cargo fmt --all --check
cargo test -p app camera_config -- --nocapture
cargo test -p app preview:: -- --nocapture
cargo test -p app components::camera:: -- --nocapture
```

Expected: config and stable-ID tests pass; preview security/lifecycle tests remain green.

---

### Task 7: Structured Logging And Runtime Ownership

**Files:**
- Create: `app/src/logging.rs`
- Modify: `app/src/lib.rs`
- Modify: `app/Cargo.toml`
- Modify: `backend/Cargo.toml`
- Modify: `Cargo.toml`
- Modify: `.gitignore`
- Modify: `app/src/preview/bridge.rs`
- Modify: `backend/src/session/controller.rs`
- Modify: `backend/src/session/catalog.rs`
- Modify: `backend/src/analysis/facade.rs`

**Interfaces:**
- Produces: leveled console output, daily JSONL logs, retained non-blocking guard, and safe structured events.

- [ ] **Step 1: Add dependencies and write a failing initialization test**

Workspace `tracing` already exists from Task 1. Add `tracing = { workspace = true }`, `tracing-subscriber` with `env-filter,json`, and `tracing-appender` to app dependencies. In the only subscriber-installing test, initialize against a temporary directory, emit `tracing::info!(camera_id = 1, "preview configured")`, drop the guard, and assert exactly one `leo.jsonl.YYYY-MM-DD` file contains JSON fields `level:"INFO"`, `camera_id:1`, and message `preview configured`.

- [ ] **Step 2: Implement logging initialization**

```rust
pub struct LogGuard {
    _worker: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init(log_directory: &Path) -> Result<LogGuard>;
```

Define `logging::Error` with `thiserror` variants for directory I/O and `tracing::subscriber::SetGlobalDefaultError`. Create the directory; build `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))`; use `tracing_subscriber::fmt::layer().compact()` for stdout and a JSON fmt layer writing to `tracing_appender::rolling::daily(log_directory, "leo.jsonl")`; install the registry with `tracing::subscriber::set_global_default`; return the non-blocking `WorkerGuard`.

- [ ] **Step 3: Retain logging and bridge owners until shutdown**

Call `logging::init(Path::new("./logs"))` before configuration/preview startup. Store `LogGuard` with `Option<Bridge>` in the existing custom event-handler closure. Stop/reap Bridge on `LoopDestroyed`, then drop the logging guard after the event loop returns. If logging initialization fails, print one stderr startup error and launch without file logging rather than preventing camera/session use.

- [ ] **Step 4: Replace touched lifecycle prints with structured events**

Replace preview `eprintln!` lifecycle paths and add session-controller/catalog/facade fields for paths, session/camera IDs, batch indices, counts, and sanitized error display values. Tasks 8-11 add their workflow/UI action events when those call sites exist. Never log checklist text, API key, SID/password, base64 images, or raw credential URLs.

- [ ] **Step 5: Ignore runtime artifacts and verify**

Add `/sessions/` and `/logs/` to `.gitignore`.

Run:

```bash
cargo fmt --all --check
cargo test -p app logging -- --nocapture
cargo test -p backend
```

Expected: logging test passes and backend behavior is unchanged.

---

### Task 8: Session Catalog Workflow And Faulted Writes

**Files:**
- Modify: `app/src/lib.rs`
- Create: `app/src/workflow/error.rs`
- Create: `app/src/workflow/mod.rs`
- Create: `app/src/workflow/workflow.rs`
- Create: `app/tests/operator_session_flow.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**
- Produces: one testable Workflow state for camera selection, durable session actions, completed-session discovery, and faulted-write safety.

- [ ] **Step 1: Write failing pure workflow tests**

Add named tests for all of these transitions:

- construction and newest-first session refresh;
- deterministic timestamp directory path;
- existing timestamp directory leaves state Idle and is never reused;
- controller creation failure leaves state Idle and preserves the directory for inspection;
- Start snapshots current camera participation/cadence;
- select camera;
- successful participation/cadence writes update state after JSONL;
- any apply error moves to Faulted and blocks more writes;
- successful Stop produces valid session and refreshes list;
- older sessions remain visible;
- checkpoint session mismatch becomes row error;
- invalid Synology URL disables only `begin_analysis`.

- [ ] **Step 2: Define the minimal state**

Put these exact state types and all logic in `workflow/workflow.rs`. Keep `workflow/mod.rs` declaration/re-export-only and put the non-trivial `thiserror` type in `workflow/error.rs`.

```rust
pub struct CameraState {
    pub config: CameraConfig,
    pub participating: bool,
}

pub enum SessionRunState {
    Idle,
    Active {
        directory: PathBuf,
        controller: SessionController,
    },
    Faulted {
        directory: PathBuf,
        message: String,
    },
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
    pub message: Option<String>,
    session_root: PathBuf,
    pub synology_url: std::result::Result<Url, String>,
}
```

Do not duplicate camera vectors in Active state or progress fields outside checkpoints.

Define documented `workflow::Error` variants with `thiserror`: `Session(backend::session::Error)`, `Io(std::io::Error)`, `SessionAlreadyActive`, `SessionNotActive`, `SessionFaulted`, `UnknownCamera(u32)`, `NoCameraSelected`, `NoSessionSelected`, `UnknownSession(Uuid)`, `EmptyChecklist`, `AnalysisAlreadyRunning(Uuid)`, `InvalidCheckpoint(Uuid)`, and `AnalysisConfiguration(String)`. Re-export `Workflow`, its state DTOs, and `Error` from the app library for integration tests.

- [ ] **Step 3: Implement session entrypoints**

```rust
pub fn new(
    cameras: Vec<CameraConfig>,
    session_root: PathBuf,
    synology_url: std::result::Result<Url, String>,
) -> Self;
pub fn select_camera(&mut self, camera_id: u32) -> Result<()>;
pub fn select_session(&mut self, session_id: Uuid) -> Result<()>;
pub fn start_session(&mut self, utc_ms: i64) -> Result<()>;
pub fn set_selected_participation(&mut self, enabled: bool) -> Result<()>;
pub fn set_selected_interval(&mut self, sample_every: Duration) -> Result<()>;
pub fn stop_session(&mut self) -> Result<()>;
pub fn refresh_sessions(&mut self) -> Result<()>;
```

`new` initializes camera participation from config, selects the first camera, and calls `refresh_sessions`. If that initial refresh fails, it keeps an empty list and stores the error in `message`, so root Dioxus state remains a plain `Signal<Workflow>`. `refresh_sessions` calls `list_sessions`, reads an existing sibling `analysis.json` with `AnalysisCheckpoint::read`, retains invalid checkpoint errors on their rows, sorts newest-first, and preserves a still-valid selected session ID.

`start_session` rejects non-Idle state, runs `create_dir_all(session_root)`, then `create_dir(session_root.join(utc_ms.to_string()))` so collisions fail. It maps current camera state to `SessionCamera`, creates `events.jsonl` with `SessionController::create`, and only then stores Active. If controller creation fails, return the error while leaving Workflow Idle and the new directory inspectable.

For each controller action, first replace `self.session` with Idle to own the controller. On success, restore Active and then update camera state. On error, store Faulted with the same directory and error text; never restore/retry the controller. `set_selected_interval` lets `SessionController` validate zero/overflow, then stores checked milliseconds in the selected `CameraConfig` only after success. This makes a zero-duration unit call a deterministic fault-transition test while the native UI prevents zero through `min=1`. On Stop success, leave Idle and call `refresh_sessions`; if refresh fails, remain Idle and return the catalog error. Add info/warn events for start/action/stop/fault with IDs and paths, never checklist contents.

- [ ] **Step 4: Add the application integration test**

`app/tests/operator_session_flow.rs` creates two `CameraConfig`s and `Workflow::new(..., Url::parse("http://127.0.0.1:5000").map_err(|error| error.to_string()))` in a temporary root. Start at UTC `1_786_291_200_000`, select camera 2, set cadence to two seconds, exclude then include it, and Stop. Assert the only directory is `1786291200000`, reload `events.jsonl` through `Session::load`, and assert actions are cadence 2s, participation false, participation true in order. Construct a second Workflow against the same root and assert the completed UUID is rediscovered. Attempt the same timestamp again and assert the path-collision error leaves state Idle and the first file unchanged. No Dioxus or model process is required.

- [ ] **Step 5: Verify workflow**

Run:

```bash
cargo fmt --all --check
cargo test -p app workflow -- --nocapture
cargo test -p app --test operator_session_flow -- --nocapture
```

Expected: state and durable integration tests pass.

---

### Task 9: Root-Scoped Background Analysis State

**Files:**
- Modify: `app/src/workflow/workflow.rs`
- Modify: `app/src/lib.rs`
- Create: `app/src/analysis_task.rs`
- Create: `app/tests/analysis_workflow.rs`

**Interfaces:**
- Consumes: backend `AnalyzeSession`, `AnalysisCheckpoint`, and `analyze_session`.
- Produces: one running job, checkpoint snapshot projection, route-independent root task, and retry state.

- [ ] **Step 1: Write failing checkpoint-projection tests**

Add named tests for all of these analysis transitions:

- empty checklist rejected before Running state;
- existing checkpoint checklist overrides textarea input;
- second analysis start rejected;
- initial checkpoint snapshot sets progress/warnings and locks checklist;
- later snapshots replace, not append to, row state;
- all observations aggregate across responses;
- latest response supplies summary/checklist;
- failure clears running ID but preserves checkpoint;
- final successful checkpoint clears running ID;
- retry uses persisted checklist;
- invalid checkpoint and invalid startup URL fail without setting Running.

- [ ] **Step 2: Implement analysis state methods**

```rust
pub fn begin_analysis(&mut self, checklist: String) -> Result<AnalyzeSession>;
pub fn apply_checkpoint(&mut self, checkpoint: AnalysisCheckpoint);
pub fn analysis_failed(&mut self, session_id: Uuid, message: String);
```

`begin_analysis` derives events path from selected row and uses Workflow's startup-loaded Synology URL. Store only `running_analysis_id` and errors; derive progress/results from checkpoints.

Validate in this exact order: selected row exists; no other analysis runs; row checkpoint is not `Err`; startup Synology URL is `Ok`; checklist is persisted checkpoint text or trimmed non-empty input. Only after all checks pass, clear that row's previous analysis error, set `running_analysis_id`, and return `AnalyzeSession { events_path, checklist, synology_url }`. Never replace an invalid checkpoint.

`apply_checkpoint` finds the row by `checkpoint.session_id`, replaces its checkpoint snapshot, clears that row's previous error, and clears the matching running ID when `responses.len() == total_batches`. Ignore/log snapshots for unknown session IDs rather than changing another row. `analysis_failed` clears only a matching running ID and stores `(session_id, message)` while preserving the row checkpoint.

- [ ] **Step 3: Add the Dioxus root task launcher**

Implement one helper called by Analyze UI:

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
        }).await;
        if let Err(error) = result {
            workflow.write().analysis_failed(session_id, error.to_string());
        }
    });
}
```

Do not hold `workflow.write()` across `.await`.

At startup, parse `LEO_SYNOLOGY_URL` or `http://127.0.0.1:5000` once into `Result<Url, String>` and add it plus `PathBuf::from("./sessions")` to `Bootstrap::Ready`. In `ReadyApp`, create `let workflow = use_signal(|| Workflow::new(cameras, session_root, synology_url));` and `use_context_provider(|| workflow)` above `Router`. Do not pass a Dioxus `Signal` through `LaunchBuilder::with_context`; it is root-scope state, not cloneable bootstrap data.

- [ ] **Step 4: Add workflow integration coverage**

In `app/tests/analysis_workflow.rs`, build a completed stored session and synthetic checkpoints with the same UUID, checklist `"Open the valve"`, fingerprint `"ab".repeat(32)`, two total batches, one truncation warning, and zero/one/two responses. Feed snapshots through `apply_checkpoint`; assert replacement rather than append, all observations aggregate in response order, latest summary/checklist win, and the two-response snapshot clears Running. Call `analysis_failed` after a one-response snapshot and assert progress remains; call `begin_analysis("")` and assert the persisted checklist is used. Build separate invalid-checkpoint and invalid-URL workflows and assert both reject before setting Running.

- [ ] **Step 5: Verify analysis state**

Run:

```bash
cargo fmt --all --check
cargo test -p app --test analysis_workflow -- --nocapture
cargo test -p app workflow -- --nocapture
```

Expected: root analysis state and retry projection pass without making provider requests.

---

### Task 10: Monitor Session Controls And Camera Status

**Files:**
- Modify: `app/src/components/camera/feed.rs`
- Modify: `app/src/views/monitor/monitor.rs`
- Modify: `app/src/views/monitor/sidebar.rs`
- Modify: `app/src/views/navbar.rs`
- Modify: `app/Cargo.toml`
- Create: `app/tests/render_workflow.rs`

**Interfaces:**
- Produces: selectable two-camera preview, included/excluded indicators, session controls, one-second cadence input, elapsed display, and fault UI.

- [ ] **Step 1: Write failing component render tests**

Add `dioxus-ssr = "0.7.9"` as an app dev dependency only. In `app/tests/render_workflow.rs`, create `Rc<RefCell<Option<Workflow>>>` seed context; a `TestRoot` takes the Workflow once into `use_signal`, provides `Signal<Workflow>` plus prepared `PreviewState`, and renders `MonitorSidebar` together with `Monitor` because the real layout composes sidebar and body separately. Build/rebuild using `VirtualDom` plus `NoOpMutations`, then call `dioxus_ssr::render(&dom)`.

Render Idle, Active, and Faulted workflows and assert the HTML contains the corresponding semantic controls/text. In Active state assert both preview video IDs remain present when camera 2 is excluded, there is exactly one `reader.js` script, the interval input has `type="number"`, `min="1"`, and value `1`, and status/error containers have `role="status"` or `role="alert"`. Seed `Workflow.message` and assert the shared alert is rendered. Assert the old strings `LIVE`, `14:42:18`, `CAM 04`, and `Camera options` are absent. Render public `App` with root context `Bootstrap::Unavailable` separately and assert no `Start session` control exists.

- [ ] **Step 2: Narrow CameraFeed props**

```rust
#[component]
pub fn CameraFeed(
    feed: PreviewFeed,
    selected: bool,
    participating: bool,
    on_select: EventHandler<u32>,
) -> Element;
```

Replace per-card `document::Script` with one native `script { src: script_url, defer: true }` in Monitor so SSR and desktop use the same element exactly once. Add stable feed key. Remove hard-coded `LIVE`, time, `Selected`, `CAM 04`, and no-op options button.

- [ ] **Step 3: Implement camera selection/status**

Add semantic Select button, minimal selected border, and labelled daisyUI badge/status dot. Use `Included`/`Excluded`, never `Recording`, because recording continues.

- [ ] **Step 4: Implement Monitor sidebar states**

Use Workflow context. Add Start, Include/Exclude, whole-second interval input plus Apply, Stop, paths, and Faulted guidance. Disable active-only actions outside Active. Put active camera controls in a keyed child component using selected camera ID as its RSX `key`, so changing selection remounts the local interval text at that camera's current value instead of leaking the previous camera's edit.

Start gets UTC milliseconds from `SystemTime` and calls `workflow.start_session`. Every Monitor handler, including camera selection, evaluates its Workflow call into a local result and then sets `workflow.write().message = result.err().map(|error| error.to_string())`; success therefore clears the previous transient error. In the shared Ready layout, render `Workflow.message` once as a `role="alert"` live region above route content so Monitor and Analyze failures remain visible after navigation.

- [ ] **Step 5: Add elapsed ticker**

Enable Tokio `time`. Use Monitor-local `use_future` with a loop that sleeps one second and increments a private `u64` tick signal while mounted; read the tick in render but calculate text from `SessionController::elapsed`. Route unmount drops the future; remount starts a new display ticker without changing the controller.

- [ ] **Step 6: Keep the layout simple and responsive**

Use existing Tailwind/daisyUI classes only. Ensure fixed sidebar/body do not overflow at narrow window widths. Remove the inert Settings control rather than styling it.

- [ ] **Step 7: Verify Monitor**

Run:

```bash
cargo fmt --all --check
cargo test -p app views::monitor -- --nocapture
cargo test -p app components::camera -- --nocapture
cargo test -p app --test render_workflow -- --nocapture
```

Expected: render tests prove both feeds remain mounted, controls/labels are semantic, and old fake status text is gone. Defer generated Tailwind output until Analyze classes are complete in Task 11.

---

### Task 11: Analyze Session List, Progress, And Results

**Files:**
- Modify: `app/src/views/analyze/sidebar.rs`
- Modify: `app/src/views/analyze/analyze.rs`
- Modify: `app/src/views/sidebar.rs`
- Modify: `app/src/views/navbar.rs`
- Modify: `app/src/workflow/workflow.rs`
- Modify: `app/tests/render_workflow.rs`
- Modify: `justfile`
- Regenerate: `app/assets/tailwind.css`

**Interfaces:**
- Produces: durable all-session selection, recap, checklist lock, explicit background start, checkpoint progress, warnings, observations, summary, and checklist results.

- [ ] **Step 1: Write failing result-projection and render tests**

Extend `render_workflow.rs` with an Analyze `TestRoot` that renders `AnalyzeSidebar` together with `Analyze`. Render no sessions, two sessions, invalid checkpoint, invalid Synology URL, zero-response checkpoint, one-of-two partial checkpoint, two-of-two complete checkpoint, running, and failed states. Assert the invalid-URL alert exists while Analyze/Resume is disabled, plus exact guidance/status labels, `<progress value="..." max="...">`, truncation warning text, observations from both responses in order, and summary/checklist text only from the latest response. Assert no provider-related control fires during render; event invocation remains covered through Workflow methods rather than fake native clicks.

- [ ] **Step 2: Implement Analyze sidebar**

Add Refresh button and newest-first session buttons. Each shows start UTC and one derived status: `Not started`, `Running`, `In progress`, `Complete`, `Complete with warning`, `Failed`, or `Invalid checkpoint`. Refresh and selection set `Workflow.message` to the optional error exactly like Monitor handlers and never start analysis.

- [ ] **Step 3: Implement selected-session recap**

Display UUID, UTC start, duration, cameras, directory, durable file paths, and checkpoint errors. Do not add video preview or raw JSON viewer.

- [ ] **Step 4: Implement checklist and explicit start/resume**

Render the selected session body through a child keyed by session UUID, so changing selection remounts its local textarea draft. Use a local textarea signal only when no checkpoint exists. Prefill/read-only from a valid persisted checkpoint; disable the action for an invalid or complete checkpoint. If `workflow.synology_url` is `Err`, render a persistent `role="alert"` saying analysis configuration is invalid and disable Analyze/Resume before any click. On click, handle Workflow errors explicitly:

```rust
let Some(session_id) = workflow.read().selected_session_id else {
    return;
};
let result = workflow.write().begin_analysis(checklist());
match result {
    Ok(request) => spawn_analysis(workflow, request, session_id),
    Err(error) => workflow.write().message = Some(error.to_string()),
}
```

Do not start on mount, selection, or text input.

- [ ] **Step 5: Render progress and results from checkpoint**

Progress is `responses.len() / total_batches`. Render typed truncation warnings first, then all observations, latest sequence summary, and latest checklist entries. Running status comes from matching `running_analysis_id`.

- [ ] **Step 6: Verify route-independent UI state**

In the Workflow integration test, select Analyze state, apply one checkpoint, perform no view-owned mutation while representing a Monitor navigation, apply the final checkpoint, then rebuild a fresh Analyze render from the same Workflow and assert final progress. The test proves route components do not own analysis progress without requiring native click automation.

Run:

```bash
cargo fmt --all --check
cargo test -p app views::analyze -- --nocapture
cargo test -p app --test analysis_workflow -- --nocapture
cargo test -p app --test render_workflow -- --nocapture
just css
```

Add this deterministic recipe:

```just
css:
    nix develop --command tailwindcss -i app/tailwind.css -o app/assets/tailwind.css
```

Expected: Analyze renders all required states, route navigation does not own/cancel progress, and generated CSS includes every Monitor/Analyze class. Do not hand-edit generated CSS.

---

### Task 12: Paid Gate, Documentation, And End-To-End Verification

**Files:**
- Create: `app/tests/paid_openai_workflow.rs`
- Modify: `app/Cargo.toml`
- Modify: `backend/Cargo.toml`
- Modify: `backend/src/analysis/analyzer/analyzer.rs`
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `app/README.md`
- Modify: `docs/architecture.md`
- Modify: `justfile`
- Test: `app/tests/operator_session_flow.rs`
- Test: `app/tests/analysis_workflow.rs`

**Interfaces:**
- Produces: safe paid-test policy, accurate docs/recipes, complete automated evidence, and a manual desktop acceptance record.

- [ ] **Step 1: Move the paid test behind three gates**

Compile it only with:

```rust
#![cfg(feature = "paid-openai-test")]

#[tokio::test]
#[ignore = "costs money; requires explicit approval and LEO_RUN_PAID_OPENAI_TEST=1"]
```

Apply those crate/function attributes to `paid_openai_analyzes_one_application_session`. Add app feature `paid-openai-test = []`. The test's first executable statement is `assert_eq!(std::env::var("LEO_RUN_PAID_OPENAI_TEST").as_deref(), Ok("1"), "paid test requires explicit LEO_RUN_PAID_OPENAI_TEST=1 approval");`, before provider construction. Delete the old Analyzer paid test and remove the now-unused backend feature.

The test starts a tiny local Axum route implementing only Recording List v5 and Download v6. List projects one camera-1 recording from the request `fromTime` for two seconds; Download returns `camera/fixtures/salon-1.mp4`. It configures camera 1 enabled and camera 2 disabled, creates and stops one one-second Workflow session in a temp root, calls `begin_analysis("Student completes the visible movement")`, passes that request to public `backend::analysis::analyze_session`, and applies every callback to the same Workflow. Assert callbacks have response counts `[0, 1]`, the matching running ID clears, one provider response is visible through the stored checkpoint, `analysis.json` reloads, and no MP4/JPEG remains in the session directory. Keep Axum as an app dev dependency for this test; do not add another mock-server crate.

- [ ] **Step 2: Document current architecture and local setup**

Update docs for:

- `backend` library boundary and future CLI reuse;
- continuous recording vs software session actions;
- camera config file;
- two virtual-camera ports;
- query-aligned finite simulator catalogue;
- first-gap truncation warnings;
- session discovery and checkpoint v2;
- root-scoped background analysis;
- logs/session directories;
- exact paid-test prohibition.

- [ ] **Step 3: Run all normal verification**

Run:

```bash
cargo fmt --all --check
cargo test -p backend
cargo test -p synology
cargo test -p app
cargo test --workspace --all-targets
cargo test --workspace --all-targets --all-features --no-run
cargo clippy -p backend --all-targets --all-features
cargo clippy -p synology --all-targets --all-features -- -D warnings
cargo clippy -p app --all-targets --all-features
git diff --check
```

Expected: all normal tests pass; the all-features command compiles but runs nothing; paid code therefore cannot make a request. Clippy exits successfully. Fix new warnings; do not refactor unrelated existing warning policy.

- [ ] **Step 4: Run exact non-paid media checks**

Run only:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
nix develop --command cargo test -p backend analysis::facade::tests::full_http_ffmpeg_and_model_pipeline_uses_the_existing_fixture -- --ignored --exact
nix develop --command cargo test -p synology api::recording::tests::downloads_partial_recording -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
```

Do not set the paid runtime gate and do not run the paid test. If exact names differ after moving modules, use the full name reported by `cargo test -- --list`; never replace these commands with blanket ignored execution.

- [ ] **Step 5: Perform the local desktop acceptance**

Start `just camera-1`, `just camera-2`, `just synology`, and `just app` in separate terminals. Execute steps 1-8 and 11-12 from `docs/superpowers/specs/2026-08-09-session-analysis-ui-design.md` without a provider call: previews, session actions, durable discovery, checklist editing, explicit-action availability, restart discovery, logs, and absence of temporary media. Progress/results steps 9-10 run only with an explicitly approved paid or local compatible endpoint.

- [ ] **Step 6: Run the paid acceptance only after explicit approval**

After the user authorizes the cost, source the existing environment and run exactly:

```bash
nix develop --command env LEO_RUN_PAID_OPENAI_TEST=1 cargo test -p app --features paid-openai-test --test paid_openai_workflow paid_openai_analyzes_one_application_session -- --ignored --exact --nocapture
```

Record model name, request count, duration, and result; never print credentials.

- [ ] **Step 7: Report implementation**

Report changed behavior, exact tests and acceptance checks, paid-test status, physical NAS checks not run, known finite-fixture/preview-phase limits, deferred work, and implementation difficulties.
