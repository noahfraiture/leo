# Local Operator Workflow Design

Date: 2026-08-09

Status: Draft pending model-endpoint confirmation

## Goal

Turn the merged recording-analysis backend into a usable local desktop workflow backed by two virtual cameras, the Synology simulator, durable session files, and an explicitly triggered model analysis.

The complete flow is:

```text
virtual cameras and Synology are already running
                    |
                    v
app starts -> two live previews
                    |
                    v
operator starts session -> sessions/<UTC-ms>/events.jsonl
                    |
                    v
operator selects cameras, changes participation/cadence
                    |
                    v
operator stops session -> completed events.jsonl
                    |
                    v
Analyze lists completed sessions from disk
                    |
                    v
operator selects session, supplies checklist, starts analysis
                    |
                    v
root-owned background job -> analysis.json + visible progress/results
```

Physical and simulated recording remain independent from software sessions. Participation events affect only future frame selection. A camera that is excluded from analysis remains recording and remains visible in the Monitor preview.

## Existing Implementation To Reuse

The merged `698ce32` backend already implements and tests:

- durable `SessionController` JSONL writes and strict completed-session replay;
- normalized participation and cadence schedules;
- List v5 recording discovery and Download v6 bounded media retrieval;
- deterministic recording coverage, sample sequences, and frame sets;
- FFmpeg JPEG extraction into temporary directories;
- structured Rig/OpenAI requests and responses;
- deterministic batching, atomic `analysis.json` checkpoints, rollback, and resume;
- guarded ignored FFmpeg and paid OpenAI integration tests.

This design does not rebuild those features. It moves the reusable modules to a library crate, exposes a small public API, adds the missing storage and truncation semantics, then wires the API into Dioxus.

## Scope

- A reusable `backend` library crate containing session, recording, and analysis modules.
- Public/private visibility only in new and moved code; restricted `pub(...)` visibility is removed from those modules.
- Two configured cameras used consistently by preview, session events, Synology fixture IDs, and analysis prompts.
- Runtime camera configuration loaded from `cameras.json`.
- Monitor controls for session start/stop, camera selection, participation, and sampling interval.
- A direct, readable included/excluded indicator on each preview card.
- Durable discovery of every valid completed session under `./sessions`.
- Analyze session selection, recap, checklist, explicit start/resume, progress, warnings, and results.
- One background analysis job that survives route navigation.
- Finite fixture coverage that truncates at the first missing scheduled sample and persists a warning.
- Structured leveled logs written to a local JSONL log file and readable console output.
- Unit, integration, media, and explicitly gated paid OpenAI coverage.
- Minimal Tailwind CSS and daisyUI presentation without decorative polish.

## Explicit Non-Goals

- Starting or stopping physical recording.
- Active-session continuation after app restart.
- Running more than one analysis concurrently.
- Session deletion, renaming, pagination, or search.
- Camera discovery or a camera Settings UI.
- Video playback on Analyze.
- Sampling faster than whole-second intervals in the first UI.
- Analysis cancellation.
- Permanent downloaded clips or JPEGs.
- Physical Synology authentication UX and multi-DS/multi-mount reconciliation.
- Packaged FFmpeg/MediaMTX delivery and installers.
- A CLI executable in this increment. The library boundary enables one later.

## Workspace Architecture

### Reusable Backend Crate

Create a workspace library crate named `backend` and move these existing app modules without changing behavior first:

```text
backend/
|-- Cargo.toml
\-- src/
    |-- lib.rs
    |-- session/
    |   |-- mod.rs
    |   |-- controller.rs
    |   |-- session.rs
    |   |-- catalog.rs
    |   \-- error.rs
    |-- recording/
    |   |-- mod.rs
    |   |-- synology.rs
    |   |-- video.rs
    |   \-- error.rs
    \-- analysis/
        |-- mod.rs
        |-- facade.rs
        |-- error.rs
        |-- agent/
        |-- analyzer/
        \-- video/
```

`backend/src/lib.rs` exposes three modules:

```rust
pub mod analysis;
pub mod recording;
pub mod session;
```

The desktop app depends on `backend`. A future CLI can depend on the same crate without importing Dioxus, preview, or view code.

