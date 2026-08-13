# Host Recording Operator Workflow Design

Date: 2026-08-12

Status: Approved

## Goal

Deliver the first complete local operator workflow without a NAS or Synology dependency:

```text
two camera RTSP streams
        |
        |-- app preview bridge -> WHEP/WebRTC -> Monitor
        |
        `-- supervised FFmpeg recorders -> session-local MKV segments
                                               |
                                               v
events.jsonl + MKV segments -> direct FFmpeg frame extraction
                                               |
                                               v
                                  model analysis -> analysis.json
```

The app records every configured camera only while a software session is active. Camera participation and cadence changes affect analysis sampling, not recording. An excluded camera remains visible and continues recording until the session stops.

The complete operator flow is:

```text
virtual or physical cameras are already running
                    |
                    v
app starts -> two live previews
                    |
                    v
Start -> all recorders must receive media
                    |
                    v
session starts -> events.jsonl + MKV segments
                    |
                    v
temporary camera loss -> finalize segment, retry, record gap
                    |
                    v
Stop -> end event, finalize recorders, completion marker
                    |
                    v
Analyze discovers completed session from disk
                    |
                    v
explicit model analysis reads MKV segments directly
                    |
                    v
progress, gap warnings, and results persist in analysis.json
```

## Decisions

- Remove Synology and NAS support rather than preserving unused compatibility code.
- Record only during a software session.
- Require every configured camera to produce media before the session begins.
- Supervise one direct FFmpeg RTSP/TCP recorder per camera.
- Use video-only stream copy into Matroska (`.mkv`); do not transcode.
- Retry a disconnected camera every second until the operator stops the session.
- Preserve and analyze media recorded after a camera reconnects.
- Put the complete portable session under one configurable data root, suitable for a later external SSD mount.
- Let the app own recorder lifetime. Recording survival and active-session recovery after an app crash are out of scope.

## Existing Implementation To Reuse

Commit `698ce32` already implements and tests:

- durable JSONL session writes and strict completed-session replay;
- normalized participation and cadence schedules;
- deterministic recording coverage, sample sequences, and frame sets;
- FFmpeg JPEG extraction from a local seekable video file;
- structured Rig/OpenAI requests and responses;
- deterministic batching, atomic `analysis.json` checkpoints, rollback, and resume;
- guarded ignored FFmpeg and paid OpenAI integration tests.

Keep the session, sampling, extraction, prompt, model, and checkpoint behavior. Delete the Synology catalogue/download boundary and make the Analyzer consume finalized local segments directly.

## Scope

- A reusable `backend` library containing session, host-recording, and analysis modules.
- Deletion of the entire `synology` workspace crate and app Synology HTTP client.
- Runtime configuration for exactly two camera IDs, names, and RTSP URLs.
- A configurable local data root, defaulting to `./data`.
- Session-scoped supervised FFmpeg recording into MKV files.
- All-camera startup readiness and rollback on startup failure.
- Automatic reconnect into a new segment after temporary camera loss.
- Session completion only after the event log and every recorder are finalized.
- Direct local segment discovery and FFprobe duration validation.
- Analysis that skips recording gaps and resumes with later segments.
- Monitor controls and recording-health status.
- Completed-session discovery, explicit background analysis, progress, warnings, and results.
- Structured console and JSONL logs.
- Unit, integration, ignored media, and explicitly gated paid model coverage.
- Minimal Tailwind CSS and daisyUI presentation.

## Explicit Non-Goals

- Continuous recording outside a software session.
- Recording survival after a process crash, forced termination, laptop sleep, or power loss.
- Active-session recovery or recorder reattachment after app restart.
- Automatic retention, storage rotation, disk-space prediction, session deletion, or export.
- Multiple concurrent sessions or analyses.
- Analysis cancellation.
- Camera discovery or a Settings UI.
- Analyze-page video playback.
- Sampling faster than whole seconds in the first UI.
- Permanent JPEG extraction artifacts.
- A generic recording-source trait or alternate recorder implementation.
- A CLI executable in this increment.
- Packaged FFmpeg, FFprobe, MediaMTX, SSD mounting, or installers.

## Workspace Architecture

The workspace contains three crates after this change:

```text
app/
|-- Cargo.toml
|-- backend/     reusable session, host recording, and analysis library
|-- app/         Dioxus desktop operator application
`-- camera/      local Axis-shaped virtual camera
```

