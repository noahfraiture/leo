# Leo Architecture

Leo is a local, session-scoped recording and analysis application for zero or more configured RTSP cameras. The desktop process owns preview, direct host recording, operator metadata, completed-session discovery, and explicit model analysis. A session is portable under one configured data root.

The [analysis workflow](analysis-workflow.md) details exercise preparation, whole-checklist scanning, evidence review and storage. The earlier [capture architecture map](architecture-map.html) remains useful for recording ownership; use the workflow document for current analysis.

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
                                `--> confirmed exercise + optional references + one model
                                      --> overlapping image windows, whole-checklist scan
                                      --> one consolidation of cited observations
                                      --> optional independent closer look per criterion
                                      --> original evidence + corrections + human assessment
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
| `exercise` | Saves one operator-owned checklist, immutable revisions and manually imported reference originals; AI assistance produces source-linked proposals and questions, kept distinct from operator decisions. |
| `model` | Explicit provider transports, structured request contracts and scheduling under a shared memory allowance. |
| `analysis::evidence` | Reuses recorded sampling schedules, resolves source identities, extracts bounded images and seekable playback clips. |
| `analysis::process` | Bounds file-producing media conversions and owns child cleanup on completion, timeout, polling errors, and normal desktop shutdown. |
| `analysis::run` | Freezes inputs, scans each window once for all criteria, consolidates observations, optionally rechecks one episode, and saves separate operator review history. |
| `analysis::session` / `analyzer` | Historical single-model evaluator retained for reproducible prior experiments; not the desktop Analyze flow. |

`recording::recorder` remains one module despite its size. Its command actor, camera supervisors, FFmpeg attempt pump, and cleanup paths share stop and fault ownership. Splitting them would require a wider process-lifecycle API without removing state-machine complexity. About three fifths of the file is colocated concurrency and cleanup coverage.

## Application Settings

Leo owns production configuration in one strict, versioned `settings.json`. The desktop app supports macOS and Linux; Windows is not supported. Production startup reads that platform file and standard platform directories, not legacy application environment variables or the process current working directory.

| Platform | Settings file | Default data root |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Leo/settings.json` | `~/Library/Application Support/Leo/data/` |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/leo/settings.json` | `${XDG_DATA_HOME:-$HOME/.local/share}/leo/` |

Settings schema 4 uses camel-case JSON and rejects unknown fields and unsupported versions. No migration is provided. Recording settings remain independent from model configuration. It stores cameras, storage, recorder timeout, logging, monitoring profiles and named model connections/profiles/sets. Camera and monitoring IDs are nonzero, immutable and not reused. Zero cameras is valid.

Each camera has a nonblank name and an RTSP URL. Participation and monitoring cadence affect later sampling, not capture. `workflow.providers` stores names, explicit Responses or Chat Completions transport, normalized HTTP(S) addresses and private keys. Each model selects a provider, its served identifier, memory allowance in MiB, total image limit, optional output-token cap and optional reasoning effort. Preparation and analysis each select a model profile directly. Older model-set settings remain readable, but the current flow does not use their reader/reviewer roles.

Execution policy controls total local model memory, simultaneous requests, requests per model and timeout. Hosted profiles have memory cost zero but still count toward concurrency. Local model allowances are estimates supplied by the operator, not measured RAM. A local server must evict idle models when required; request scheduling does not unload them. The current analysis executes requests sequentially with one model. Evidence window span, overlap and image derivatives are frozen per run. All request image limits include reference images.

Invalid optional model configuration cannot disable recording. Each selected model resolves independently and checks its memory allowance and request policy before analysis or checklist review is offered, so an unrelated incomplete model set cannot block a usable profile. Numeric editor text is retained and validated instead of silently replacing an invalid entry with its previous value. Recorder timeout must fit FFmpeg microseconds and a Rust deadline; reconnect delay remains one second and graceful Stop five seconds.

Settings text inputs retain native typing while input events update the root-owned draft. The selected camera editor is keyed by stable camera ID, so switching cameras replaces its native fields with that camera's own values. Saves preserve later edits, and returning from another route restores the current draft.

`dataRoot: null` selects the platform default. Save validates recording fields, creates required settings and session directories, and atomically publishes synchronized owner-only settings. A root-owned worker performs storage operations while the Settings form is disabled; navigation and recording controls remain responsive. Invalid optional profile definitions can be saved with warnings so they cannot prevent correcting recording configuration. Malformed optional JSON sections load independently with a visible feature error; the original file is not rewritten on load. Diagnostic log-directory creation is best effort.

Settings displays complete RTSP URLs for editing, but errors and logs omit them. The API key is masked by default. Save is allowed during recording or analysis. Later edits remain visibly unsaved, and save completion acknowledges only the submitted snapshot. It neither interrupts active work nor changes logging, storage, cameras, preview, recorder, catalogue, or provider until restart, and changing the data root does not move existing sessions. There is no import, migration, hot reload, or automatic restart.

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
route table. The `operator` module owns recording and session selection. `exercises` owns configuration editor state; `analysis` owns run histories, execution, playback and corrections. `inference` shares admission between formatting and analysis; `media` serves registered playback assets. Route views remain under `views`; Settings groups capture, monitoring and model configuration.

A missing file opens Settings with a zero-camera draft. An unreadable file, unsupported schema, invalid recording fields, or unavailable required storage opens a recovery shell with Settings accessible. Optional metadata errors do not prevent recorder startup. Valid recording settings spawn `RecorderRuntime` after its FFmpeg/FFprobe preflight. Logging falls back to stderr and catalogue failure produces a warning while capture remains available. Preview startup remains independent.

Every shell branch receives `RuntimeAvailability` and `SettingsContext`. Concrete `ResolvedSettings`, `PreviewState`, `RecorderBootstrap`, and initial operator-state contexts exist only in the ready branch; `ReadyApp` alone takes the recorder event receiver and provides `Signal<OperatorState>`. Setup, failed, and ready Settings routes therefore never require operational contexts. Missing settings initially select Settings. A runtime failure after valid settings initially selects Monitor, where the failure and a Settings link remain visible.

Zero cameras is a ready runtime: Leo skips MediaMTX, retains catalogue discovery and Analyze, omits Start from Monitor, and independently rejects an attempted empty start before creating session storage. Preview startup failure is warning-only: it produces ready operator state with recovery guidance, and recording remains available.

The desktop event-loop owner retains `RecorderRuntime`, the preview `Bridge`, and `LogGuard`. On normal loop destruction it first closes analysis conversion admission and stops and reaps registered converter children, including conversions running on blocking workers. It then requests recorder shutdown, interrupts in-progress readiness or finalization, stops or kills and reaps children, joins the management thread, stops and reaps MediaMTX, and drops the log guard so queued JSON events flush.

`ReadyApp` owns the single recorder event receiver and allocates one `Signal<OperatorState>` in Dioxus's root scope above the router. Root-scoped session and analysis futures own work across awaits, so changing between Monitor and Analyze cannot cancel recording finalization or model analysis. Recorder threads communicate with UI state only through the event channel.

Start reserves its transition synchronously, then a worker creates exclusive staging directories before the recorder starts. The initial metadata write and failed-start directory cleanup also run on workers. Every continuation rechecks ownership before proceeding or publishing, so a late startup failure cannot delete a fault-owned folder or replace its warning.

If the recorder event channel closes, recorder availability stays failed until restart. This blocks another Start even if the app was idle or a successful Stop was still publishing its completed recording.

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

The generated configuration disables recording and unrelated protocols and grants read access only from loopback to generated paths. Each bridge startup generates a fresh preview password, held in its private temporary configuration and passed to Leo's mounted WHEP readers. Other browser pages cannot read the preview without that password. Credential-bearing RTSP URLs remain in the private configuration and are never sent to the webview. Preview access is separate from camera and provider credentials and is never persisted in settings or analysis inputs.

The MediaMTX version check has a five-second deadline and accepts at most 128 bytes of output. A stalled check is stopped and reaped, then reported as a preview warning while the recording interface remains available.

Monitor keeps one keyed feed mounted per configured camera. Each reader verifies a per-mount owner token and retains its own video element, so delayed configuration delivery, script loading and cleanup cannot claim a newer view's stream after navigation. An unmounted feed does not start a reader when loading finishes. Monitor displays preview failure, analysis inclusion, and recorder status separately. Recorder states are Idle, Starting, Recording, or Reconnecting.

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

There is no video transcoding and no audio output. Matroska accepts H.264 or H.265 stream copy and allows useful finalization after interruptions. FFmpeg progress must report at least one output frame and the partial file must be nonempty before that camera is ready. The session becomes Active after every configured camera is ready. `SessionController` then attempts to create the start event; a metadata failure produces a warning while capture continues.

If any initial recorder fails or times out, startup interrupts every attempt, stops or kills and reaps all children, removes the staging directory when cleanup is sound, and returns to Idle with one shared error. It creates neither a completed event log nor a completion marker.

### Operator Actions

Participation and monitoring profile changes affect the later sampling schedule only. Before Start they prepare initial selections. During recording, the app appends one durable event before changing visible state. A bulk assignment records all targeted camera IDs in a single event after validating the entire target set. Metadata write uncertainty leaves capture active, retains the last saved selections, disables further metadata changes, and keeps Stop usable. It does not trigger recorder cleanup.

Event schema 2 includes a contiguous sequence number, session UUID, UTC audit time, deterministic monotonic session offset, and action. The start event snapshots all monitoring definitions and each camera's initial profile and participation. Replay resolves IDs only from this immutable snapshot, never from current Settings.

| Operator intent | Durable event | Effect on recording | Effect on later analysis |
| --- | --- | --- | --- |
| Start | `SessionController::create` writes `session_started` after every recorder is ready. | All configured cameras are already recording. | Snapshots all profile definitions and each camera's initial participation and profile. |
| Include or exclude a camera | `SetCameraParticipation` writes `camera_participation_changed`. | None; the camera keeps recording. | Starts or ends that camera's sampling period at the event offset. |
| Apply monitoring profile | `SetMonitoringProfile` writes one `monitoring_profile_changed` with camera IDs and profile ID. | None. | A different profile starts a new cadence at that offset; excluded cameras use it when participation resumes. |
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

Operator Stop first attempts the durable end event and always commands recorder Stop, even if the event append failed. End-event writes, completion-marker synchronization and catalogue reads run in filesystem workers; the root-owned task awaits them in order without blocking UI navigation. Fatal cleanup also writes its end event on a worker. Each camera receives FFmpeg's graceful `q`; a child still alive after five seconds is killed and reaped. Every nonempty attempt is probed before promotion:

- exactly one video stream is required;
- container start must be finite and nonnegative;
- duration must be finite and positive;
- the segment start is estimated from first media progress and clamped after the previous segment end;
- valid media is atomically renamed to `<segment-start-UTC-ms>.mkv` without overwrite;
- empty attempts are removed and invalid nonempty attempts remain for diagnosis.

After recorder Stop, probing, and promotion succeed, the workflow returns to Idle even if the end event or completion marker failed. Such failures produce a metadata warning and keep the incomplete folder discoverable, allowing the next session. Discovery failure retains existing rows, and its error remains visible alongside any metadata warning. Stop admission invalidates older catalogue reads and reserves discovery until finalization applies its own result. Only a valid ended log plus a durable completion marker makes a session eligible for analysis. Recorder cleanup or media-finalization failures still leave the session Faulted.

## Portable Session Storage

All files needed to move and resume a completed session remain below one directory:

```text
<data-root>/
|-- logs/
|   `-- leo.jsonl.<date>
|-- exercises/<exercise-id>/
|   |-- draft.json
|   |-- revisions/<revision-id>.json
|   `-- assets/<asset-id>.<jpg|png>
`-- sessions/
    `-- <start-request-UTC-ms>/
        |-- events.jsonl
        |-- recording-complete
        |-- analyses/<run-id>/
        |   |-- inputs.json
        |   |-- checkpoint.json
        |   |-- responses/
        |   |-- references/
        |   |-- review.json
        |   |-- operator-review.json
        |   `-- cache/
        `-- recordings/
            `-- camera-<id>/
                |-- <segment-start-UTC-ms>.mkv
                |-- <later-segment-start-UTC-ms>.mkv
                `-- .attempt-<uuid>.partial.mkv