The extraction includes `app/src/session`, `app/src/recording`, and `app/src/analysis` together. Move `Video` from `analysis::video` to `recording` during the mechanical extraction, so recording owns the value it returns and analysis consumes it. This removes the current reciprocal module dependency without adding another crate.

### Desktop App

The app keeps only desktop responsibilities:

```text
app/src/
|-- main.rs
|-- lib.rs
|-- camera_config.rs
|-- analysis_task.rs
|-- logging.rs
|-- workflow/
|   |-- mod.rs
|   |-- workflow.rs
|   \-- error.rs
|-- preview/
|-- components/
\-- views/
```

`main.rs` becomes a thin call to `app::launch()`. `lib.rs` owns startup, Dioxus context, routes, and modules. This makes pure application orchestration importable from integration tests while keeping the executable trivial.

### Visibility Rule

The user explicitly supersedes the current narrow-visibility repository rule for new and moved code. Update root `AGENTS.md` to require private items or plain `pub`, not `pub(crate)`, `pub(super)`, or `pub(in ...)`. Child modules such as `analysis::agent` remain private, so their plain-`pub` sibling-facing items are not externally reachable; `analysis/mod.rs`, `recording/mod.rs`, and `session/mod.rs` re-export only the documented external API.

Apply this normalization to the moved backend modules and new app modules. Do not churn unrelated camera and Synology simulator modules solely to normalize old visibility.

### Dependency Changes

- Add `backend` to workspace members and make `app` depend on it by path.
- Move base64, `ffmpeg-sidecar`, Reqwest, Rig, and their test-only HTTP/model dependencies from app to backend. Keep UUID in both app and backend because Workflow stores session IDs.
- Add `sha2` to backend for stable plan fingerprints.
- Add `tracing` as a workspace dependency used by backend and app.
- Add `url` and UUID as workspace dependencies used by backend and app.
- Add `tracing-subscriber` and `tracing-appender` to app.
- Enable Tokio `time` in app for the Monitor ticker.
- Keep Serde, Serde JSON, tempfile, thiserror, and Tokio in workspace dependencies where multiple crates use them.
- Remove unused `dioxus-icons` and Git-based `dioxus-primitives`; the UI uses native elements plus Tailwind/daisyUI.
- Add app feature `paid-openai-test = []`; paid integration-test code is absent unless that feature is explicitly enabled.

## Backend Public Interfaces

### Session Domain

The public session API is:

```rust
pub enum OperatorAction {
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

pub struct SessionCamera {
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    pub sample_every: Duration,
}

pub struct Session {
    pub id: Uuid,
    pub start_utc_ms: i64,
    pub end_offset: Duration,
    pub cameras: Vec<SessionCamera>,
    pub actions: Vec<(Duration, OperatorAction)>,
}

pub struct SessionController { /* private fields */ }

impl SessionController {
    pub fn create(
        events_path: PathBuf,
        cameras: Vec<SessionCamera>,
    ) -> Self;

    pub fn apply(&mut self, action: OperatorAction) -> Result<()>;

    pub fn elapsed(&self) -> Duration;
}

impl Session {
    pub fn load(events_path: &Path) -> Result<Self>;
}
```

`elapsed` delegates to the controller's existing monotonic `Instant`; the UI does not create another session clock.

### Stored Session Catalog

`events.jsonl` already contains all metadata Analyzer needs: session UUID, UTC anchor/end, stable camera IDs/names, initial cadence/participation, and every change. Do not introduce another metadata file.

Add:

```rust
pub struct StoredSession {
    pub events_path: PathBuf,
    pub session: Session,
}

pub fn list_sessions(root: &Path) -> Result<Vec<StoredSession>>;
```

`list_sessions` scans only direct child directories, loads `events.jsonl`, skips incomplete or invalid logs with a structured warning, and sorts valid completed sessions newest-first by `(start_utc_ms, id)`. A missing root returns an empty list. Root directory I/O failure is returned. The directory is `events_path.parent()` and the analysis path is `events_path.with_file_name("analysis.json")`; do not store derivable path copies.

### Analysis Results And Checkpoint

The structured model types become public with public data fields:

```rust
pub struct Observation {
    pub timestamp: String,
    pub description: String,
}

pub struct ChecklistProgress {
    pub item: String,
    pub status: String,
    pub note: String,
}

pub struct AnalysisResponse {
    pub observations: Vec<Observation>,
    pub sequence_summary: String,
    pub checklist_progress: Vec<ChecklistProgress>,
}

pub enum AnalysisWarning {
    RecordingCoverageEnded {
        camera_id: u32,
        session_offset_ms: u64,
    },
}

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

The checkpoint schema increments from version 1 to version 2. There is no shipped production checkpoint compatibility requirement. The response vector position is the batch index, so the v1 `CompletedBatch { index, response }` wrapper and contiguous-index validation are deleted.

`read` validates schema, expected session ID, non-empty checklist, non-empty fingerprint, and response-count bounds. Analyzer performs additional expected checklist, fingerprint, total-batch, and warning validation.

The fingerprint is stable SHA-256 over this canonical byte sequence: ASCII `leo-analysis-plan-v1\0`; batch size and frame-set count as checked `u64` little-endian; then, for each frame set, session-offset milliseconds and frame count as checked `u64` little-endian; then each frame's camera ID as `u32` little-endian and recording ID, sample index, and recording-offset milliseconds as checked `u64` little-endian. A golden digest test locks the encoding. It prevents accepting old responses when events, recording coverage, frame ordering, or batch boundaries change while the total batch count happens to remain equal.

Persisting the checklist prevents a retry or app restart from mixing previous responses with new checklist text. The Analyze textarea is initialized from and locked by an existing checkpoint.

Analyzer atomically saves a zero-response checkpoint immediately after successful planning and before the first model request. The checklist, fingerprint, total batches, and truncation warnings therefore survive a first-batch provider failure.

### Analysis Entrypoint

Keep generic `Agent<M>` and `Analyzer<M>` private to the backend analysis module. The app and a future CLI use one concrete facade instead of coordinating provider, recording, checkpoint, and batching internals themselves:

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

`analyze_session` derives `analysis.json` beside `events.jsonl`, uses a fixed five-frame-set batch size, constructs the existing unauthenticated Synology client and OpenAI Responses agent, saves the initial planned checkpoint, then emits a complete checkpoint snapshot after planning and after every successful batch save. The facade lives in `analysis/facade.rs`; its public `thiserror` error lives in `analysis/error.rs`, keeping `analysis/mod.rs` declaration/re-export-only.

The private generic implementation accepts `Agent<M>` and `SynologyClient` so existing Rig mock/Axum tests remain deterministic without expanding the public API. Prompt construction, frame planning, extraction, and checkpoint save helpers remain private.

`ANALYSIS_MODEL` selects the model, `OPENAI_API_KEY` provides credentials, and optional `OPENAI_BASE_URL` selects a compatible endpoint. The local acceptance configuration uses `ANALYSIS_MODEL=luna` as the current model identifier.

## Finite Recording Coverage

Two separate problems must remain distinct:

1. A fixed historical fixture and a newly created session have completely different UTC timelines.
2. A correctly aligned finite fixture may end before the software session.

Truncating at the end of media solves only the second. It cannot make an August 8 recording overlap a session created on August 9. The local simulator therefore retains an explicit query-alignment launch mode, while Analyzer gains normal finite-end truncation.

### Simulator Query Alignment

Add an opt-in simulator CLI flag:

```text
--align-recordings-to-query
```

The flag is simulator configuration only. It is not a Synology HTTP field or fixture field. Without the flag, every current API response remains unchanged.

Carry the flag beside existing camera state without changing fixture rows:

```rust
#[derive(Clone)]
struct ApiState {
    cameras: CameraState,
    align_recordings_to_query: bool,
}

