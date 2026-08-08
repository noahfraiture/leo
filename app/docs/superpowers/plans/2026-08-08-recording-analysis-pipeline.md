# Recording-to-Analysis Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert continuously recorded Synology video and append-only operator events into deterministic, resumable multimodal exercise analysis without storing extracted images permanently.

**Architecture:** Synology records every camera continuously. A session JSONL records software-only camera participation and sampling changes. Replaying that file produces one `SamplingSchedule` per camera; matching Synology catalogue entries become `Video` values, schedules produce `SampleSequence`s, and those sequences merge into canonical `FrameSet`s. `Analyzer` downloads only the next batch's required recording windows, extracts temporary JPEGs through `ffmpeg-sidecar`, builds a Rig message, delegates the model call to a transport-only `Agent`, and checkpoints the structured response.

**Tech Stack:** Rust 2024, Tokio, Reqwest, Serde/JSONL, UUID, `thiserror`, `tempfile`, `ffmpeg-sidecar` 2.5.2, FFmpeg 8, Rig 0.41, Synology Surveillance Station Recording API v6.

## Full-Solution Integration

```text
Synology continuously records every camera
                    +
Operator actions -> session events.jsonl
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

The future operator UI will create the session log, append actions, finish the session, and launch analysis. This plan implements the backend pipeline only; it does not wire Dioxus controls.

## Global Constraints

- Cameras continue recording regardless of software camera participation events.
- Camera enable and disable events affect sampling and analysis only.
- The JSONL event log is the durable source of session metadata.
- Store UTC milliseconds and session-relative milliseconds in every event.
- Generate sampling positions from integer millisecond intervals; do not persist floating-point FPS.
- Preserve `Video`, `SamplingRateChange`, `SamplingSchedule`, `SampleSequence`, `Frame`, and `FrameSet` as explicit domain stages.
- Use session-relative `Duration` for runtime scheduling and grouping; do not persist or rebuild `Instant` values.
- Keep analysis independent from `preview::CameraSource` and RTSP URLs; join domains with stable Synology camera IDs.
- A scheduled sample without exactly one matching recording is an error for now.
- Use `Recording.List` and `Recording.Download`; do not scan undocumented Surveillance Station directories.
- Use the primary documented Recording API v6 response. Add compatibility only after observing a different target-NAS response.
- Authentication is optional client plumbing: unauthenticated requests omit `_sid`; `login` stores one SID when the NAS requires it. Do not add refresh, logout, role, credential-storage, or gateway machinery.
> Everything will run on a controlled and closed environment, security is not important and we should stay as simple as possible.
- Download only recording windows required by the current analysis batch.
- Use `ffmpeg-sidecar` with default features disabled so it uses FFmpeg from the Nix environment.
> Eventually I will have to compile the app and install it on other laptop, it would be nice to have ffmpeg with it so I don't have to install it in the path of the other app. If it's possible, we can build the app with nix to easily install everything we need in the path, if not we might want to have another solution ? 
- Write JPEGs to a temporary directory; do not parse MJPEG pipes or add a JPEG encoder.
- Keep model-provider details inside `Agent`; keep domain orchestration inside `Analyzer`.
- Do not introduce `PromptFrame`, `PromptFrameSet`, `AnalysisBatch`, `AnalysisRequest`, or an extraction trait.
> What does that mean ?
- Preserve existing atomic checkpoint replacement and rollback after a failed save.
- Checkpoint plan/checklist fingerprinting remains deferred.
- Keep all new APIs `pub(crate)` or narrower and keep `mod.rs` files declaration/export-only.
- Put non-trivial module errors in that module's `error.rs` and derive them with `thiserror`.
- Do not implement operator UI, active-session restart recovery, NAS-side extraction, direct NAS filesystem access, media caching, bookmarks, digital zoom, or face blurring.
- Do not modify the unrelated worktree changes in `AGENTS.md`, `app/assets/tailwind.css`, or `ratio.md`.
- Do not commit this implementation plan.

> I would like to include the checkpoint and resume system in this plan

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
struct SessionEvent {
    schema_version: u8,
    sequence: u64,
    session_id: Uuid,
    utc_ms: i64,
    session_offset_ms: u64,
    action: SessionAction,
}

#[serde(tag = "type", rename_all = "snake_case")]
enum SessionAction {
    SessionStarted {
        cameras: Vec<CameraSnapshot>,
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

pub(crate) struct CameraSnapshot {
    pub camera_id: u32,
    pub name: String,
    pub enabled: bool,
    pub sample_every_ms: u64,
}
```

Representative file:

```jsonl
{"schema_version":1,"sequence":0,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204800000,"session_offset_ms":0,"action":{"type":"session_started","cameras":[{"camera_id":1,"name":"Front","enabled":true,"sample_every_ms":5000},{"camera_id":2,"name":"Side","enabled":true,"sample_every_ms":2000}]}}
{"schema_version":1,"sequence":1,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204810000,"session_offset_ms":10000,"action":{"type":"camera_participation_changed","camera_id":2,"enabled":false}}
{"schema_version":1,"sequence":2,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204815000,"session_offset_ms":15000,"action":{"type":"sampling_interval_changed","camera_id":1,"sample_every_ms":1000}}
{"schema_version":1,"sequence":3,"session_id":"5a660250-36fc-4c2b-93fa-b04247bdad20","utc_ms":1786204830000,"session_offset_ms":30000,"action":{"type":"session_ended"}}
```

### Runtime Video Domain

```rust
pub(crate) struct Video {
    pub recording_id: u64,
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
}

pub(crate) struct SamplingRateChange {
    pub offset: Duration,
    pub sample_every: Duration,
}

enum SamplingChange {
    Participation {
        offset: Duration,
        enabled: bool,
    },
    Rate(SamplingRateChange),
}

> why do we have this in addition to SamplingRateChange ? Is it work to have an enum here ?
> We can change the existing interface it allows for a clean domain here
> When implementing, add one or two line per element of the domain of small doc explaining what it represents

pub(crate) struct SamplingSchedule {
    pub camera_id: u32,
    initial_enabled: bool,
    initial_sample_every: Duration,
    changes: Vec<SamplingChange>,
}

pub(crate) struct Frame {
    pub camera_id: u32,
    pub recording_id: u64,
    pub sample_index: usize,
    pub session_offset: Duration,
    pub recording_offset: Duration,
}

pub(crate) struct SampleSequence {
    pub camera_id: u32,
    pub frames: Vec<Frame>,
}

pub(crate) struct FrameSet {
    pub session_offset: Duration,
    pub frames: Vec<Frame>,
}
```

JPEG bytes never enter these structs. They exist only while constructing the current Rig message.

---

### Task 1: Session JSONL Storage and Replay

**Files:**
- Create: `app/src/session/mod.rs`
- Create: `app/src/session/session.rs`
- Create: `app/src/session/error.rs`
- Modify: `app/src/main.rs`
- Modify: `app/Cargo.toml`

**Interfaces:**

```rust
impl SessionLog {
    pub(crate) fn create(
        path: PathBuf,
        cameras: Vec<CameraSnapshot>,
    ) -> Result<Self>;

    pub(crate) fn append(
        &mut self,
        action: OperatorAction,
    ) -> Result<()>;

    pub(crate) fn finish(&mut self) -> Result<()>;
}

impl Session {
    pub(crate) fn load(path: &Path) -> Result<Self>;
}
```

> Let's make this a bit stronger : eventually events will be able to send to the session logs but also send to the camera for some of them (change the pan/zoom of the camera for example). I think we should have a single interface for this that can be used from the UI and the internal, some kind of controller, will decide to either send it to logs, camera, synology of mutliple and handle the actual logic. So this is this elements that will expose the interface to communicate with the hardware part of the system. If you disagree or think we can do better, say it to me. This can require a bit of renaming of change of interface

> Another agents running on that question made this output, it's not exactly up to date or correct but this can source of inspiration
```
Action Boundary
Use a backend SessionController, not a load balancer or generic event engine.
Frontend
  -> OperatorAction
  -> SessionController::apply()
       -> EventLog
       -> AxisClient
       -> SynologyClient
The frontend never writes JSONL or calls a camera directly.
enum OperatorAction {
    SetCameraParticipation {
        camera_id: u32,
        enabled: bool,
    },
    SetSamplingInterval {
        camera_id: u32,
        sample_every_ms: u64,
    },
    MoveCameraRelative {
        camera_id: u32,
        pan_degrees: Option<f64>,
        tilt_degrees: Option<f64>,
        zoom_steps: Option<i32>,
    },
}
SessionController::apply(&mut self, action) uses one explicit match. The mutable controller serializes actions and JSONL sequence numbers without an event bus.
Routing Rules
Action	Destination	Persisted events
Camera participation	JSONL only	CameraParticipationChanged
Sampling interval	JSONL only	SamplingIntervalChanged
Relative PTZ/zoom	Axis camera and JSONL	Requested, then accepted or failed
Hardware flow:
1. Validate the action.
2. Append CameraCommandRequested.
3. Send the VAPIX command.
4. Append CameraCommandAccepted or CameraCommandFailed.
5. Correlate the outcome with the request event’s sequence number.
If the application stops after step 2, replay reports an unresolved command but does not assume it was applied.
“Accepted” is more accurate than “Succeeded”: Axis returning success means the command was accepted, not that physical movement finished.
Official PTZ Contract
Use relative commands because they match both Axis VAPIX and the existing simulator:
- rpan: -360.0..=360.0 degrees
- rtilt: -360.0..=360.0 degrees
- rzoom: -9999..=9999 zoom steps
Do not support absolute PTZ yet.
The client must recognize:
- 204 No Content as accepted.
- 200 OK with a body beginning Error: as failure.
- Transport ambiguity as failure without automatic retry, because relative movements are not idempotent.
The virtual camera needs only two small changes:
- Accept and validate rzoom.
- Advertise rpan, rtilt, and rzoom from info=1.
No position tracking, automatic retries, movement-completion polling, generic VAPIX passthrough, or frontend routing logic is needed now.
This becomes a new phase between JSONL storage and schedule replay. The later UI will receive a handle to SessionController; how Dioxus delivers commands to that handle is UI wiring, not domain routing.
```

