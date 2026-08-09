# Recording-to-Analysis Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert continuously recorded Synology video and append-only operator events into deterministic, resumable multimodal exercise analysis without storing extracted images permanently.

**Architecture:** Synology records every camera continuously. All operator and internal actions enter through a concrete `SessionController`, which validates and records current software-only participation and sampling actions in session JSONL. Replaying that file produces one normalized `SamplingSchedule` per camera; matching Synology catalogue entries become `Video` values, schedules produce `SampleSequence`s, and those sequences merge into canonical `FrameSet`s. `Analyzer` downloads only the next batch's required recording windows, extracts temporary JPEGs through `ffmpeg-sidecar`, builds a Rig message directly from canonical frames, delegates the model call to a transport-only `Agent`, and atomically checkpoints the structured response.

**Tech Stack:** Rust 2024, Tokio, Reqwest, Serde/JSONL, UUID, `thiserror`, `tempfile`, `ffmpeg-sidecar` 2.5.2, FFmpeg 8, Rig 0.41, Synology Surveillance Station Recording API v6.

## Full-Solution Integration

```text
Synology continuously records every camera
                    +
Operator actions -> SessionController -> events.jsonl
                    |
                    v
       SamplingSchedule per camera
                    +
          Synology Video catalogue
                    |
                    v
        SampleSequence per camera
                    |
                    v
          Ordered canonical FrameSets
                    |
                    v
 Temporary video windows and JPEG extraction
                    |
                    v
      Analyzer -> Agent -> AnalysisResponse
                    |
                    v
            Durable checkpoint
```

The future operator UI will use `SessionController` to create a session, apply actions, finish the session, and launch analysis. The UI will never write JSONL or call hardware directly. This plan implements the backend pipeline only; it does not wire Dioxus controls or hardware commands.

## Global Constraints

- Cameras continue recording regardless of software camera participation events.
- Camera enable and disable events affect sampling and analysis only.
- The JSONL event log is the durable source of session metadata.
- Store UTC milliseconds and session-relative milliseconds in every event.
- Generate sampling positions from integer millisecond intervals; do not persist floating-point FPS.
- Preserve `Video`, `SamplingSchedule`, `SamplingPeriod`, `SampleSequence`, `Frame`, and `FrameSet` as explicit domain stages.
- Use session-relative `Duration` for replayed scheduling and grouping; never persist `Instant`. The active `SessionController` may use one process-local `Instant` only to measure event offsets while it is running.
- Keep analysis independent from `preview::CameraSource` and RTSP URLs; join domains with stable Synology camera IDs.
- A scheduled sample without exactly one matching recording is an error for now.
- Use `Recording.List` and `Recording.Download`; do not scan undocumented Surveillance Station directories.
- Use the primary documented Recording API v6 response. Add compatibility only after observing a different target-NAS response.
- Everything runs on a controlled, isolated network. Treat Synology login only as protocol plumbing: unauthenticated requests omit `_sid`, and one explicit `login` stores a SID because supported Recording APIs require it. Do not add refresh, logout, role, credential-storage, gateway, or other security machinery.
- Download only recording windows required by the current analysis batch.
- Use `ffmpeg-sidecar` with default features disabled. Resolve FFmpeg from `PATH` or beside the executable without hardcoding a Nix-store path, so a later macOS or Windows package can supply the binary without changing analysis code.
- Defer packaged-app delivery. Development and verification use FFmpeg from the current Apple Silicon Nix shell.
- Write JPEGs to a temporary directory; do not parse MJPEG pipes or add a JPEG encoder.
- Keep model-provider details inside `Agent`; keep domain orchestration inside `Analyzer`.
- Construct each Rig message directly while iterating canonical `FrameSet`/`Frame` metadata. Extract a JPEG into a local value, append it to the message, and drop it; do not copy frames into `PromptFrame`, `PromptFrameSet`, `AnalysisBatch`, or `AnalysisRequest` adapter structs.
- Call the concrete FFmpeg extraction function directly; do not add a trait with only one implementation.
- Include the checkpoint and resume system in this plan. Preserve atomic checkpoint replacement and rollback after a failed save.
- Checkpoint plan/checklist fingerprinting remains deferred.
- Keep all new APIs `pub(crate)` or narrower and keep `mod.rs` files declaration/export-only.
- Put non-trivial module errors in that module's `error.rs` and derive them with `thiserror`.
- Do not implement operator UI, active-session restart recovery, hardware commands/PTZ, NAS-side extraction, direct NAS filesystem access, media caching, bookmarks, digital zoom, or face blurring.
- Do not modify the unrelated worktree changes in `AGENTS.md`, `app/assets/tailwind.css`, or `ratio.md`.
- Do not commit this implementation plan.