```

The `camera-<id>/` directory repeats for every camera recorded in the session.

The directory timestamp is a collision-resistant creation key; the UUID in `events.jsonl` is the durable session identity. After Stop finalization, `recording-complete` is installed without overwrite as a synchronized one-byte sentinel, the parent directory is synchronized, and the marker is truncated and synchronized to its accepted zero-byte state. A complete run directory is atomically published after analysis planning. An active or crashed recording can lack the completion marker.

Exercise writers and analysis execution/review hold exclusive file-lock guards. A guard explicitly unlocks before closing its descriptor, so a descriptor duplicated during child process creation cannot extend the writer's ownership. A competing run reports busy; lock I/O failures remain storage errors.

In practice, the files form a small commit protocol:

| Path | Created or changed when | Meaning |
| --- | --- | --- |
| `events.jsonl` | Created after all cameras report ready; one synchronized line is appended per action. | Authoritative session timeline. The first line must be `session_started`, sequence numbers and offsets cannot decrease, and the last action must be `session_ended`. |
| `recordings/camera-<id>/.attempt-*.partial.mkv` | Written during one FFmpeg connection attempt. | In-progress or diagnostically retained media; never consumed by analysis. |
| `recordings/camera-<id>/<UTC-ms>.mkv` | Atomically promoted after a nonempty attempt passes FFprobe validation. | Finalized media whose filename estimates its first frame's UTC time. |
| `recording-complete` | After the end event and every recorder cleanup and promotion succeed, installs a synced one-byte sentinel, syncs the directory, then truncates and syncs it to zero. | Only the final zero-byte state makes the directory visible to the session catalogue. |
| `analyses/<run-id>/` | Complete frozen inputs are published before execution; accepted responses precede atomic checkpoint updates. | Independent saved runs, durable job progress and separate human review history. |

`Session::load` reconstructs state by replaying `events.jsonl`; there is no separate mutable session snapshot. The catalogue requires both a valid ended log and the completion marker. Analysis then combines that replayed timeline with the finalized segments, so moving the whole completed directory does not change the plan identity.

The catalogue also returns direct recording folders with missing or invalid metadata as incomplete. Copied folders sharing a session ID are all unavailable until only one remains in the catalogue, so selection and cached analysis cannot target the wrong folder. A saved run copied under a different recording is listed as unavailable and cannot update that recording's history or another recording's cached corrections. Original files remain in place. Analyze offers folder access without treating these as analyzable sessions. Manual refresh runs in a root-owned background task, preserving later selection changes and rejecting results superseded by newer discovery. No automatic repair or crash-continuity subsystem is provided.

Discovery scans direct children only and rejects symbolic links. A completed session requires both a valid ended event log and the marker. Finalized segment discovery accepts only direct regular files whose names are numeric UTC milliseconds with exact `.mkv` extension. It ignores partial and unrelated files, probes duration, rejects duplicate starts and same-camera overlap, and sorts by stable camera ID and start time.

Absolute paths are excluded from checkpoint plan identity, so a completed directory can move intact to another local filesystem path. Leo does not perform that move.

## Checklist And Evidence Analysis

The operator prepares an exercise through four screens: source/context, reference images, clarification/final wording, and confirmation. The original checklist, model proposals, questions, operator answers and one final checklist stay separate. Source changes make old answers visibly stale; confirmation requires explicit reconciliation. A saved publication is immutable.

Library discovery isolates damaged configuration folders and keeps their paths visible for repair while healthy drafts remain available. A damaged publication does not discard its editable draft or affect other configurations; its storage warning persists through ordinary edits, and the unreadable publication is not offered for analysis. Discovery leaves retained files untouched. Folder buttons verify a direct directory and await the platform launcher asynchronously, so the launch wait does not block navigation or recording controls; the file browser can remain open after Leo closes.

Clarification rows have stable runtime identities separate from their persisted text. Queued remove, answer and source-confirmation events target those identities, so a removed row cannot retarget a neighboring question or trigger an index panic. The identities survive route changes, saves and new suggestions within the editor, and are neither saved nor sent to a provider.

Back and Continue clamp their state changes to the four preparation steps. Already queued clicks cannot underflow the first step or move beyond confirmation.

Operator text fields use native initial values while input events update their root-owned draft. Delayed host renders cannot replace more recent typing; returning to the editor restores the draft's current text. A new review explicitly replaces the displayed suggestions, and Apply replaces the final checklist fields. Add consumes pasted steps and replaces the cleared input. Runtime edit identities reject input queued from the previous fields and prevent an old Add event from consuming a newer paste buffer. Criterion identities and reference links remain stable.

`create_run` rechecks the selected recording's session identity before discovering media, rejecting a folder replaced after catalogue selection. It freezes the exercise revision, reference originals, one model, evidence options, execution limits, source hashes and all planned windows. Every overlapping window receives the complete checklist once. A text consolidation combines all cited observations into one result per criterion. There is no multi-reader vote, per-window visual reviewer, evidence selector or agent retrieval loop.

An optional independent closer look receives original images, reference pixels if enabled, and the operator's policy, without the first verdict. It is limited to one episode per criterion and must retain every original citation. Distant or over-budget evidence remains for human review. Reference pixels and automatic rechecks default off; manual rechecks share the same single allowance. Configuration ambiguity and unresolved contradictions stay Cannot verify.

Sampling preserves recorded camera participation, cadence, source identities and gaps. Missing samples cannot prove omission or hidden state. Responses are saved before checkpoints; explicit Resume reuses completed jobs and preserves failures. Older strategy runs are read-only summaries with their original files retained, not resumed under the replacement engine.

The operator can inspect original footage, save a reasoned correction, mark each current result reviewed and record an overall student assessment. Corrections never overwrite model output. A changed result makes its review and the previous overall assessment stale. The next run starts with fresh review state.

Only one human review write is admitted per run. Review decision writes temporarily disable that run's model actions, and active model work blocks its overall assessment Save before the new checkpoint arrives. Draft editing, navigation and writes to other runs remain available. The same admission checks guard queued clicks as well as the visible buttons.

Correction reasons and assessment notes retain native typing across delayed renders. A successful correction save replaces its reason field only when the submitted draft still matches; later edits remain intact. The reason field's runtime identity also includes its criterion, so selection changes restore the correct unsaved draft.

The candidate viewer creates a bounded MP4 derivative from the cited MKV segment with seeking and up to one minute of context. Repeated identical clicks share a pending preparation; another timestamp or context length starts a new request, and failed preparation can be retried. Reopening the same canonical derivative reuses its asset URL. Each accepted request gives the player a distinct identity so it resets to the selected citation and rejects older metadata callbacks. Reference examples stay separately labeled. See [analysis workflow](analysis-workflow.md) for contracts, persistence and limits.

## End-To-End Coverage

Leo has focused integration slices plus three opt-in macOS recording and analysis E2E scenarios. They use the same explicit two-fixture setup: both fixture-camera binaries and the production WKWebView application entry point. The scenarios cover the complete Start-through-Analyze flow, resuming unfinished jobs after one provider failure, and recording when preview startup fails, including mixed profiles, bulk assignment, participation changes, a forced metadata-write failure, retained playable video, and a subsequent session. The fixed setup does not constrain production camera count. Additional zero-camera checks cover library recovery alongside damaged files, queued clarification events, bounded preparation navigation, delayed typing, explicit proposal application, paste/delete/publication controls, legacy model assignments and profile persistence. They verify saved edits; only the proposal check contacts the loopback mock provider.

| Coverage | What is real | What is substituted or absent |
| --- | --- | --- |
| Workspace and operator tests | Session persistence, state transitions, catalogue rules, sampling plans, checkpoints, and failure handling. | External processes are isolated where the test does not need media. |
| Dioxus SSR render tests | Monitor and Analyze controls plus projections of prepared operator states on both routes. | No DOM-event dispatch, native webview, browser media stack, or mouse automation; operator actions are covered separately by state and task tests. |
| Virtual-camera and recorder checks | MediaMTX, RTSP/TCP, two simultaneous readers, FFmpeg stream copy, playable MKV output, reconnect, and process cleanup. | Fixture video replaces physical cameras. |
| Local analysis check | Completed session directory, real FFprobe/FFmpeg extraction, gap-aware planning, durable callbacks, and checkpoint output. | A deterministic Rig mock replaces the model provider. |
| Desktop E2E scenarios | Two fixture-camera processes, WKWebView and Dioxus event handlers, FFmpeg recording, Stop finalization, session discovery, local extraction, production OpenAI HTTP transport, checkpoint recovery, degraded preview handling, results UI, and shutdown. | The fixed pair is a test setup; fixture video and a loopback OpenAI-compatible server are the default, and DOM events are programmatic rather than OS pointer events. |
| Paid evaluation compile check | The historical evaluator type-checks through the backend checkpoint API. | It does not run or contact OpenAI without separate approval. |

The desktop E2E creates a strict owner-only temporary settings file and injects its explicit path through a feature-gated launcher; this is a test seam, not a production override. Provider variables are removed from the app child. The mounted driver reads the ready-only active `ResolvedSettings` context and permits only a numeric loopback provider or real mode with both paid gates, which keeps its safety decision aligned with the runtime actually under test.

Before driving a scenario, the native driver also runs the production preview-reader program against controlled video elements and a reader double. This checks delayed unmount/remount ownership and normal attachment/cleanup without opening an extra connection; it is separate from the scenario's real WebRTC preview checks.

A separate local review-text scenario reuses recording and analysis setup to check delayed typing, correction consumption, saved notes and draft restoration across runs. It omits playback operations, allowing these checks when the platform rejects scripted playback. The complete scenario still requires playback, seeking and decoded pixels.

Packaged-app pointer automation and physical-camera acceptance remain separate work. External-storage loss has an optional manual drill in the [validation checklist](validation.md); hot recovery is not implemented. The complete desktop scenario has a doubly gated real-OpenAI mode for manual output judgment.

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

The six local media checks must be selected by these exact test names; never run a blanket ignored suite:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
nix develop --command cargo test -p backend analysis::session::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_starts_with_low_bitrate_video -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture
```