Delete `synology/` and remove it from workspace members.

### Backend Crate

Move the existing backend modules out of the desktop binary and remove Synology-specific code while doing so:

```text
backend/
|-- Cargo.toml
`-- src/
    |-- lib.rs
    |-- session/
    |   |-- mod.rs
    |   |-- controller.rs
    |   |-- session.rs
    |   |-- catalog.rs
    |   `-- error.rs
    |-- recording/
    |   |-- mod.rs
    |   |-- recorder.rs
    |   |-- segment.rs
    |   `-- error.rs
    `-- analysis/
        |-- mod.rs
        |-- facade.rs
        |-- error.rs
        |-- agent/
        |-- analyzer/
        `-- video/
```

`backend/src/lib.rs` exposes:

```rust
pub mod analysis;
pub mod recording;
pub mod session;
```

The desktop app owns Dioxus state and views. The backend owns processes and files that a future CLI could also use.

### Desktop App

```text
app/src/
|-- main.rs
|-- lib.rs
|-- analysis_task.rs
|-- camera_config.rs
|-- logging.rs
|-- session_task.rs
|-- workflow/
|-- preview/
|-- components/
`-- views/
```

`main.rs` becomes a thin call to `app::launch()`. Root-scoped tasks own recorder startup/shutdown and model analysis across route navigation. View components only issue actions and render shared `Signal<Workflow>` state.

### Visibility And Error Rules

For new and moved modules, use private items or plain `pub`, not restricted `pub(...)`. Keep child modules private and re-export only the documented module API. This explicitly supersedes the current narrow-visibility rule for this reorganization; unrelated camera code is not churned.

Use `thiserror`. Keep each large module's non-trivial error in its `error.rs`. Public entry points return public error types, and public structs have concise responsibility and ambiguous-field documentation.

## Runtime Configuration

### Camera Configuration

Add workspace-root `cameras.json`:

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

`LEO_CAMERA_CONFIG` overrides the default `./cameras.json`. The loader requires exactly two rows and rejects unknown fields, zero or duplicate IDs, blank names or RTSP URLs, unsupported URL schemes, and sampling intervals that are zero or not a whole number of seconds.

The stable camera ID is shared by preview metadata, session events, recording directories, frame metadata, warnings, and UI state. MediaMTX vector indices remain private preview path identifiers.

### Data Root

`LEO_DATA_DIR` defaults to `./data`. The app creates:

```text
data/
|-- sessions/
`-- logs/
```

Changing only `LEO_DATA_DIR` to a mounted SSD path moves every session's event log, recordings, checkpoint, and results together. Camera configuration remains an app deployment setting rather than session data.

The app validates that the root and child directories can be created. Capacity monitoring and mount management are deferred.

### Recorder Calibration

`LEO_RECORDER_TIMEOUT_SECS` defaults to `10` and must be a positive integer representable as FFmpeg microseconds and as a Rust deadline. It controls both initial all-camera readiness and FFmpeg's bounded RTSP network I/O. A real camera/network can tune this without recompiling. Reconnect backoff is fixed at one second, and graceful shutdown gets five seconds before forced termination.

Before creating any session directory, the recorder runtime verifies that `ffmpeg` and `ffprobe` are available and executable. Tests use a private constructor that accepts alternate executable paths; there is no user-facing executable-path setting.

## Session Storage

One session is portable and self-contained:

```text
data/sessions/<start-request-UTC-ms>/
|-- events.jsonl
|-- recording-complete
|-- analysis.json
`-- recordings/
    |-- camera-1/
    |   |-- 1786552800123.mkv
    |   |-- 1786553050456.mkv
    |   `-- .attempt-<uuid>.partial.mkv
    `-- camera-2/
        `-- 1786552800188.mkv