## Locked File Structure

```text
app/
|-- Cargo.toml
|-- docs/
|   \-- architecture.md
\-- app/
    |-- Cargo.toml
    \-- src/
        |-- main.rs
        |-- session/
        |   |-- mod.rs
        |   |-- controller.rs
        |   |-- session.rs
        |   \-- error.rs
        |-- recording/
        |   |-- mod.rs
        |   |-- synology.rs
        |   \-- error.rs
        \-- analysis/
            |-- mod.rs
            |-- agent/
            |   |-- mod.rs
            |   |-- agent.rs
            |   \-- error.rs
            |-- analyzer/
            |   |-- mod.rs
            |   |-- analyzer.rs
            |   |-- progress.rs
            |   \-- error.rs
            \-- video/
                |-- mod.rs
                |-- video.rs
                |-- extractor.rs
                \-- error.rs
```

Delete `app/src/analysis/runner/` only after its files have moved to `analysis/analyzer/` and the mechanically renamed tests pass.

## Data Contracts

### Persisted Session Event

```rust
/// One durably ordered fact on a session's monotonic and UTC timelines.
pub(crate) struct SessionEvent {
    schema_version: u8,
    sequence: u64,
    session_id: Uuid,
    utc_ms: i64,
    session_offset_ms: u64,
    action: SessionAction,
}

/// The persisted effect of one accepted controller action.
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum SessionAction {
    SessionStarted {
        cameras: Vec<SessionCamera>,
    },
    CameraParticipationChanged {
        camera_id: u32,
        enabled: bool,
    },
    SamplingIntervalChanged {
        camera_id: u32,
        sample_every_ms: u64,
    },
    SessionEnded,
}

/// One camera's stable identity and initial software state for this session.
pub(crate) struct SessionCamera {
    #[serde(rename = "camera_id")]
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "sample_every_ms", with = "duration_millis")]
    pub sample_every: Duration,
}

/// A validated completed session reconstructed from its JSONL events.
pub(crate) struct Session {
    pub id: Uuid,
    pub start_utc_ms: i64,
    pub end_offset: Duration,
    pub cameras: Vec<SessionCamera>,
    pub events: Vec<SessionEvent>,
}
```

`SessionCamera` is the controller-facing domain value, not a persistence DTO or image snapshot. Its private `duration_millis` Serde helper writes the existing `sample_every_ms` JSON number and reconstructs `Duration` on load; no second camera representation is introduced.

Representative file:

```jsonl
{"schema_version":1,"sequence":0,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204800000,"session_offset_ms":0,"action":{"type":"session_started","cameras":[{"camera_id":1,"name":"Front","enabled":true,"sample_every_ms":5000},{"camera_id":2,"name":"Side","enabled":true,"sample_every_ms":2000}]}}
{"schema_version":1,"sequence":1,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204810000,"session_offset_ms":10000,"action":{"type":"camera_participation_changed","camera_id":2,"enabled":false}}
{"schema_version":1,"sequence":2,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204815000,"session_offset_ms":15000,"action":{"type":"sampling_interval_changed","camera_id":1,"sample_every_ms":1000}}
{"schema_version":1,"sequence":3,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204830000,"session_offset_ms":30000,"action":{"type":"session_ended"}}
```

### Runtime Video Domain

```rust
/// One retained Synology recording segment on the shared UTC timeline.
pub(crate) struct Video {
    pub recording_id: u64,
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
}

/// One enabled interval with a stable sampling cadence.
pub(crate) struct SamplingPeriod {
    pub start: Duration,
    pub end: Duration,
    pub sample_every: Duration,
}

/// The normalized enabled periods for one camera over a completed session.
pub(crate) struct SamplingSchedule {
    pub camera_id: u32,
    pub periods: Vec<SamplingPeriod>,
}

/// Metadata locating one scheduled sample in one retained recording.
pub(crate) struct Frame {
    pub camera_id: u32,
    pub recording_id: u64,
    pub sample_index: usize,
    pub session_offset: Duration,
    pub recording_offset: Duration,
}

/// The chronologically ordered samples selected for one camera.
pub(crate) struct SampleSequence {
    pub camera_id: u32,
    pub frames: Vec<Frame>,
}

/// Samples from all participating cameras at one scheduled session offset.
pub(crate) struct FrameSet {
    pub session_offset: Duration,
    pub frames: Vec<Frame>,
}
```