pub async fn server::start(
    cameras: Vec<Camera>,
    address: SocketAddr,
    align_recordings_to_query: bool,
) -> io::Result<()>;
```

Camera and ExternalRecording handlers continue receiving only `CameraState`; the entry dispatcher passes the boolean only to Recording List. Download ignores it.

For List v5/v6 requests with non-zero `fromTime`, alignment computes request-local projected bounds:

```text
projectedStart = fromTime + (recording.startTime - earliestCatalogueStart)
projectedStop  = fromTime + (recording.stopTime  - earliestCatalogueStart)
```

Filtering, sorting, pagination, and v5 response timestamps use projected bounds. V6 remains metadata-only. IDs, paths, durations, Download behavior, and stored catalogue values remain unchanged. Checked arithmetic failure returns Recording error 401.

### Analyzer Truncation

For each scheduled sample, zero matching recordings records `first_uncovered`; more than one matching recording always remains an overlap error. Sequence construction stops collecting frames at its first zero-match sample.

Analyzer then:

1. Builds one sequence candidate per participating camera.
2. Finds the earliest missing scheduled sample across all candidates.
3. Removes every frame at or after that session offset from every camera.
4. Adds `RecordingCoverageEnded` only for cameras whose `first_uncovered` equals that global truncation offset.
5. Continues normally if at least one frame set remains.
6. Returns `NoAnalyzableFrames` before checkpoint or model creation if truncation leaves no frame sets.

This handles a camera/network archive ending during a session without pretending recording continued. It intentionally stops analysis at the first gap rather than skipping a hole and resuming later.

Warnings are persisted in `analysis.json` and rendered above model observations. They are application facts, not model-generated `AnalysisResponse` content.

## Two-Camera Local Fixtures

Normalize the tracked synchronized salon files with the existing `prepare-camera-video` command and commit the resulting files:

```text
camera/fixtures/salon-1.mp4
camera/fixtures/salon-2.mp4
```

The normalized media is H.264 Constrained Baseline level 3.1, 1280x720, fixed 15 FPS, no B-frames, no audio, and one-second GOPs. This supports WHEP playback and accurate stream-copy range downloads. The prepared files are approximately 4.8 MB and 4.3 MB.

The Synology fixture catalogue has two 24-second rows, both beginning at the same fixed UTC timestamp, with IDs and camera IDs 1 and 2. The local aligned launch projects both to each requested session start.

Add recipes:

```text
just camera-1   # HTTP 8080, RTSP 8554, salon-1.mp4
just camera-2   # HTTP 8081, RTSP 8555, salon-2.mp4
just synology   # both HTTP camera addresses, two-row catalogue, alignment flag
just app
```

Preview loops indefinitely because the virtual camera's MediaMTX loops files. Synology archive coverage remains finite at 24 seconds. The analyzed files are synchronized with each other, but independently launched preview loops are not guaranteed to show the exact same phase later analyzed offline.

## Camera Configuration

Add a checked-in runtime file at workspace root:

```text
cameras.json
```

Schema:

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

App interface:

```rust
pub struct CameraConfig {
    pub id: u32,
    pub name: String,
    pub rtsp_url: String,
    pub enabled: bool,
    pub sample_every_ms: u64,
}

pub fn load_cameras(path: &Path) -> Result<Vec<CameraConfig>>;
```

The loader requires exactly two rows and rejects unknown fields, zero/duplicate IDs, empty names/URLs, and zero intervals. `LEO_CAMERA_CONFIG` can override the default `./cameras.json` path. A later Settings page may replace this file; no settings persistence is added now.

Load `LEO_SYNOLOGY_URL` once at startup, defaulting to `http://127.0.0.1:5000`, parse it as `url::Url`, and store it in Workflow. Invalid URLs disable analysis with a visible startup error before any request while preview and session controls remain available.

The same values map into:

- `CameraSource { id, name, rtsp_url }` for preview;
- `SessionCamera { id, name, enabled, sample_every }` at session start;
- selected-camera UI state.

`PreviewFeed` also carries `camera_id`. MediaMTX paths remain based on vector index (`camera-0`, `camera-1`) and never become Synology identity.

## Session Storage And Discovery

Session root is `./sessions`, ignored by Git. Start creates:

```text
sessions/<UTC-milliseconds>/events.jsonl
```

Directory creation and event-log creation fail rather than reuse an existing path. Analysis writes beside it:

```text
sessions/<UTC-milliseconds>/analysis.json
```

Analyze calls `list_sessions` and displays all valid completed sessions, including sessions created before the current app launch. Incomplete active logs are not listed and are recorded as structured warnings. No separate index or metadata file is needed.

Each list item shows:

- start UTC;
- duration;
- session UUID;
- camera count;
- analysis state: not started, in progress, complete, warning, or invalid checkpoint.

Selecting a session loads its checkpoint when present and displays a recap. Starting a new session never deletes or hides older completed sessions.

Workflow loads each existing checkpoint with `AnalysisCheckpoint::read(path, stored.session.id)`. A mismatched session ID is stored as that row's checkpoint error; its checklist and responses are never displayed.

## App Workflow State