```

The timestamped directory is a collision-resistant key, not the durable session UUID. Creation uses `create_new`/`create_dir`, never path reuse.

Only a direct regular file whose stem is a valid UTC millisecond integer and whose extension is exactly `.mkv` is a finalized segment. `.partial.mkv` files are never analyzed. They may remain after an invalid interrupted write or unclean app exit for diagnosis. Session, camera, marker, event, checkpoint, and segment discovery uses no-follow metadata and rejects symbolic links.

`recording-complete` is a zero-byte marker created atomically only after:

1. `EndSession` was appended successfully;
2. every FFmpeg child was stopped and reaped;
3. every valid final segment was probed and renamed;
4. recorder shutdown produced no fatal host error.

The session catalogue requires both a valid ended `events.jsonl` and `recording-complete`. This prevents Analyze from racing recorder finalization or selecting an incomplete crash directory.

## Host Recorder

### Public Data And Runtime

```rust
pub struct RecordingCamera {
    pub id: u32,
    pub rtsp_url: String,
}

pub struct RecorderSettings {
    pub io_timeout: Duration,
    pub retry_delay: Duration,
    pub stop_timeout: Duration,
}

pub enum RecorderStatus {
    Starting,
    Recording,
    Reconnecting,
    Stopped,
}

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
    )>;

    pub fn shutdown(self) -> Result<()>;
}

impl RecorderHandle {
    pub async fn start(
        &self,
        cameras: Vec<RecordingCamera>,
        recordings_root: PathBuf,
    ) -> Result<()>;

    pub async fn stop(&self) -> Result<Vec<RecordingSegment>>;
}
```

`RecorderRuntime::spawn` preflights FFmpeg and FFprobe, starts one dedicated management thread, and returns a cloneable command handle plus a single event receiver. The runtime thread owns the active `RecorderSet`, every supervisor thread, and every FFmpeg child. `RecorderHandle` sends commands with one-shot replies, so Dioxus tasks await readiness/finalization without blocking its executor.

The runtime owns a shared shutdown token before any blocking startup work begins. `RecorderRuntime::shutdown` sets that token, stops/reaps every active child even during initial readiness or Stop finalization, closes the command channel, and joins the management thread. The desktop event-loop owner retains `RecorderRuntime` beside the preview bridge and log guard and calls `shutdown` during normal destruction. A hard process crash remains out of scope.

The Tokio event receiver is the only cross-thread UI boundary. Recorder threads never access a Dioxus signal.

Every public backend entry point independently validates that the camera list is non-empty, IDs are non-zero and unique, URLs parse with the `rtsp` scheme, output paths are direct directories under the supplied recordings root, and settings are positive and safely representable. The app loader is not treated as a trust boundary for reusable backend code.

### FFmpeg Command

Each camera supervisor starts FFmpeg equivalent to:

```text
ffmpeg -hide_banner -loglevel info
  -rtsp_transport tcp
  -timeout <timeout-microseconds>
  -i <camera-rtsp-url>
  -map 0:v:0 -an -c:v copy
  -avoid_negative_ts make_zero
  -f matroska
  <camera-dir>/.attempt-<uuid>.partial.mkv