JSONL retains the original participation and interval changes. Replay resolves them into enabled `SamplingPeriod`s, so the runtime sampling domain does not duplicate the event log with another change enum. Disabled time is represented by a gap between periods, and a participation or interval change begins a new period when the camera is enabled.

JPEG bytes never enter these structs. `Analyzer` extracts one JPEG into a local value while iterating a `Frame`, appends it to the current Rig message, and then drops it.

### Analysis Checkpoint

```rust
/// Durable model responses for the completed prefix of one session's batch plan.
#[derive(Serialize, Deserialize)]
pub(crate) struct AnalysisCheckpoint {
    pub schema_version: u8,
    pub session_id: Uuid,
    pub total_batches: usize,
    pub completed_batches: Vec<CompletedBatch>,
}

/// One successfully analyzed batch and the response carried into the next batch.
#[derive(Serialize, Deserialize)]
pub(crate) struct CompletedBatch {
    pub index: usize,
    pub response: AnalysisResponse,
}
```

Representative `analysis.json` beside `events.jsonl`:

```json
{
  "schema_version": 1,
  "session_id": "5a660250-36fc-4c2b-93fa-b04247bdad20",
  "total_batches": 3,
  "completed_batches": [
    {
      "index": 0,
      "response": {
        "observations": [],
        "sequence_summary": "The exercise has started.",
        "checklist_progress": []
      }
    }
  ]
}
```

After each successful model response, `Analyzer` serializes the complete checkpoint to a temporary file in the same directory, appends a newline, flushes and syncs it, then atomically replaces `analysis.json`. Resume rebuilds the media plan from JSONL and Synology metadata, validates checkpoint schema/session/batch count/contiguous indices, skips the completed prefix, and carries the last saved response into the next prompt. Videos and JPEGs are regenerated and never stored in the checkpoint.

---

### Task 1: Session Controller, JSONL Storage, and Replay

**Files:**
- Create: `app/src/session/mod.rs`
- Create: `app/src/session/controller.rs`
- Create: `app/src/session/session.rs`
- Create: `app/src/session/error.rs`
- Modify: `app/src/main.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**

```rust
/// Actions accepted from future UI and internal callers.
pub(crate) enum OperatorAction {
    SetCameraParticipation {
        camera_id: u32,
        enabled: bool,
    },
    SetSamplingInterval {
        camera_id: u32,
        sample_every: Duration,
    },
    EndSession,
}

/// The single backend entry point that validates, routes, and serializes session actions.
pub(crate) struct SessionController {
    log: SessionLog,
}

impl SessionController {
    pub(crate) fn create(
        events_path: PathBuf,
        cameras: Vec<SessionCamera>,
    ) -> Result<Self>;

    pub(crate) fn apply(
        &mut self,
        action: OperatorAction,
    ) -> Result<()>;
}