The local desktop scenarios are also exact ignored Cargo tests:

```bash
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_operator_flow_records_two_cameras_and_analyzes -- --ignored --exact --nocapture --test-threads=1
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_analysis_resumes_after_transient_provider_failure -- --ignored --exact --nocapture --test-threads=1
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e desktop_recording_remains_usable_without_preview -- --ignored --exact --nocapture --test-threads=1
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e library::desktop_exercise_library_keeps_healthy_configurations_available -- --ignored --exact --nocapture --test-threads=1
LEO_E2E_REAL_OPENAI=0 LEO_RUN_PAID_OPENAI_TEST=0 cargo test -p camera --features desktop-e2e --test desktop_e2e questions::desktop_question_events_keep_the_remaining_answer_intact -- --ignored --exact --nocapture --test-threads=1
```

## Paid Evaluation Gates

The paid application evaluations are absent unless Cargo feature `paid-openai-evaluations` is enabled, remain ignored with explicit cost warnings, and assert `LEO_RUN_PAID_OPENAI_TEST=1` before constructing temporary storage, session, or provider. `OPENAI_API_KEY` and `ANALYSIS_MODEL` are evaluation-process inputs only; production gets the corresponding values from Settings. The documented paid recipe rejects `OPENAI_BASE_URL` because desktop paid validation targets OpenAI directly. The historical checkpoint evaluation uses one short local MKV and calls the preserved backend evaluator directly. The current desktop workflow is covered by the run coordinator and desktop scenarios.