```

There is no re-encoding. MKV is chosen because it accepts H.264/H.265 stream copy, is directly seekable by FFmpeg, and is more recoverable than ordinary MP4 after interruption. Browser playback is not a requirement.

Never log the RTSP URL because it may contain credentials.

### Initial Readiness

A supervisor parses FFmpeg's normal periodic `frame=... time=...` status lines and reports `Recording` after progress contains at least one output video frame and the partial output is non-empty. Do not pass `-nostats`, because those status lines are the readiness and timestamp source. Start requires all configured cameras, including cameras initially excluded from analysis, to reach that state within `io_timeout`. The shared shutdown token can interrupt this wait.

If any camera cannot become ready:

- request graceful quit from every started child;
- force-kill and reap children that exceed `stop_timeout`;
- remove the not-yet-started session directory when cleanup succeeds;
- return the Workflow to Idle with one visible error;
- do not create `events.jsonl` or `recording-complete`.

Only after all recorders are ready does `SessionController::create` durably write the session-start event. The recordings therefore include a short lead-in before session offset zero.

If event-log creation then fails, stop all recorders and remove the staging directory when possible; the Workflow remains Idle.

### Segment Time Bounds And Finalization

The partial output path is chosen before media arrives, so it cannot encode the actual segment start. On the first qualifying FFmpeg progress event for an attempt, freeze an estimated media start:

```text
observed wall-clock UTC - FFmpeg output media time
```

Do not update the estimate on later progress lines; parser and shutdown delays must not move an existing segment. For a camera's later segment, clamp that start to at least the previous finalized segment's end to prevent reconnect timestamp jitter from creating overlap. Default stream-copy behavior begins the output at the first copied keyframe, so the frozen estimate describes the first decodable output media rather than the original connection attempt.

On process exit or Stop:

1. Wait for FFmpeg to write its Matroska trailer when it exits normally.
2. Run FFprobe with JSON output for `format.start_time`, `format.duration`, and selected video stream indices.
3. Require exactly one video stream, finite nonnegative `format.start_time`, and finite positive `format.duration`. Stream-copy MKV may retain a small positive start because of codec reordering even with `-avoid_negative_ts make_zero`.
4. Convert start time with checked floor rounding and duration with checked ceiling rounding to milliseconds, then compute `media_span_ms = duration_ms - start_time_ms` and require a positive span.
5. Set the segment start to the frozen media-timeline-zero estimate plus `start_time_ms`, clamp that start against the previous finalized end, and set the exclusive end to `start_utc_ms + media_span_ms`.
6. Atomically rename the valid file to `<start_utc_ms>.mkv` without overwriting.
7. Keep an invalid non-empty partial file for diagnosis; remove an empty attempt.

Finalized segment identity is `(camera_id, start_utc_ms, end_utc_ms)`. Paths are intentionally excluded so a complete session can move between the host and SSD without invalidating an analysis checkpoint.

### Reconnect Behavior

After initial readiness, an unexpected FFmpeg exit or bounded RTSP timeout is not a session fault by itself:

1. finalize any valid media from the attempt;
2. report `Reconnecting` with a sanitized error;
3. wait one second;
4. start a new partial file;
5. report `Recording` after new media arrives;
6. continue until Stop, however long the camera remains unavailable.

Other camera supervisors continue recording independently.

Before retrying, write and sync a one-byte probe file in the camera directory. Failure indicates a host/storage problem rather than camera loss. A host/storage error or inability to spawn, stop, kill, or reap a child is fatal. The runtime emits `RecorderEvent::Faulted`; the root task attempts to append `EndSession`, commands the runtime to stop all recorders, moves the Workflow to Faulted, preserves the directory, and does not write `recording-complete`.

### Shutdown

Operator Stop first attempts to append `EndSession`, then always stops all recorders so an event-log failure cannot leave capture running.

For each child:

- send FFmpeg `q` for graceful trailer writing;
- wait up to five seconds;
- force-kill if necessary;
- reap the child;
- finalize any valid segment.

If the end event and recorder shutdown both succeed, create `recording-complete`, return to Idle, and refresh completed sessions. Any uncertain JSONL write or fatal process/storage cleanup error moves the session to Faulted and leaves it unselectable for analysis.

Normal app/event-loop shutdown requests the same recorder cleanup. A hard crash can leave FFmpeg children and partial files; crash-safe process supervision is explicitly deferred.

## Session Domain And Discovery

The existing event schema remains software-session metadata. Rename Surveillance Station-specific documentation to stable camera identity; no persisted schema change is required.

Public session APIs are:

```rust
pub enum OperatorAction {
    SetCameraParticipation { camera_id: u32, enabled: bool },
    SetSamplingInterval { camera_id: u32, sample_every: Duration },
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
    pub fn create(events_path: PathBuf, cameras: Vec<SessionCamera>) -> Result<Self>;
    pub fn apply(&mut self, action: OperatorAction) -> Result<()>;
    pub fn elapsed(&self) -> Duration;
}
```

`elapsed` uses the existing monotonic `Instant`.

Completed-session discovery adds:

```rust
pub struct StoredSession {
    pub directory: PathBuf,
    pub session: Session,
}