impl Session {
    pub(crate) fn load(events_path: &Path) -> Result<Self>;
}
```

`events_path` names the session's `events.jsonl`; later analysis stores `analysis.json` in the same directory. `SessionController::apply` uses one explicit match and owns the private `SessionLog`, which serializes action handling and JSONL sequence assignment without an event bus. Current participation and interval actions validate their payload and append the matching event. `EndSession` appends `SessionEnded` and rejects later actions.

Future PTZ or Synology-control actions will be added as explicit `OperatorAction` variants and routed from this same match, with request/outcome events where hardware ambiguity matters. This plan does not add unused hardware clients or generic routing abstractions before those actions are implemented.

- [ ] **Step 1: Add the UUID dependency and module skeleton**

Add to `app/app/Cargo.toml`:

```toml
uuid = { version = "1.24.0", features = ["serde", "v4"] }
```

Declare `mod session;` in `app/src/main.rs`. Keep `session/mod.rs` limited to submodule declarations and narrow reexports. Reexport `SessionController`, `OperatorAction`, `SessionCamera`, and the completed `Session`; keep `SessionLog` private to the module.

- [ ] **Step 2: Write failing serialization and replay tests**

Add tests for the representative event schema, `SessionCamera` duration-to-milliseconds serialization, controller action routing, contiguous appends, file reopen, malformed JSON, unsupported schema versions, mismatched session IDs, non-contiguous sequences, duplicate initial cameras, zero intervals, unknown-camera actions, actions after session end, missing session end, and duplicate session end.

`SessionController::create` must create its private log and write sequence zero immediately. Each applied action must serialize one line, append `\n`, flush, and call `sync_data`.

- [ ] **Step 3: Run the focused tests and verify red**

Run:

```bash
cargo test -p app session:: -- --nocapture
```

Expected: compilation or assertions fail because the types and validation are not implemented.

- [ ] **Step 4: Implement the minimum writer and loader**

Implement `SessionController::apply` with one explicit match and no generic dispatcher. Validate `Duration` values at the controller boundary and convert them to persisted milliseconds only during serialization. Generate UTC milliseconds from `SystemTime`. Keep an internal `Instant` in the private `SessionLog` and derive every runtime session offset from its elapsed duration. Generate one UUID at session creation and reuse it in every event.

`Session::load` treats the first event's UTC time as the session UTC anchor and the `SessionEnded` offset as the exclusive session end. Later analysis uses `start_utc_ms + session_offset_ms` for deterministic recording alignment; per-event UTC remains audit metadata.

Do not add file locking, active-session reopening, generic event payloads, or recovery of a partially written final line.

- [ ] **Step 5: Run focused tests and format**

Run:

```bash
cargo fmt --all --check
cargo test -p app session:: -- --nocapture
```

Expected: all session tests pass.

---

### Task 2: Sampling, Videos, Sequences, and FrameSets

**Files:**
- Modify: `app/src/analysis/video/video.rs`
- Modify: `app/src/analysis/video/error.rs`
- Modify: `app/src/analysis/video/mod.rs`
- Modify: `app/src/analysis/mod.rs`

**Interfaces:**

```rust
impl SamplingSchedule {
    pub(crate) fn from_session(
        session: &Session,
        camera_id: u32,
    ) -> Result<Self>;

    pub(crate) fn sample_offsets(&self) -> Result<Vec<Duration>>;
}

impl SampleSequence {
    pub(crate) fn from_videos(
        session_start_utc_ms: i64,
        schedule: &SamplingSchedule,
        videos: &[Video],
    ) -> Result<Self>;
}

impl FrameSet {
    pub(crate) fn from_sequences(
        sequences: Vec<SampleSequence>,
    ) -> Result<Vec<Self>>;
}
```

- [ ] **Step 1: Write failing schedule tests**

Cover these exact rules:

- Initially enabled cameras sample at offset zero.
- Initially disabled cameras do not sample until enabled.
- Disabling at an offset removes the sample at that offset.
- Enabling samples immediately and starts a new cadence.
- Changing the interval samples immediately and starts a new cadence.
- Same-offset events are all applied before deciding whether to sample that offset.
- Repeating the current enabled state or interval is a no-op and does not reset cadence.
- Session end is exclusive.
- Every normalized period has `start < end` and a non-zero interval.
- Periods are ordered and do not overlap.
- Disabled time appears as a gap between periods.
- Generated offsets are ordered and unique.

- [ ] **Step 2: Replace invalid fields without deleting the domain stages**

Replace process-local `Instant` timestamps with session-relative `Duration`. Replace `CameraSource` with `camera_id`. Replace integer FPS with `sample_every: Duration`. Replay participation and interval events into normalized `SamplingPeriod`s instead of retaining another runtime change list. Replace the unfinished `Video::sample` ownership model with `SampleSequence::from_videos`, because one camera schedule may span multiple Synology recording files.

Keep `Video`, `SamplingSchedule`, `SampleSequence`, `Frame`, and `FrameSet` as the explicit pipeline stages; replace the old rate-change representation with the cleaner normalized `SamplingPeriod` domain.

Add concise one- or two-sentence documentation for `Video`, `SamplingPeriod`, `SamplingSchedule`, `Frame`, `SampleSequence`, and `FrameSet`, including ambiguous timestamp and offset fields.

- [ ] **Step 3: Write failing sequence and recording-coverage tests**

Use `start <= sample < end` recording boundaries. Verify correct recording offsets and global sample indices across two consecutive videos. Verify missing coverage and overlapping videos both fail.

- [ ] **Step 4: Implement sequence construction**

For each generated sample offset, calculate checked UTC milliseconds from the session anchor, require exactly one matching `Video`, and calculate the recording-relative offset. Do not decode video here.

- [ ] **Step 5: Write failing FrameSet merge tests**

Cover mixed camera intervals, partial sets, deterministic camera-ID order, unsorted input sequences, and duplicate camera frames at one offset.

- [ ] **Step 6: Implement and verify FrameSet merging**

Retain the existing peekable-iterator merge, but compare generated session offsets and return validation errors instead of relying on unchecked assumptions.

Run:

```bash
cargo fmt --all --check
cargo test -p app analysis::video:: -- --nocapture
```

Expected: all video-domain tests pass and the existing Agent/Runner tests still compile unchanged.

---

### Task 3: Synology Recording Catalogue and Download Client

**Files:**
- Create: `app/src/recording/mod.rs`
- Create: `app/src/recording/synology.rs`
- Create: `app/src/recording/error.rs`
- Modify: `app/src/main.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**