The safe verification is compile-only:

```bash
cargo test -p app --features paid-openai-evaluations openai_evaluations::natural_fixture_exercises_application_checkpoint_flow --no-run
```

The complete desktop scenario uses a loopback mock unless both `LEO_E2E_REAL_OPENAI=1` and `LEO_RUN_PAID_OPENAI_TEST=1` are set; the recovery scenario always uses a loopback failure, and the preview-degraded scenario makes no provider request. These variables are test gates, not app configuration. Mock mode disables inherited HTTP proxies so fixture images remain on loopback. The mounted driver independently permits only a numeric loopback base URL or real mode with both gates, no base override, and nonblank credentials. Real mode preserves its session for inspection under `target/desktop-e2e-real/` or `LEO_E2E_OUTPUT_DIR`. Every successful scenario proves the app and fixture-camera process groups have no surviving children. The preview-enabled scenarios also prove clean preview shutdown; the degraded scenario proves recording completes without a preview bridge.

Do not set `LEO_RUN_PAID_OPENAI_TEST=1`, execute either paid path, or send an external provider request without separate explicit approval. Normal suites, the mock desktop E2E, and the six exact local media checks do not perform paid work.

## Logging And Sensitive Data

The app emits compact human-readable logs to stderr and daily JSON lines to `<data-root>/logs/leo.jsonl.<date>`. The persisted `LogLevel` controls both. Before valid settings are available, and whenever file logging initialization fails, Leo attempts an `info`-level or configured-level stderr fallback so the shell can still report startup failure. The retained nonblocking writer guard flushes during normal desktop shutdown.