Define one plain Rust application state shared through `Signal<Workflow>` above the router:

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
    pub checkpoint: Result<Option<AnalysisCheckpoint>, String>,
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
    pub synology_url: Result<url::Url, String>,
}
```

Entry points:

```rust
impl Workflow {
    pub fn new(
        cameras: Vec<CameraConfig>,
        session_root: PathBuf,
        synology_url: Result<url::Url, String>,
    ) -> Result<Self>;

    pub fn refresh_sessions(&mut self) -> Result<()>;
    pub fn select_camera(&mut self, camera_id: u32);
    pub fn select_session(&mut self, session_id: Uuid);

    pub fn start_session(&mut self, utc_ms: i64) -> Result<()>;
    pub fn set_selected_participation(&mut self, enabled: bool) -> Result<()>;
    pub fn set_selected_interval(&mut self, sample_every: Duration) -> Result<()>;
    pub fn stop_session(&mut self) -> Result<()>;

    pub fn begin_analysis(
        &mut self,
        checklist: String,
    ) -> Result<AnalyzeSession>;

    pub fn apply_checkpoint(&mut self, checkpoint: AnalysisCheckpoint);
    pub fn analysis_failed(&mut self, session_id: Uuid, message: String);
}
```

`start_session` takes injected UTC milliseconds so path construction is deterministic in tests. The controller still owns authoritative UTC event timestamps and monotonic offsets.

Participation and interval changes are write-before-state: UI values change only after `SessionController::apply` succeeds. Any controller action error moves the session to `Faulted`, disables further writes, preserves the directory for inspection, and requires starting a new process/session. A write, flush, or sync error can leave an uncertain complete or partial event, so the UI must not restore the controller and continue assigning sequence numbers. Successful Stop returns to Idle and refreshes the durable session list.

Only one analysis may be Running. A software session may run while an older completed session is analyzed because the controller and Analyzer own separate files and clients.

## Background Analysis Entrypoint

`begin_analysis` trims and rejects an empty checklist before any client or model construction. If an existing valid checkpoint is present, its checklist is authoritative and the textarea is read-only. An invalid checkpoint is never replaced and disables Analyze/Resume for that row. Otherwise the entered checklist is frozen in `AnalyzeSession`. It rejects a second start while `running_analysis_id` is set.

The Analyze button calls backend `analyze_session` through Dioxus 0.7 `spawn_forever`:

```rust
dioxus::dioxus_core::spawn_forever(async move {
    let session_id = request_session_id;
    let result = backend::analysis::analyze_session(request, move |checkpoint| {
        workflow.write().apply_checkpoint(checkpoint);
    })
    .await;

    if let Err(error) = result {
        workflow
            .write()
            .analysis_failed(session_id, error.to_string());
    }
});
```

`spawn_forever` runs in the root scope and is not canceled when the route unmounts. The future owns backend analysis state across awaits and holds a workflow signal write guard only while applying each complete checkpoint snapshot or final error. `apply_checkpoint` clears the matching `running_analysis_id` when `responses.len() == total_batches`; `analysis_failed` clears it on failure. Progress and results are derived from the selected session's checkpoint; they are not duplicated in another update DTO.

`LaunchBuilder` receives only cloneable bootstrap data (`PreviewState`, camera configuration, parsed Synology URL or its analysis-only error, and session root). The root `App` creates `Signal<Workflow>` with `use_signal` and provides it with `use_context_provider`; a Dioxus signal is not injected through `LaunchBuilder::with_context`.

## Monitor UI

Keep the existing Monitor route and route-specific sidebar.

### Preview Grid

- Render exactly two configured feeds with stable keys.
- Render `reader.js` once in Monitor rather than once per card.
- Remove hard-coded `LIVE`, timestamp, `Selected`, `CAM 04`, and unconditional status claims.
- Provide a semantic Select button on each card.
- Show a small labelled status indicator: green `Included` or red `Excluded`.
- Keep excluded previews mounted and playing.
- Use a simple selected border, not decorative cards, gradients, shadows, or animation.

### Monitor Sidebar

Idle state:

- Start session button.
- Selected camera name and configured initial cadence.
- Session storage path.

Active state:

- Active status and elapsed time from `SessionController::elapsed`.
- Selected camera name.
- Include/Exclude button for the selected camera.
- Native integer input labelled `Sampling interval (seconds)`, minimum 1, default 1.
- Apply cadence button that emits `SetSamplingInterval`.
- Stop session button.
- Current session directory.

Faulted state:

- Visible error and affected session directory.
- No further participation, cadence, or Stop writes.
- Guidance to restart the app and inspect the incomplete JSONL file.

Successful Stop returns to idle controls and refreshes the session list. The app remains on Monitor; navigation is explicit.

A Monitor-local `use_future` ticker increments a private display signal once per second while Monitor is mounted. The tick only triggers rerendering; displayed elapsed time always comes from `SessionController::elapsed`. Leaving Monitor cancels the ticker but does not affect the session controller.

Use semantic buttons, labels, inputs, status/alert live regions, Tailwind layout utilities, and daisyUI `btn`, `input`, `badge`, and `alert` classes. Styling remains intentionally sparse.

## Analyze UI

Keep the Analyze route. It has no video preview.

### Analyze Sidebar

- Refresh sessions button.
- Newest-first completed session list.
- Each row shows start time and analysis status.
- Selecting a row updates body details.

### Analyze Body

For the selected session show:

- UUID, start UTC, duration, camera count, and directory;
- checkpoint warning or validation error;
- checklist textarea, prefilled and read-only when a checkpoint exists;
- explicit Analyze or Resume button;
- progress element with completed/total batches;
- persisted analysis warnings;
- all observations from completed batches;
- latest cumulative sequence summary;
- latest checklist item status and note;
- `events.jsonl` and `analysis.json` paths.

Opening or selecting a session never starts a model request. Only the explicit button does. While an analysis runs, navigation remains enabled and its status remains visible when returning to Analyze.

## Structured Logging

Use `tracing`, `tracing-subscriber`, and `tracing-appender`.

App startup creates `./logs`, ignored by Git, and installs:

- compact human-readable console logs;
- daily JSON logs under `./logs/leo.jsonl.<date>`;
- `RUST_LOG` filtering with default level `info`.

The non-blocking appender `WorkerGuard` is retained alongside the MediaMTX `Bridge` owner until the desktop event loop is destroyed.

Add structured events/spans for:

- configuration load and preview startup;
- session start/action/stop with paths and camera IDs;
- session catalog skips;
- analysis planning, batch start/completion, truncation warnings, failure, and completion;
- simulator alignment mode and projected List bounds.

Never log API keys, passwords, SIDs, full prompts/checklists, image data, or credential-bearing URLs.

## Error Handling

- Invalid camera configuration launches an unavailable-state UI with the error and no session controls.
- Invalid `LEO_SYNOLOGY_URL` leaves preview/session controls available, renders a persistent Analyze alert, and disables Analyze/Resume before a click.
- Fallible Monitor/Analyze actions copy their error into the shared live `Workflow.message`; the next successful action clears it.
- Session directory or initial event creation failure leaves workflow idle.
- Any participation, cadence, or End append error moves the session to Faulted and disables further writes because file state may be uncertain.
- Incomplete/invalid persisted sessions are skipped from the selectable list and logged.
- Invalid checkpoints remain visible as a per-session error; they are never silently replaced.
- Empty checklist fails before model construction.
- Missing environment, List, coverage, Download, FFmpeg, model, and checkpoint errors remain distinct.
- Mid-session finite media end produces a persisted warning and successful truncated analysis when frames remain.
- Missing coverage at the first sample produces `NoAnalyzableFrames` and no model request.
- Failed analysis preserves all checkpointed responses and can be retried.

## Testing Strategy

### Moved Backend Regression Suite

Move all existing session, recording, analysis, FFmpeg, Analyzer, and Agent tests into `backend` unchanged before behavior changes. The mechanical extraction is complete only when their original assertions pass from the new crate.

### New Backend Unit Tests

- Session catalog scans valid direct children, ignores unrelated paths, skips incomplete logs, and sorts newest-first.
- Checkpoint v2 persists checklist, warnings, plan fingerprint, and zero-response initial planning state.
- Checkpoint reads reject a mismatched session before exposing checklist or responses.
- Resume rejects changed checklist or frame/batch plan fingerprints.
- Sequence planning reports first missing sample while preserving overlap errors.
- Analyzer truncates all cameras at the earliest gap and persists warnings.
- Analyzer rejects zero analyzable frame sets before model invocation.
- The concrete analysis facade emits complete checkpoint snapshots after planning and every saved batch.

### Simulator Tests

- CLI defaults alignment off and parses the explicit flag.
- Fixed mode preserves every existing response.
- Alignment shifts the global earliest recording to `fromTime` and preserves relative offsets/durations.
- Filtering, sorting, pagination, v5 timestamps, and v6 selection use projected bounds.
- `fromTime=0` stays fixed.
- Checked projection overflow returns Recording error 401.
- Download behavior remains unchanged.

### App Unit Tests

- Camera config parsing and validation.
- Stable ID mapping into PreviewFeed and SessionCamera independent of MediaMTX index.
- Workflow start, write-before-state participation/cadence, faulted-session transition, stop, and catalog refresh.
- Session selection and checkpoint checklist locking.
- Checkpoint projection aggregates all observations and uses the latest summary/checklist.
- Root task state remains independent from route-local components.

### Integration Tests

Create importable app-library tests under `app/tests`:

- `operator_session_flow.rs` runs Start, participation/cadence changes, Stop, catalog discovery, and exact durable-file assertions in a temporary root.
- `analysis_workflow.rs` feeds planned and completed checkpoint snapshots through the same Workflow callback used by the root task, verifying navigation-independent state, warnings, result projection, failure, and resume presentation. Backend's existing private generic Analyzer tests continue to cover Axum and Rig mocks.
- Dioxus `VirtualDom` render tests verify Monitor and Analyze render the expected controls/status/results for prepared workflow states. They do not pretend to be native click automation.

### Paid OpenAI Test

Keep one paid test only at `app/tests/paid_openai_workflow.rs`. The app feature `paid-openai-test` compiles that test; it exercises the public backend `analyze_session` entrypoint, real FFmpeg, HTTP media retrieval, real OpenAI transport, checkpoint save, Workflow checkpoint callback, and final result projection. It remains:

- compiled only with Cargo feature `paid-openai-test`;
- `#[ignore]`;
- protected by `LEO_RUN_PAID_OPENAI_TEST=1` before client construction;
- filtered by exact test name;
- absent from normal CI and blanket ignored-test commands;
- limited to one short batch.