pub fn list_sessions(root: &Path) -> Result<Vec<StoredSession>>;
```

Scan direct child directories only. Skip/log unrelated, invalid, active, or missing-marker directories. Sort valid sessions newest-first by `(start_utc_ms, id)`. A missing root returns an empty vector; a root I/O failure is returned.

No session index or recording manifest is introduced. Derive `events.jsonl`, `analysis.json`, `recording-complete`, and `recordings/` from `StoredSession.directory`.

## Local Segment Discovery

### Segment Type

```rust
pub struct RecordingSegment {
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
    pub path: PathBuf,
}

pub fn list_segments(
    recordings_root: &Path,
    camera_ids: &[u32],
) -> Result<Vec<RecordingSegment>>;
```

For every requested camera, scan only `recordings/camera-<id>/`. Ignore partial and unrelated files. Parse finalized names as UTC milliseconds, FFprobe each file's `format.start_time` and `format.duration`, recompute `media_span_ms = ceil(duration) - floor(start_time)`, set the exclusive end to filename start plus that span, and sort by `(camera_id, start_utc_ms)`. Finalization and rediscovery therefore use the same interval calculation without double-counting a positive container start time.

Reject:

- a missing camera directory;
- zero/invalid duration;
- timestamp or duration overflow;
- two same-camera finalized segments with the same start;
- overlapping same-camera finalized intervals.

Zero finalized segments for one camera is valid recording evidence: analysis emits one full-session `RecordingGap` for that camera. Segments from different cameras normally overlap and are independent. `NoAnalyzableFrames` is returned only if no camera contributes any planned frame.

## Direct Analysis

### Coverage And Recording Gaps

For each camera, clip its ordered finalized segments to the software session UTC interval. Derive uncovered intervals from the complement of that coverage. Persist one warning per contiguous uncovered interval:

```rust
pub enum AnalysisWarning {
    RecordingGap {
        camera_id: u32,
        start_offset_ms: u64,
        end_offset_ms: u64,
    },
}
```

Offsets are half-open session-relative bounds. Disabled analysis periods do not change recording-gap facts; a physical gap is still shown even if the camera happened to be excluded from sampling then.

Sampling behavior changes from strict complete coverage:

- exactly one segment covers a scheduled sample: create a frame;
- no segment covers it: omit that camera frame at that offset;
- more than one segment covers it: return an overlap error.

Merge every remaining camera frame by session offset. A frame set may contain one or both cameras. An offset with no available camera frame produces no frame set. Continue after gaps, including frames from recovered segments. Return `NoAnalyzableFrames` only when the complete merged plan is empty.

### Frame Identity

Each internal frame references its local segment and stores:

- stable camera ID;
- segment start and end UTC milliseconds;
- session offset;
- segment-relative recording offset;
- sample index within that camera schedule;
- local path used only for extraction.

The deterministic plan fingerprint hashes batch size, ordered frame-set offsets, and every frame's camera ID, segment UTC bounds, sample index, and recording offset. It never hashes absolute paths, JPEG bytes, checklist text, or model output.

### Checkpoint V2

The model result DTOs are publicly re-exported from `backend::analysis` with public data fields because the app renders them directly:

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
```

The approved v2 checkpoint remains:

```rust
pub struct AnalysisCheckpoint {
    pub schema_version: u8,
    pub session_id: Uuid,
    pub checklist: String,
    pub plan_fingerprint: String,
    pub total_batches: usize,
    pub warnings: Vec<AnalysisWarning>,
    pub responses: Vec<AnalysisResponse>,
}
```

There is no shipped checkpoint compatibility requirement. Delete the v1 completed-batch wrapper. Vector position is the batch index.

Read validates schema, expected session UUID, non-empty checklist/fingerprint, and response-count bounds. Analyzer additionally validates expected checklist, plan fingerprint, total batches, and warnings. Save an initial zero-response checkpoint immediately after planning and before the first model request.