```rust
pub(crate) struct SynologyClient {
    http: reqwest::Client,
    base_url: reqwest::Url,
    sid: Option<String>,
}

impl SynologyClient {
    pub(crate) fn new(base_url: reqwest::Url) -> Self;

    pub(crate) async fn login(
        &mut self,
        account: &str,
        password: &str,
    ) -> Result<()>;

    pub(crate) async fn list_videos(
        &self,
        camera_ids: &[u32],
        from_utc_ms: i64,
        to_utc_ms: i64,
    ) -> Result<Vec<Video>>;

    pub(crate) async fn download(
        &self,
        video: &Video,
        range: Range<Duration>,
        destination: &Path,
    ) -> Result<()>;
}
```

- [ ] **Step 1: Add direct HTTP and runtime dependencies**

Add to `app/app/Cargo.toml`:

```toml
reqwest = { version = "0.13.4", features = ["json", "stream"] }
tokio = { workspace = true, features = ["fs", "io-util", "rt"] }
```

Add `axum = { workspace = true }` as an app dev-dependency for local HTTP tests. Merge the existing Tokio dev features rather than declaring conflicting entries.

- [ ] **Step 2: Write failing response and pagination tests**

Use a local Axum server. Cover a successful single page, multiple pages, deterministic sorting, malformed ranges, a Synology HTTP-200 JSON error, and the primary documented `recordings` response shape.

`list_videos` calls `SYNO.SurveillanceStation.Recording.List` version 6 with camera IDs and UTC second bounds. It converts returned seconds into integer milliseconds and follows `offset`/`limit` until all records are loaded.

- [ ] **Step 3: Implement the minimum catalogue client**

Hardcode the documented `/webapi/entry.cgi` path and version 6. Do not add API discovery or old response compatibility until a target appliance requires it.

- [ ] **Step 4: Write failing optional-login tests**

Verify requests omit `_sid` before login and include the SID returned by `SYNO.API.Auth` afterward. The password must not remain in `SynologyClient`. Supported physical Surveillance Station Recording APIs require this login; the optional unauthenticated path exists for the trusted simulator or an appliance proven to accept it.

- [ ] **Step 5: Implement one optional SID login**

Do not add automatic login, retries, refresh, logout, cookies, role checks, or credential persistence. The caller explicitly invokes the one login when required by the target NAS.

- [ ] **Step 6: Write failing streaming-download tests**

Cover MP4 bytes arriving in multiple chunks, destination write errors, non-success HTTP status, and JSON error responses. Assert no complete-video `Bytes` allocation is used.

- [ ] **Step 7: Implement `Recording.Download`**

Send recording `id`, `offsetTimeMs`, and `playTimeMs`. Stream successful media chunks into the supplied destination path. Treat JSON responses as Synology API errors.

Run:

```bash
cargo fmt --all --check
cargo test -p app recording:: -- --nocapture
```

Expected: all recording-client tests pass.

---

### Task 4: FFmpeg JPEG Extraction