Update `AGENTS.md`: never set the paid-test gate or run the paid test without explicit user approval; never run blanket `cargo test -- --ignored` because it includes paid coverage.

Native Dioxus/Wry click automation is not available in the current repository. Do not add a harness in this increment. The paid automated test covers the same application workflow entrypoint used by UI handlers; final desktop acceptance verifies actual clicks and rendering manually.

## Local Acceptance

1. Run the two prepared virtual cameras.
2. Run aligned Synology with the two-row catalogue.
3. Launch the desktop app and verify two distinct previews.
4. Start a session.
5. Select camera 2, set its interval to two seconds, then exclude and re-include it.
6. Stop after a short interval.
7. Verify the session appears in Analyze and `events.jsonl` contains ordered start, cadence, participation, and end events.
8. Without a paid/provider-approved endpoint, verify selection, checklist editing, explicit Analyze availability, and that no request starts on mount or selection.
9. After explicit paid/provider approval, start analysis, navigate to Monitor while it runs, then return and verify progress continued.
10. Verify warnings/results render and `analysis.json` is beside `events.jsonl`.
11. Restart the app and verify the completed session and checkpoint are rediscovered.
12. Verify the session directory contains no downloaded clips or JPEGs.

The paid OpenAI portion runs only after explicit approval. All non-paid desktop and simulator checks run first.

## Deferred Work

- A CLI binary using `backend`.
- Multiple concurrent analyses and cancellation.
- Search, pagination, deletion, and naming for stored sessions.
- Settings UI replacing `cameras.json` and environment configuration.
- Camera discovery.
- Active-session recovery after restart.
- Skipping a recording gap and resuming later media.
- Physical NAS authentication UI and identity reconciliation.
- Raw JSON viewers and native open-folder actions.
- Packaged sidecars and installers.
- Automated native Wry click-driving unless a harness is explicitly selected.

## Pending Confirmation

- Confirm that `luna` is the exact `ANALYSIS_MODEL` value and identify the OpenAI Responses-compatible `OPENAI_BASE_URL` used for approved local acceptance.