### Analysis Entrypoint

```rust
pub struct AnalyzeSession {
    pub directory: PathBuf,
    pub checklist: String,
}

pub async fn analyze_session(
    request: AnalyzeSession,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint>;
```

The facade:

1. trims/rejects an empty checklist;
2. requires `recording-complete`;
3. loads `events.jsonl`;
4. discovers/probes local segments;
5. builds gap warnings and the sample plan;
6. derives `analysis.json` beside the event log;
7. constructs the OpenAI Responses agent from environment;
8. uses exactly five frame sets per batch;
9. emits a complete checkpoint snapshot after initial planning and every successful batch save.

Batch materialization directly calls existing `extract_jpeg(segment.path, recording_offset)` through `spawn_blocking`. Delete Synology download windows, local clip offsets, batch-local downloaded-video maps, and temporary MP4 files. Temporary JPEG cleanup remains inside `extract_jpeg`.

`ANALYSIS_MODEL`, `OPENAI_API_KEY`, and optional `OPENAI_BASE_URL` configure the provider. No model name is hard-coded in the app or acceptance plan.

## App Workflow

### Shared State

The root provides one `Signal<Workflow>` above the router. Presentation state includes:

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
```

Workflow also owns completed-session rows, selected camera/session IDs, one running analysis ID, analysis error, transient message, session root, and one cloneable `RecorderHandle`. `RecorderRuntime` remains outside Dioxus state in the event-loop owner. Do not duplicate progress/results outside checkpoints.

### Root Session Task

Start and Stop are root-scoped async actions, not component-local futures:

- Start changes state to `Starting`, awaits `RecorderHandle::start`, then creates the controller and changes to `Active`.
- Recorder events update camera status through a Tokio channel and short signal writes.
- Stop changes state to `Stopping`, appends the end event, awaits `RecorderHandle::stop`, creates the marker, then returns to Idle.
- A `RecorderEvent::Faulted` starts the same root-owned end-and-cleanup path exactly once and leaves Workflow Faulted.
- Route navigation never cancels recorder lifecycle work.
- Duplicate Start/Stop actions are rejected by state.

Participation and cadence remain write-before-state. A JSONL append error faults the session because file state is uncertain, but the root task still stops every recorder.

### Root Analysis Task

Only one model analysis may run. `begin_analysis` rejects an empty checklist, missing selection, active/incomplete session, invalid checkpoint, or second running analysis before model construction. Existing checkpoint checklist text is authoritative.

Use Dioxus `spawn_forever` from root scope. The future owns backend analysis across awaits and writes complete checkpoint snapshots or a final error into Workflow. A final successful checkpoint clears the running ID. Route unmount does not cancel analysis.

## Monitor UI

### Preview Grid

- Render exactly two configured feeds with stable camera-ID keys.
- Load `reader.js` once.
- Keep both previews mounted while recording, reconnecting, or excluded from analysis.
- Provide a semantic Select button.
- Show `Included`/`Excluded` analysis status separately from recording health.
- Show recorder health as `Idle`, `Starting`, `Recording`, or `Reconnecting`.
- Remove fake `LIVE`, timestamps, camera numbers, selected claims, and inert options controls.
- Use a simple selected border with no gradients, animation, or decorative shadows.

### Sidebar

Idle:

- Start session;
- selected camera name and initial cadence;
- configured data/session root.

Starting:

- disabled Start;
- per-camera readiness status;
- Cancel is not added; startup either succeeds or reaches its timeout.

Active:

- elapsed time from `SessionController::elapsed`;
- per-camera recording/reconnecting status;
- selected camera name;
- Include/Exclude analysis action;
- integer `Sampling interval (seconds)` input with minimum/default `1`;
- Apply cadence;
- Stop session;
- current session directory.

Stopping:

- disabled session controls;
- visible finalization status.

Faulted:

- blocking error and affected directory;
- no further metadata writes;
- confirmation that recorder cleanup was attempted;
- restart/inspection guidance.

Status and errors use semantic live regions. Preview remains usable if recording is Idle. A preview failure does not prove recorder failure, and recorder health never claims that preview is healthy.

## Analyze UI

Analyze has no video preview.

Sidebar:

- Refresh sessions;
- newest-first completed session list;
- start UTC and derived analysis state per row.

Selected-session body:

- UUID, start UTC, duration, camera count, and directory;
- event log, recordings directory, completion marker, and analysis path;
- checkpoint validation error when present;
- checklist textarea, locked to persisted text after a checkpoint exists;
- explicit Analyze or Resume action;
- completed/total batch progress;
- persisted camera/time-range recording-gap warnings;
- all completed-batch observations;
- latest cumulative sequence summary;
- latest checklist statuses and notes.

Selection, refresh, mount, and typing never start a provider request. Navigation remains enabled while analysis runs.

## Logging

Use `tracing`, `tracing-subscriber`, and `tracing-appender`:

- compact human-readable console logs;
- daily JSON logs under `<data-root>/logs/leo.jsonl.<date>`;
- `RUST_LOG` filtering with default `info`.

Retain the non-blocking appender guard until desktop shutdown.

Structured events cover:

- camera configuration and data-root load;
- preview startup;
- recorder attempt/start/readiness/exit/reconnect/finalization/stop;
- session start/action/stop/fault;
- session catalogue skips;
- segment discovery and gap warnings;
- analysis planning, batch start/save, failure, and completion.

Never log API keys, RTSP credentials or full URLs, prompts/checklists, image bytes, or model request bodies.

## Error Handling

- Invalid camera/data/recorder configuration launches an unavailable-state UI with no Start control.
- Failure of any initial camera recorder rolls back every recorder and leaves Workflow Idle.
- Temporary camera loss after session start changes that camera to Reconnecting and never stops other cameras.
- Reconnect retries until Stop; recovered media is finalized and analyzed.
- Host/storage/process-supervision failure faults the session and stops all recorders.
- Initial event-log failure rolls back recorders and leaves Workflow Idle.
- Participation/cadence/end append errors fault metadata state and trigger recorder cleanup.
- Invalid/partial MKV files remain ignored diagnostic artifacts.
- Invalid or overlapping finalized segments fail analysis without replacing an existing checkpoint.
- Missing coverage creates persisted warnings and skips only unavailable frames.
- No frames across all cameras returns `NoAnalyzableFrames` before model construction.
- Provider, extraction, and checkpoint failures preserve every previously saved response for retry.
- Invalid checkpoints remain visible and are never silently replaced.

## Synology Removal

Delete rather than deprecate:

- workspace member and complete `synology/` crate;
- `app/src/recording/synology.rs` and Synology-specific errors/tests;
- Synology client use from Analyzer and facade;
- List v5 and Download v6 parsing/pagination/login/download code;
- Reqwest and `futures-util` dependencies no longer used elsewhere;
- app Axum HTTP mocks used only to simulate Synology;
- Synology fixtures, launch recipes, alignment mode, and media-download tests;
- `LEO_SYNOLOGY_URL` and all authentication/mount/DS assumptions;
- current README and architecture sections that describe Synology as a system component.

Leave historical design/plan documents in place as historical records. Rewrite the current architecture, README, current specification, and implementation plan around host recording. Do not preserve code compatibility aliases, dormant features, or a generic remote-recording abstraction.

## Testing Strategy

### Mechanical Backend Regression

Before behavior changes, move reusable session, sampling, extraction, Agent, and checkpoint tests into `backend`. Delete Synology client and HTTP-mock assertions. The unchanged reusable assertions must pass from the new crate.

### Recorder Unit Tests

- exact FFmpeg args use RTSP/TCP, bounded timeout, video-only stream copy, Matroska, and no RTSP URL logging;
- all-camera readiness requires one media-producing attempt per camera;
- one startup failure stops/reaps previously ready recorders and creates no event log;
- process exit finalizes a valid segment and emits Reconnecting;
- retry creates a new partial path and later emits Recording;
- Stop sends graceful quit, force-kills after timeout, and reaps every child;
- zero/invalid media is never promoted to a finalized filename;
- segment start clamping prevents same-camera overlap;
- storage probe/finalization failure emits a fatal event.

Normal process tests use small fake FFmpeg/FFprobe executables through the recorder module's private test constructor; do not add executable paths to public `RecorderSettings` and do not add a one-implementation process trait.

### Local Segment And Analysis Tests

- finalized filename parsing and FFprobe duration conversion;
- partial/unrelated file ignoring;
- missing camera directory, invalid duration, duplicate start, and overlap rejection;
- gap derivation before, between, and after segments;
- missing camera frames are skipped while other-camera frame sets remain;
- recovered post-gap segments contribute later frames;
- an entirely empty merged plan fails before model invocation;
- fingerprint is independent of absolute session path;
- checkpoint warnings and initial zero-response state persist;
- direct extraction uses the segment path and segment-relative offset;
- model/save rollback and resume semantics remain unchanged.

### App Tests

- camera/data/recorder configuration validation;
- stable camera IDs through preview, session, recording directories, and Workflow;
- Start/Starting/Active/Stopping/Idle transitions;
- write-before-state participation and cadence;
- reconnect status without session fault;
- fatal recorder event to Faulted plus cleanup request;
- completed-session discovery requires the marker;
- checklist locking, checkpoint projection, success/failure/retry state;
- root tasks remain independent from route-local components;
- Dioxus SSR renders semantic Monitor and Analyze states, including Reconnecting and recording-gap warnings.

### Ignored Media Tests

Run only by exact name inside the Nix development shell:

- start a real virtual camera and record a playable MKV with FFmpeg;
- stop the camera, verify reconnect status, restart it, and produce a second finalized segment;
- FFprobe both segments and verify a gap between them;
- extract JPEGs from segments before and after the gap;
- execute one full direct local FFmpeg plus mock-model analysis batch;
- retain the existing two-reader virtual-camera stream test.

### Paid Model Test

Keep one app-level paid test only. It is:

- compiled only with Cargo feature `paid-openai-test`;
- `#[ignore]`;
- gated by `LEO_RUN_PAID_OPENAI_TEST=1` before provider construction;
- selected by its full exact test name;
- absent from normal and blanket ignored-test execution;
- limited to one short batch over a local finalized MKV fixture;
- routed through public `analyze_session` and the real Workflow checkpoint callback.