**Files:**
- Create: `app/src/analysis/video/extractor.rs`
- Modify: `app/src/analysis/video/error.rs`
- Modify: `app/src/analysis/video/mod.rs`
- Modify: `app/Cargo.toml`

**Interface:**

```rust
pub(super) fn extract_jpeg(
    input: &Path,
    offset: Duration,
) -> Result<Vec<u8>>;
```

- [ ] **Step 1: Add `ffmpeg-sidecar` without download support**

Add to `app/app/Cargo.toml`:

```toml
ffmpeg-sidecar = { version = "=2.5.2", default-features = false }
```

Keep executable resolution portable: use `ffmpeg-sidecar`'s normal adjacent-executable/`PATH` lookup and do not embed a Nix-store path. The current Nix shell supplies FFmpeg for development. Building an installable app bundle with FFmpeg is a separate packaging task.

- [ ] **Step 2: Write the ignored fixture test first**

Use `camera/fixtures/default.mp4`, seek to `1000ms`, and assert the result starts with `ff d8 ff`, ends with `ff d9`, and is non-empty.

Run:

```bash
cargo test -p app extracts_fixture_frame_as_jpeg -- --ignored
```

Expected: compilation fails because extraction is not implemented.

- [ ] **Step 3: Implement the one-frame file-output command**

Construct the equivalent of:

```bash
ffmpeg -n -ss 1000ms -i input.mp4 -map 0:V:0 \
  -frames:v 1 -c:v mjpeg -f image2 frame.jpg
```

Use this builder shape:

```rust
FfmpegCommand::new()
    .no_overwrite()
    .seek(format!("{offset_ms}ms"))
    .arg("-i")
    .arg(input)
    .map("0:V:0")
    .frames(1)
    .codec_video("mjpeg")
    .format("image2")
    .arg(output)
    .spawn()?
    .wait()?;
```

Create a `TempDir`, use a path that does not yet exist, check the exit status, distinguish missing output, validate JPEG bytes, and let the directory clean itself up.

Do not pipe raw video or MJPEG, add a JPEG codec dependency, or download an FFmpeg binary.

- [ ] **Step 4: Verify normal and ignored tests**

Run:

```bash
cargo fmt --all --check
cargo test -p app analysis::video:: -- --nocapture
cargo test -p app extracts_fixture_frame_as_jpeg -- --ignored
```

Expected: all tests pass in the Nix development shell.

---

### Task 5: Mechanically Rename Runner to Analyzer

**Files:**
- Move: `app/src/analysis/runner/mod.rs` -> `app/src/analysis/analyzer/mod.rs`
- Move: `app/src/analysis/runner/runner.rs` -> `app/src/analysis/analyzer/analyzer.rs`
- Move: `app/src/analysis/runner/progress.rs` -> `app/src/analysis/analyzer/progress.rs`
- Move: `app/src/analysis/runner/error.rs` -> `app/src/analysis/analyzer/error.rs`
- Modify: `app/src/analysis/mod.rs`

- [ ] **Step 1: Rename only names and paths**

Rename `AnalysisRunner` to `Analyzer` and `RunnerError` to `AnalyzerError`. Keep the current `AnalysisBatch` API and all behavior temporarily. Do not mix the media integration into this mechanical step.

- [ ] **Step 2: Run the unchanged behavioral tests**

Run:

```bash
cargo fmt --all --check
cargo test -p app analysis::analyzer:: -- --nocapture
```

Expected: the existing checkpoint, resume, model-failure, and failed-save tests pass under their new module/type names.

---

### Task 6: Canonical Analyzer and Transport-Only Agent

**Files:**
- Modify: `app/src/analysis/agent/agent.rs`
- Modify: `app/src/analysis/agent/error.rs`
- Modify: `app/src/analysis/agent/mod.rs`
- Modify: `app/src/analysis/analyzer/analyzer.rs`
- Modify: `app/src/analysis/analyzer/error.rs`
- Modify: `app/src/analysis/analyzer/progress.rs`
- Modify: `app/src/analysis/analyzer/mod.rs`
- Modify: `app/src/analysis/mod.rs`

**Agent interface:**

```rust
impl<M: CompletionModel> Agent<M> {
    pub(crate) async fn analyze(
        &self,
        prompt: Message,
    ) -> Result<AnalysisResponse>;
}
```

**Analyzer interface:**