- [ ] **Step 1: Add the UUID dependency and module skeleton**

Add to `app/app/Cargo.toml`:

```toml
uuid = { version = "1.24.0", features = ["serde", "v4"] }
```

Declare `mod session;` in `app/src/main.rs`. Keep `session/mod.rs` limited to submodule declarations and narrow reexports.

- [ ] **Step 2: Write failing serialization and replay tests**

Add tests for the representative event schema, contiguous appends, file reopen, malformed JSON, unsupported schema versions, mismatched session IDs, non-contiguous sequences, duplicate initial cameras, zero intervals, unknown-camera actions, actions after session end, missing session end, and duplicate session end.

`SessionLog::create` must write sequence zero immediately. Each append must serialize one line, append `\n`, flush, and call `sync_data`.

- [ ] **Step 3: Run the focused tests and verify red**

Run:

```bash
cargo test -p app session:: -- --nocapture
```

Expected: compilation or assertions fail because the types and validation are not implemented.

- [ ] **Step 4: Implement the minimum writer and loader**

Generate UTC milliseconds from `SystemTime`. Keep an internal `Instant` in `SessionLog` and derive every runtime session offset from its elapsed duration. Generate one UUID at session creation and reuse it in every event.

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

    pub(crate) fn sample_offsets(
        &self,
        session_end: Duration,
    ) -> Result<Vec<Duration>>;
}

impl SampleSequence {
    pub(crate) fn from_videos(
        session_start_utc_ms: i64,
        session_end: Duration,
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
- Generated offsets are ordered and unique.

- [ ] **Step 2: Replace invalid fields without deleting the domain stages**

Replace process-local `Instant` timestamps with session-relative `Duration`. Replace `CameraSource` with `camera_id`. Replace integer FPS with `sample_every: Duration`. Replace the unfinished `Video::sample` ownership model with `SampleSequence::from_videos`, because one camera schedule may span multiple Synology recording files.

Keep the existing struct names and their pipeline responsibilities.

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

    > Is the login required in our system ? I told that we should ignore most of the login system when possible to simplify the system

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

Verify requests omit `_sid` before login and include the SID returned by `SYNO.API.Auth` afterward. The password must not remain in `SynologyClient`.

- [ ] **Step 5: Implement one optional SID login**

Do not add automatic login, retries, refresh, logout, cookies, role checks, or credential persistence. The caller decides whether the target NAS requires `login`.

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

> I don't see the model of the checkpoint file, what's the idea to actually make checkpoints ?

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

Reject an empty generated plan. Continue to validate only batch count and contiguous completed indices; do not add plan fingerprints in this task.

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

Repeat FrameSets chronologically and Frames by camera ID. Do not expose recording IDs to the model unless required for diagnostics.

- [ ] **Step 8: Preserve failure and checkpoint semantics**

Download or extraction failure must occur before the model call. Model failure must not modify progress. After a successful model response, append the completed batch, atomically save, and pop it again if saving fails.

Keep the current pretty JSON checkpoint format and its atomic `NamedTempFile` replacement. Plan/checklist fingerprinting remains explicitly deferred.

- [ ] **Step 9: Add focused Analyzer tests**

Cover prompt ordering, previous response, batch ranges, download failure before model invocation, model failure without progress, failed checkpoint rollback, complete analysis, and resume from the first incomplete batch.

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

Keep module declarations and narrow reexports in each `mod.rs`. Use `pub(crate)` only for cross-module contracts and `pub(super)` for implementation details. Add one- or two-sentence documentation to the primary structs and entry-point methods.

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
- Bookmark, note, digital zoom, and face-blurring event semantics.
- Direct access to Surveillance Station's internal files.
- Server-side Synology range exports.
- NAS-side FFmpeg or a frame-extraction service.
- Cross-batch video caching.
- Parallel FFmpeg extraction.
- Model-specific image limits or dynamic image-count batching.
- Checkpoint fingerprints for events, recordings, checklist, or batch boundaries.
- Source decoder frame indices and exact decoded PTS diagnostics.