## Local Acceptance

1. Start virtual cameras 1 and 2.
2. Launch the desktop app and verify two distinct previews.
3. Start a session and verify both cameras pass Starting before Active appears.
4. Select camera 2, set its interval to two seconds, exclude it, then re-include it; its recording remains active.
5. Stop camera 2 temporarily and verify its preview fails independently while recorder status becomes Reconnecting.
6. Restart camera 2 and verify recorder status returns to Recording without ending the session.
7. Stop the session and verify `events.jsonl`, `recording-complete`, and numeric MKV files exist under one session directory.
8. Verify FFprobe reads every finalized MKV and no recorder process remains.
9. Open Analyze, refresh/select the session, and verify a camera-2 recording-gap warning.
10. Enter a checklist and explicitly start analysis only after approved provider configuration is available.
11. Navigate to Monitor and back while analysis runs; verify progress continued and pre/post-gap results appear.
12. Restart the app and verify the completed session and checkpoint are rediscovered from `LEO_DATA_DIR`.
13. Verify no downloaded clips or JPEG files remain in the session directory.

The provider request and paid test run only after explicit approval. All camera, recorder, storage, UI, and mock-model checks run first.

## Deferred Work

- External SSD mount discovery, device identity checks, and eject handling.
- Disk-capacity monitoring and retention/deletion/export workflows.
- Recorder daemon/service independent of the desktop app.
- App-crash cleanup, orphan process detection, and active-session recovery.
- Continuous all-day recording outside sessions.
- Multiple concurrent sessions or analyses and cancellation.
- Settings UI and camera discovery.
- Video playback and raw JSON viewers.
- Physical camera acceptance and recorder timeout calibration.
- Packaged FFmpeg/FFprobe/MediaMTX and installers.