```rust
impl<M: CompletionModel> Analyzer<M> {
    pub(crate) async fn resume(
        agent: Agent<M>,
        synology: SynologyClient,
        session: Session,
        checklist: String,
        frame_sets_per_batch: NonZeroUsize,
        progress_path: PathBuf,
    ) -> Result<Self>;

    pub(crate) fn next_batch_index(&self) -> usize;

    pub(crate) async fn analyze_next(
        &mut self,
    ) -> Result<&AnalysisResponse>;
}
```

- [ ] **Step 1: Rewrite Agent tests around a prebuilt Rig message**

Keep schema, structured-response parsing, and missing-text tests. Replace request-building inputs with `Message::user(...)`. Move the previous-response and image-order test to Analyzer.

- [ ] **Step 2: Make Agent transport-only**

Keep model configuration, stable instructions, output schema, completion, and response parsing. Delete `PromptFrame`, `PromptFrameSet`, `AnalysisBatch`, and `AnalysisRequest` from Agent and remove their exports.

- [ ] **Step 3: Write failing Analyzer planning tests**

`Analyzer::resume` must select cameras with samples, list videos for the complete session UTC interval, build schedules and sequences, merge FrameSets, divide them with `chunks(frame_sets_per_batch)`, and validate the existing checkpoint against the resulting batch count.

Reject an empty generated plan. Rename the moved `AnalysisProgress` model to the documented `AnalysisCheckpoint`, add schema version, session ID, and total batch count, and validate them together with contiguous completed indices. Do not add event/checklist/plan fingerprints in this task.

- [ ] **Step 4: Implement deterministic planning in `resume`**

Store the session, videos, FrameSets, batch size, Synology client, Agent, checklist, progress path, and loaded progress in `Analyzer`. The generated media plan is reconstructed after every application restart; temporary files are not checkpointed.

- [ ] **Step 5: Write failing batch-window tests**

For every recording used by one batch, compute:

```text
download_start = minimum requested recording offset
download_end   = min(recording duration, maximum requested offset + 1 second)
```

Verify one recording is downloaded once per batch even when several frames reference it. Verify local extraction offsets subtract `download_start`.

- [ ] **Step 6: Implement temporary batch materialization**

Create one batch-scoped `TempDir`. Download every required recording range into it, then walk canonical FrameSets and Frames in order. Call `extract_jpeg` through `tokio::task::spawn_blocking` and append the resulting base64 JPEG immediately to the Rig message.

Add this deliberate-limit comment next to local media materialization:

```rust
// ponytail: each batch downloads its required video windows locally;
// move extraction onto the NAS if transfer becomes a bottleneck.
```

- [ ] **Step 7: Build the prompt without adapter DTOs**

Prompt content order is:

1. Correct sequence checklist.
2. Previous complete `AnalysisResponse`, or first-batch text.
3. FrameSet timestamp formatted as `HH:MM:SS.mmm`.
4. Stable camera ID/name and the same session timestamp.
5. Corresponding JPEG.

Repeat canonical FrameSets chronologically and Frames by camera ID. For each `Frame`, resolve its downloaded clip, call the concrete `extract_jpeg` function, append frame metadata and the returned JPEG directly to the Rig message, then drop the local bytes. Do not copy metadata/images into prompt-specific frame structs, add a one-implementation extractor trait, or expose recording IDs to the model unless required for diagnostics.

- [ ] **Step 8: Preserve failure and checkpoint semantics**

Download or extraction failure must occur before the model call. Model failure must not modify progress. After a successful model response, append the completed batch, atomically save, and pop it again if saving fails.

Store one `analysis.json` beside the session's `events.jsonl`. Serialize the complete `AnalysisCheckpoint` as pretty JSON through a `NamedTempFile` in the same directory, append a newline, flush and sync it, then atomically replace the checkpoint. On resume, rebuild videos, schedules, sequences, FrameSets, and batch boundaries; reject the wrong schema/session/batch count or non-contiguous indices; skip the completed prefix; and use the last saved response as context for the next model request. Temporary media is regenerated.

Plan/checklist fingerprinting remains explicitly deferred. The known residual risk is accepting changed event/checklist input that belongs to the same session and happens to produce the same number of batches.

- [ ] **Step 9: Add focused Analyzer tests**