Structured events cover settings state, preview startup, recorder attempts and cleanup, operator-state transitions, discovery skips, gap planning, checkpoint saves, and analysis completion or failure. Logs must not contain API keys, RTSP credentials or full URLs, checklists, prompts, image bytes, or model request bodies. The provider-payload and Dioxus VNode tracing targets are permanently disabled even at `trace`.

## Current Limits

- The app process owns recording. A hard crash, force quit, laptop sleep, or power loss can leave FFmpeg children or partial MKVs; orphan cleanup, a recorder daemon, active-session recovery, and recorder reattachment are not implemented.
- Normal shutdown is supervised, but recording does not survive desktop-process loss.
- A selected data root may be on an external SSD that is already mounted. Device discovery, identity checks, mounting, capacity monitoring, eject handling, and physical-SSD acceptance are not implemented.
- Parsed and filterable in-app log viewing is not implemented; see [issue #50](https://github.com/noahfraiture/leo/issues/50). Automatic log retention is not implemented; see [issue #51](https://github.com/noahfraiture/leo/issues/51). Automatic deletion, export and storage forecasting are absent. Candidate evidence playback is available through bounded derived clips.
- Physical camera acceptance and timeout calibration remain separate hardware work. Current acceptance uses local virtual cameras and fixtures only.
- Camera discovery, keychain integration, hot apply, automatic restart, and moving or aggregating old sessions are not implemented.
- A bounded request is buffered in memory. Each eligible temporal window is scanned once for the entire checklist. Long recordings can still create large consolidations; provider context failures pause explicitly without truncation or fallback. The memory allowance estimates model residency and cannot force external-server eviction. Analysis has durable resume but no explicit Cancel control.
- Packaged media executables, multiple concurrent sessions or analyses, and recording cancellation are not implemented.
