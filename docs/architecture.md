# Leo Architecture

Leo is a local, session-scoped recording and analysis application for zero or more configured RTSP cameras. The desktop process owns preview, direct host recording, operator metadata, completed-session discovery, and explicit model analysis. A session is portable under one configured data root.

Open the [visual architecture map](architecture-map.html) for the same system as process, data, and ownership flows.

## Workspace

The Cargo workspace contains exactly three crates:

| Crate | Type | Responsibility |
| --- | --- | --- |
| `app` | Library plus desktop binary | Dioxus routes and views, startup configuration, preview bridge, shared operator state, logging, and root-scoped tasks. |
| `backend` | Library | Durable session events, FFmpeg recorder supervision, finalized segment discovery, sampling plans, gap warnings, frame extraction, provider transport, and checkpoints. |
| `camera` | Binary | One fixture-backed Axis-shaped virtual camera per process for local development and acceptance. |

The desktop crate depends on `backend`. The virtual camera is a separate local process and communicates only through HTTP and RTSP sockets.

## End-To-End Flow

```text
each configured camera RTSP (0..N)
                                +--> app MediaMTX --> WHEP/WebRTC --> keyed preview
                                |
                                `--> FFmpeg supervisor --> camera-<id>/*.mkv

events.jsonl + finalized MKVs + recording-complete
                                |
                                `--> local FFprobe/FFmpeg analysis plan
                                      --> explicit provider batches
                                      --> analysis.json
```

Preview and recording make independent RTSP/TCP connections. Preview availability does not establish recording health, and a failed preview does not stop a recorder. Camera participation and cadence are analysis settings only: an excluded camera remains visible and records for the complete active session.

### Backend Responsibility Boundaries

| Module | Responsibility |
| --- | --- |
| `recording::recorder` | Owns the command actor, per-camera supervisors, FFmpeg attempts, reconnects, and complete child-process cleanup. |
| `recording::segment` | Discovers only finalized direct MKV files and uses bounded FFprobe calls to validate their media timing and reject overlaps. It neither starts recording nor repairs partial files. |
| `session::controller` | Creates an active event log and durably appends validated operator actions. |
| `session::event_log` | Defines and strictly replays the private versioned JSONL schema into a completed `Session`. |
| `session::catalog` | Lists only ended, marker-gated sessions and durably creates the completion marker. |
| `analysis::session` | Validates a selected completed directory, discovers its segments, resumes analysis, and emits durable checkpoints. |
| `analysis::analyzer` | Builds the deterministic sampling plan, materializes model batches, and maintains `analysis.json`. |

`recording::recorder` remains one module despite its size. Its command actor, camera supervisors, FFmpeg attempt pump, and cleanup paths share stop and fault ownership. Splitting them would require a wider process-lifecycle API without removing state-machine complexity. About three fifths of the file is colocated concurrency and cleanup coverage.

## Application Settings

Leo owns production configuration in one strict, versioned `settings.json`. The desktop app supports macOS and Linux; Windows is not supported. Production startup reads that platform file and standard platform directories, not legacy application environment variables or the process current working directory.

| Platform | Settings file | Default data root |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Leo/settings.json` | `~/Library/Application Support/Leo/data/` |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/leo/settings.json` | `${XDG_DATA_HOME:-$HOME/.local/share}/leo/` |

The schema uses strict camel-case JSON and rejects unknown fields or unsupported schema versions. It persists `schemaVersion`, `nextCameraId`, an ordered `cameras` list, optional `dataRoot`, `recorderTimeoutSecs`, `analysisFrameSetsPerPrompt`, `analysisOverlapFrameSets`, OpenAI key/model/base URL fields, and `logLevel`. Leo generates nonzero camera IDs monotonically; IDs are visible but immutable, are not reused after removal, and remain shared by preview metadata, operator state, session events, recording directories, warnings, and results. Zero cameras and any configured camera count are valid.

Each camera requires a nonblank name, an `rtsp` URL, an initial analysis-participation flag, and a positive whole-second sampling cadence. Analysis sends a positive configured number of synchronized frame sets per prompt and can repeat fewer than that number between adjacent prompts; one frame set can contain one image per participating camera. The persisted recorder timeout is the initial all-camera readiness deadline and bounded FFmpeg RTSP network I/O timeout; it must fit both FFmpeg microseconds and a Rust deadline. Reconnect delay remains one second, and graceful Stop has five seconds before forced termination. An optional provider base URL must be absolute HTTP or HTTPS. A blank provider key or model disables Analyze with sanitized guidance but leaves Monitor and completed-session discovery available.

`dataRoot: null` selects the platform default. The one effective data root contains `sessions/` and `logs/` children. Save validates the draft, creates those directories, and writes an owner-only `settings.json`.

Settings displays complete RTSP URLs for editing, but errors and logs omit them. The API key is masked by default. Save is allowed during recording or analysis. It neither interrupts active work nor changes logging, storage, cameras, preview, recorder, catalogue, or provider until restart, and changing the data root does not move existing sessions. There is no import, migration, hot reload, or automatic restart.

## Desktop Ownership

`app::launch` resolves the platform store and follows this ownership flow:

```text
platform settings.json
    |-- missing -> Dioxus shell + first-run Settings
    |-- invalid -> startup error
    `-- valid -> logging + recorder + catalogue + preview -> Dioxus shell
```

The desktop crate keeps these responsibilities in explicit modules: `desktop::bootstrap` loads
settings and prepares runtime dependencies, `desktop::launch` binds process owners to the native
event loop, `desktop::shell` installs Dioxus contexts and root-scoped tasks, and `route` defines the
route table. The `operator` module owns route-independent state plus recording and analysis task
coordination. Route views remain under `views`; the Settings view is composed from separate camera,
storage, recording, provider, application, and sidebar sections.

A missing file opens Settings with a valid zero-camera draft and does not create runtime directories. Any other settings load, validation, or directory-preparation error fails startup. Valid settings initialize compact stderr and daily JSON logging, spawn `RecorderRuntime` after its `ffmpeg`/`ffprobe` preflight, create `OperatorState` and discover completed sessions, and then start preview. Logging, recorder, or catalogue failure leaves the shell running with route-specific failure guidance and Settings reachable.

Every shell branch receives `RuntimeAvailability` and `SettingsContext`. Concrete `ResolvedSettings`, `PreviewState`, `RecorderBootstrap`, and initial operator-state contexts exist only in the ready branch; `ReadyApp` alone takes the recorder event receiver and provides `Signal<OperatorState>`. Setup, failed, and ready Settings routes therefore never require operational contexts. Missing settings initially select Settings. A runtime failure after valid settings initially selects Monitor, where the failure and a Settings link remain visible.

Zero cameras is a ready runtime: Leo skips MediaMTX, retains catalogue discovery and Analyze, omits Start from Monitor, and independently rejects an attempted empty start before creating session storage. Preview startup failure is warning-only: it produces ready operator state with recovery guidance, and recording remains available.

The desktop event-loop owner retains `RecorderRuntime`, the preview `Bridge`, and `LogGuard`. On normal loop destruction it requests recorder shutdown, interrupts in-progress readiness or finalization, stops or kills and reaps children, joins the management thread, stops and reaps MediaMTX, and then drops the log guard so queued JSON events flush.

`ReadyApp` owns the single recorder event receiver and allocates one `Signal<OperatorState>` in Dioxus's root scope above the router. Root-scoped session and analysis futures own work across awaits, so changing between Monitor and Analyze cannot cancel recording finalization or model analysis. Recorder threads communicate with UI state only through the event channel.

## Preview

The app supervises MediaMTX `v1.18.2` as a loopback-only protocol adapter:

```text
configured camera RTSP URL
    -> on-demand RTSP/TCP pull
    -> private MediaMTX camera index path
    -> 127.0.0.1:8889 WHEP
    -> WebRTC media on 127.0.0.1:8189/UDP
    -> native webview video element
```

The generated configuration disables recording and unrelated protocols and grants anonymous read access only from loopback to generated paths. Credential-bearing RTSP URLs remain in the private temporary configuration and are never sent to the webview. The webview receives only loopback WHEP and reader-script URLs.

Monitor keeps one keyed feed mounted per configured camera. It displays preview failure, analysis inclusion, and recorder status separately. Recorder states are Idle, Starting, Recording, or Reconnecting.

## Session Lifecycle

Only one recording session may run at a time.

### Start

Start creates an exclusive timestamped staging directory and one camera directory per configured ID. Every camera is sent to `RecorderRuntime`, including cameras initially excluded from analysis. One supervisor per camera starts direct FFmpeg recording equivalent to:

```text
ffmpeg -hide_banner -loglevel info
  -rtsp_transport tcp
  -timeout <persisted recorder timeout in microseconds>
  -i <configured camera URL>
  -map 0:v:0 -an -c:v copy
  -avoid_negative_ts make_zero
  -f matroska
  <camera directory>/.attempt-<uuid>.partial.mkv
```

There is no video transcoding and no audio output. Matroska accepts H.264 or H.265 stream copy and allows useful finalization after interruptions. FFmpeg progress must report at least one output frame and the partial file must be nonempty before that camera is ready. The session becomes Active only after every configured camera is ready and `SessionController` durably creates `events.jsonl` with the start event.

If any initial recorder fails or times out, startup interrupts every attempt, stops or kills and reaps all children, removes the staging directory when cleanup is sound, and returns to Idle with one shared error. It creates neither a completed event log nor a completion marker.

### Operator Actions

Participation and whole-second sampling cadence changes are appended to `events.jsonl` before UI state changes. They affect the later sampling schedule, not capture. Metadata write uncertainty faults the session and triggers recorder cleanup.

Each event includes schema version, contiguous sequence number, session UUID, UTC audit time, deterministic session-relative offset, and the operator action. The monotonic session clock determines action offsets.

| Operator intent | Durable event | Effect on recording | Effect on later analysis |
| --- | --- | --- | --- |
| Start | `SessionController::create` writes `session_started` after every recorder is ready. | All configured cameras are already recording. | Captures each camera's initial participation and cadence. |
| Include or exclude a camera | `SetCameraParticipation` writes `camera_participation_changed`. | None; the camera keeps recording. | Starts or ends that camera's sampling period at the event offset. |
| Change cadence | `SetSamplingInterval` writes `sampling_interval_changed`. | None. | While participating, samples immediately at the event offset and starts the new cadence. While excluded, the cadence applies when participation resumes. |
| Stop | `EndSession` writes `session_ended` before the app separately stops the recorder. | The action itself does not control FFmpeg. | Establishes the exclusive end of the sampling timeline. |

Session selection, route navigation, checklist edits, and Analyze do not append operator events. `SessionController::apply` flushes and synchronizes each JSONL line before returning. For participation and cadence changes, the workflow updates corresponding UI state only after that call succeeds.

### Disconnect And Reconnect

After initial readiness, an unexpected camera exit or bounded RTSP timeout is a recording gap rather than an immediate session fault:

1. Finalize any valid media from that attempt.
2. Set only that camera to Reconnecting.
3. Wait one second.
4. Start a new unique partial MKV.
5. Return to Recording when new media arrives.

Other camera supervisors continue independently, and retries continue until Stop. A failed host storage probe, child spawn, interruption, kill, reap, or finalization is fatal. A fatal event is claimed once by the root workflow, sets the canonical shared alert, attempts the end event and recorder cleanup, preserves the faulted directory, and never writes a completion marker.

### Stop

Operator Stop first attempts the durable end event and always commands recorder Stop, even if the event append failed. Each camera receives FFmpeg's graceful `q`; a child still alive after five seconds is killed and reaped. Every nonempty attempt is probed before promotion:

- exactly one video stream is required;
- container start must be finite and nonnegative;
- duration must be finite and positive;
- the segment start is estimated from first media progress and clamped after the previous segment end;
- valid media is atomically renamed to `<segment-start-UTC-ms>.mkv` without overwrite;
- empty attempts are removed and invalid nonempty attempts remain for diagnosis.

Only when the end event, all recorder cleanup, probing, and promotion succeed does Stop create `recording-complete`, return the workflow to Idle, and refresh completed sessions. Any uncertain finalization leaves the session Faulted and unavailable to Analyze.

## Portable Session Storage

All files needed to move and resume a completed session remain below one directory:

```text
<data-root>/
|-- logs/
|   `-- leo.jsonl.<date>
`-- sessions/
    `-- <start-request-UTC-ms>/
        |-- events.jsonl
        |-- recording-complete
        |-- analysis.json
        `-- recordings/
            `-- camera-<id>/
                |-- <segment-start-UTC-ms>.mkv
                |-- <later-segment-start-UTC-ms>.mkv
                `-- .attempt-<uuid>.partial.mkv
```

The `camera-<id>/` directory repeats for every camera recorded in the session.

The directory timestamp is a collision-resistant creation key; the UUID in `events.jsonl` is the durable session identity. After Stop finalization, `recording-complete` is installed without overwrite as a synchronized one-byte sentinel, the parent directory is synchronized, and the marker is truncated and synchronized to its accepted zero-byte state. `analysis.json` appears after analysis planning. An active or crashed session can lack the marker and checkpoint.

In practice, the files form a small commit protocol:

| Path | Created or changed when | Meaning |
| --- | --- | --- |
| `events.jsonl` | Created after all cameras report ready; one synchronized line is appended per action. | Authoritative session timeline. The first line must be `session_started`, sequence numbers and offsets cannot decrease, and the last action must be `session_ended`. |
| `recordings/camera-<id>/.attempt-*.partial.mkv` | Written during one FFmpeg connection attempt. | In-progress or diagnostically retained media; never consumed by analysis. |
| `recordings/camera-<id>/<UTC-ms>.mkv` | Atomically promoted after a nonempty attempt passes FFprobe validation. | Finalized media whose filename estimates its first frame's UTC time. |
| `recording-complete` | After the end event and every recorder cleanup and promotion succeed, installs a synced one-byte sentinel, syncs the directory, then truncates and syncs it to zero. | Only the final zero-byte state makes the directory visible to the session catalogue. |
| `analysis.json` | Created when a deterministic plan is built and atomically replaced after each model response. | Resumable checkpoint; absence means analysis has not started. |

`Session::load` reconstructs state by replaying `events.jsonl`; there is no separate mutable session snapshot. The catalogue requires both a valid ended log and the completion marker. Analysis then combines that replayed timeline with the finalized segments, so moving the whole completed directory does not change the plan identity.

Discovery scans direct children only and rejects symbolic links. A completed session requires both a valid ended event log and the marker. Finalized segment discovery accepts only direct regular files whose names are numeric UTC milliseconds with exact `.mkv` extension. It ignores partial and unrelated files, probes duration, rejects duplicate starts and same-camera overlap, and sorts by stable camera ID and start time.

Absolute paths are excluded from checkpoint plan identity, so a completed directory can move intact to another local filesystem path. Leo does not perform that move.

## Direct Local Analysis

Analyze operates only on a selected completed session while recording is Idle. `backend::analysis::analyze_session` performs these steps:

1. Require a nonblank checklist, direct session directory, and valid completion marker.
2. Load and validate the ended `events.jsonl`.
3. Discover and FFprobe finalized MKV segments under every camera directory.
4. Replay participation and cadence events into deterministic sample schedules.
5. Derive every uncovered camera interval as a persisted recording-gap warning.
6. Omit samples without media, retain available frames from other cameras, and continue after reconnect gaps.
7. Build configured overlapping frame-set batches and write or validate the initial zero-response `analysis.json` checkpoint.
8. Construct the provider only if an incomplete batch remains.
9. Extract requested JPEG bytes directly from local MKVs with FFmpeg, send one structured batch request, and atomically replace the checkpoint after success.
10. Emit each complete durable checkpoint snapshot to the real `OperatorState` callback.

A frame set may contain any available subset of the session cameras. An offset with no available frame is omitted. No available frames across the complete plan fails before provider construction. Invalid or overlapping segments fail without replacing a valid prior checkpoint.

The checkpoint stores schema version, session UUID, authoritative checklist, path-independent plan fingerprint, total batches, recording-gap warnings, and completed responses. Vector position is the batch number. Resume validates all plan identity and keeps prior responses. A completed checkpoint returns without provider construction.

Frame extraction removes its temporary JPEG after reading the bytes. There are no downloaded clips, temporary MP4s, or persistent JPEGs in the session tree. Provider or checkpoint failures preserve the last durable response prefix for retry.

## End-To-End Coverage

Leo has focused integration slices plus one opt-in macOS desktop E2E. That E2E is explicitly a two-fixture test scenario: it starts both fixture-camera binaries, launches the production WKWebView application entry point, drives the rendered controls from Start through Analyze, and validates the resulting durable files. It does not constrain production camera count.

| Coverage | What is real | What is substituted or absent |
| --- | --- | --- |
| Workspace and operator tests | Session persistence, state transitions, catalogue rules, sampling plans, checkpoints, and failure handling. | External processes are isolated where the test does not need media. |
| Dioxus SSR render tests | Monitor and Analyze controls plus projections of prepared operator states on both routes. | No DOM-event dispatch, native webview, browser media stack, or mouse automation; operator actions are covered separately by state and task tests. |
| Virtual-camera and recorder checks | MediaMTX, RTSP/TCP, two simultaneous readers, FFmpeg stream copy, playable MKV output, reconnect, and process cleanup. | Fixture video replaces physical cameras. |
| Local analysis check | Completed session directory, real FFprobe/FFmpeg extraction, gap-aware planning, durable callbacks, and checkpoint output. | A deterministic Rig mock replaces the model provider. |
| Full desktop E2E | Two fixture-camera processes, both MediaMTX layers, live WKWebView previews, Dioxus event handlers, FFmpeg recording, Stop finalization, session discovery, production OpenAI HTTP transport, local extraction, results UI, and shutdown. | The fixed pair is a test setup; fixture video and a loopback OpenAI-compatible server are the default, and DOM events are programmatic rather than OS pointer events. |
| Paid workflow compile check | The feature-gated application path type-checks through the real `OperatorState` callback. | It does not run or contact OpenAI without separate approval. |

The desktop E2E creates a strict owner-only temporary settings file and injects its explicit path through a feature-gated launcher; this is a test seam, not a production override. Provider variables are removed from the app child. The mounted driver reads the ready-only active `ResolvedSettings` context and permits only a numeric loopback provider or real mode with both paid gates, which keeps its safety decision aligned with the runtime actually under test.

Packaged-app pointer automation, physical-camera acceptance, and external-SSD failure handling remain separate work. The desktop E2E has a doubly gated real-OpenAI mode for manual output judgment.

## Virtual Cameras

One `camera` process represents one local fixture-backed source and supervises one RTSP-only MediaMTX child. The checked-in recipes start two independent fixture examples:

| Recipe | HTTP | RTSP | Fixture |
| --- | --- | --- | --- |
| `just camera-1` | `127.0.0.1:8080` | `127.0.0.1:8554` | `camera/fixtures/salon-1.mp4` |
| `just camera-2` | `127.0.0.1:8081` | `127.0.0.1:8555` | `camera/fixtures/salon-2.mp4` |

Each RTSP stream is available at `/axis-media/media.amp`. The HTTP process exposes health and a narrow PTZ-shaped development endpoint; PTZ commands validate inputs but do not alter fixture video. Ctrl-C, HTTP failure, or MediaMTX exit stops the process and cleans up its child and temporary configuration.

## Development Recipes

Enter `nix develop` in each of three terminals, then start the two-fixture local example:

```bash
just camera-1
just camera-2
just app
```

On first run, `just app` opens Settings. Add the two fixture RTSP URLs shown in the README, save, and restart to activate the example. `just vlc` inspects camera 1 independently. `just css` regenerates the app's checked-in Tailwind CSS and daisyUI output.

The five local media checks must be selected by these exact test names; never run a blanket ignored suite:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
nix develop --command cargo test -p backend analysis::session::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture
```

The full local desktop flow is also an exact ignored Cargo test:

```bash
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_operator_flow_records_two_cameras_and_analyzes -- --ignored --exact --nocapture --test-threads=1
```

## Paid-Test Gates

The paid application checks are absent unless Cargo feature `paid-openai-test` is enabled, remain ignored with explicit cost warnings, and assert `LEO_RUN_PAID_OPENAI_TEST=1` before constructing temporary storage, recorder runtime, `OperatorState`, session, or provider. `OPENAI_API_KEY` and `ANALYSIS_MODEL` are paid-test-process inputs only; production gets the corresponding values from Settings. The documented paid recipe rejects `OPENAI_BASE_URL` because desktop paid validation targets OpenAI directly. The focused workflow check uses one short local MKV and applies backend checkpoints through the real `OperatorState` callback.

The safe verification is compile-only:

```bash
cargo test -p app --features paid-openai-test paid_openai_workflow::paid_openai_analyzes_one_local_application_session --no-run
```

The desktop E2E uses a loopback mock unless both `LEO_E2E_REAL_OPENAI=1` and `LEO_RUN_PAID_OPENAI_TEST=1` are set. These variables are test gates, not app configuration. Mock mode disables inherited HTTP proxies so fixture images remain on loopback. The mounted driver independently permits only a numeric loopback base URL or real mode with both gates, no base override, and nonblank credentials. Real mode preserves its session for inspection under `target/desktop-e2e-real/` or `LEO_E2E_OUTPUT_DIR`. A successful run also proves the app and fixture-camera process groups have no surviving children and that recorder and preview shutdown logged success.

Do not set `LEO_RUN_PAID_OPENAI_TEST=1`, execute either paid path, or send an external provider request without separate explicit approval. Normal suites, the mock desktop E2E, and the five exact local media checks do not perform paid work.

## Logging And Sensitive Data

The app emits compact human-readable logs to stderr and daily JSON lines to `<data-root>/logs/leo.jsonl.<date>`. The persisted `LogLevel` controls both. Before valid settings are available, and whenever file logging initialization fails, Leo attempts an `info`-level or configured-level stderr fallback so the shell can still report startup failure. The retained nonblocking writer guard flushes during normal desktop shutdown.

Structured events cover settings state, preview startup, recorder attempts and cleanup, operator-state transitions, discovery skips, gap planning, checkpoint saves, and analysis completion or failure. Logs must not contain API keys, RTSP credentials or full URLs, checklists, prompts, image bytes, or model request bodies. The provider-payload and Dioxus VNode tracing targets are permanently disabled even at `trace`.

## Current Limits

- The app process owns recording. A hard crash, force quit, laptop sleep, or power loss can leave FFmpeg children or partial MKVs; orphan cleanup, a recorder daemon, active-session recovery, and recorder reattachment are not implemented.
- Normal shutdown is supervised, but recording does not survive desktop-process loss.
- A selected data root may be on an external SSD that is already mounted. Device discovery, identity checks, mounting, capacity monitoring, eject handling, and physical-SSD acceptance are not implemented.
- Parsed and filterable in-app log viewing is not implemented; see [issue #50](https://github.com/noahfraiture/leo/issues/50). Automatic log retention is not implemented; see [issue #51](https://github.com/noahfraiture/leo/issues/51). Automatic deletion, export, storage forecasting, and playback are also absent.
- Physical camera acceptance and timeout calibration remain separate hardware work. Current acceptance uses local virtual cameras and fixtures only.
- Camera discovery, keychain integration, hot apply, automatic restart, and moving or aggregating old sessions are not implemented.
- Each analysis request is fully buffered in memory, and overlap can substantially increase sequential paid image work. Analysis cannot currently be cancelled; operators should choose conservative batching and overlap values and enforce provider spend controls.
- Packaged media executables, multiple concurrent sessions or analyses, and recording cancellation are not implemented.