Cover prompt ordering, previous response, batch ranges, download failure before model invocation, model failure without progress, failed checkpoint rollback, wrong checkpoint schema, wrong session ID, changed batch count, non-contiguous indices, complete analysis, and resume from the first incomplete batch.

Use local HTTP responses and Rig's `MockCompletionModel`. Do not add an extractor trait. Keep the complete HTTP + FFmpeg + model pipeline as an ignored integration test using the existing MP4 fixture.

- [ ] **Step 10: Run focused tests**

Run:

```bash
cargo fmt --all --check
cargo test -p app analysis::agent:: -- --nocapture
cargo test -p app analysis::analyzer:: -- --nocapture
```

Expected: all Agent and Analyzer unit tests pass.

---

### Task 7: Documentation, Exports, and End-to-End Verification

**Files:**
- Modify: `docs/architecture.md`
- Modify: `app/src/main.rs`
- Modify: `app/src/session/mod.rs`
- Modify: `app/src/recording/mod.rs`
- Modify: `app/src/analysis/mod.rs`
- Modify: `app/src/analysis/agent/mod.rs`
- Modify: `app/src/analysis/analyzer/mod.rs`
- Modify: `app/src/analysis/video/mod.rs`
- Modify: `app/Cargo.toml`

- [ ] **Step 1: Update architecture decisions**

Document continuous physical recording, software-only participation events, JSONL as session metadata, supported Recording API catalogue/download access, batch-local downloads, FFmpeg extraction, and the possible future NAS-side extractor.

Remove statements that the master session action starts or stops camera recording. Keep Synology responsible for storage and catalogue ownership.

- [ ] **Step 2: Tighten exports and documentation**

Keep module declarations and narrow reexports in each `mod.rs`. Use `pub(crate)` only for cross-module contracts and `pub(super)` for implementation details. Add one- or two-sentence documentation to every main domain element and ambiguous field, including session cameras/events/actions/controller, videos, sampling periods/schedules, frames/sequences/sets, extraction, Analyzer, Agent, checkpoint, completed batches, and their entry-point methods.

- [ ] **Step 3: Confirm obsolete prompt types are gone**

Run:

```bash
rg "PromptFrame|PromptFrameSet|AnalysisBatch|AnalysisRequest|AnalysisRunner" app/src
```

Expected: no matches.

- [ ] **Step 4: Run all automated verification**

Run:

```bash
cargo fmt --all --check
cargo test -p app
cargo test -p app extracts_fixture_frame_as_jpeg -- --ignored
cargo clippy -p app --all-targets --all-features
```

Expected: formatting passes, all normal tests pass, extraction passes in the Nix shell, and Clippy exits successfully. Existing dead-code warnings from backend APIs not yet wired to Dioxus are acceptable if no stronger warning policy is configured.

- [ ] **Step 5: Run one target-NAS acceptance check when hardware is available**

Use one short completed session range:

1. Call `Recording.List` for one camera.
2. Confirm the target response matches the documented `recordings` shape.
3. Download a short range.
4. Extract one JPEG.
5. Confirm temporary files disappear after the batch scope.

If the appliance returns the conflicting historical `events` shape, stop and update only the private response parser and its fixture. Do not add speculative support before observing it.


- [ ] **Step 6: Report implementation results**

Report changed behavior, tests run, target-NAS checks not run, known deferred limits, and any difficulty encountered. Do not commit unless explicitly requested.

## Explicitly Deferred

- Dioxus operator controls and status UI.
- Reopening and continuing an active session after application restart.
- Relative PTZ/zoom actions, Axis client routing, hardware request/outcome events, and simulator changes; these will extend `SessionController` in a focused follow-up.
- Bookmark, note, analysis-time digital cropping, and face-blurring event semantics.
- Packaged application delivery, Nix runtime wrappers, bundled FFmpeg/MediaMTX binaries, and Windows support. Extraction remains portable by avoiding hardcoded executable paths.
- Direct access to Surveillance Station's internal files.
- Server-side Synology range exports.
- NAS-side FFmpeg or a frame-extraction service.
- Cross-batch video caching.
- Parallel FFmpeg extraction.
- Model-specific image limits or dynamic image-count batching.
- Checkpoint fingerprints for events, recordings, checklist, or batch boundaries.
- Source decoder frame indices and exact decoded PTS diagnostics.
