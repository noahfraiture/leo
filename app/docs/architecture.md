# Leo Architecture

Leo is a local, session-scoped two-camera recording and analysis application. The desktop process owns preview, direct host recording, operator metadata, completed-session discovery, and explicit model analysis. A session is portable under one configured data root.

## Workspace

The Cargo workspace contains exactly three crates:

| Crate | Type | Responsibility |
| --- | --- | --- |
| `app` | Library plus desktop binary | Dioxus routes and views, startup configuration, preview bridge, shared workflow state, logging, and root-scoped tasks. |
| `backend` | Library | Durable session events, FFmpeg recorder supervision, finalized segment discovery, sampling plans, gap warnings, frame extraction, provider transport, and checkpoints. |
| `camera` | Binary | One fixture-backed Axis-shaped virtual camera per process for local development and acceptance. |

The desktop crate depends on `backend`. The virtual camera is a separate local process and communicates only through HTTP and RTSP sockets.

## End-To-End Flow

```text
camera 1 RTSP ------------------+--> app MediaMTX --> WHEP/WebRTC --> preview 1
                                |
                                `--> FFmpeg recorder 1 --> camera-1/*.mkv

camera 2 RTSP ------------------+--> app MediaMTX --> WHEP/WebRTC --> preview 2
                                |
                                `--> FFmpeg recorder 2 --> camera-2/*.mkv

events.jsonl + finalized MKVs + recording-complete
                                |
                                `--> local FFprobe/FFmpeg analysis plan
                                      --> explicit provider batches
                                      --> analysis.json
```

Preview and recording make independent RTSP/TCP connections. Preview availability does not establish recording health, and a failed preview does not stop a recorder. Camera participation and cadence are analysis settings only: an excluded camera remains visible and records for the complete active session.

## Runtime Configuration

Configuration is loaded before the desktop UI starts. Relative defaults are resolved from the app process working directory.

| Variable | Default | Meaning |
| --- | --- | --- |
| `LEO_CAMERA_CONFIG` | `./cameras.json` | JSON deployment configuration for exactly two cameras. |
| `LEO_DATA_DIR` | `./data` | Parent of `sessions/` and `logs/`. |
| `LEO_RECORDER_TIMEOUT_SECS` | `10` | Initial all-camera readiness deadline and bounded FFmpeg RTSP network I/O timeout. |
| `OPENAI_API_KEY` | none | Required only for an explicit provider analysis. |
| `ANALYSIS_MODEL` | none | Required provider model name; the app has no hard-coded model. |
| `OPENAI_BASE_URL` | provider default | Optional endpoint override read by the provider client. |
| `RUST_LOG` | `info` | Filter for compact console and JSON file logging. |

The camera file must contain exactly two rows. IDs must be unique and nonzero; names and URLs must be nonblank; URLs must use the `rtsp` scheme; and `sampleEveryMs` must be a positive whole number of seconds. Stable camera IDs are shared by preview metadata, workflow state, session events, recording directories, warnings, and results.

`LEO_DATA_DIR`, `sessions/`, and `logs/` must be direct directories and not symbolic links. The app creates missing directories. The recorder timeout must be a positive integer that is representable both as FFmpeg microseconds and as a Rust deadline. Reconnect delay is fixed at one second, and graceful Stop has five seconds before forced termination.

Missing `OPENAI_API_KEY` or `ANALYSIS_MODEL` disables the Analyze action with a sanitized message. Selection, refresh, route navigation, and checklist edits never construct a provider or send a request. `OPENAI_BASE_URL` is consumed only when explicit analysis constructs the provider.

## Desktop Ownership

`app::launch` establishes process-level ownership in this order:

1. Load and validate camera and data configuration.
2. Install compact stderr logging and a nonblocking daily JSON appender.
3. Spawn `RecorderRuntime`, which preflights `ffmpeg` and `ffprobe` and owns its management thread.
4. Create `Workflow` and discover completed sessions.
5. Start the app-owned MediaMTX preview bridge.
6. Launch Dioxus with the validated shared state.

Configuration, logging, recorder preflight, or workflow discovery failure produces one unavailable UI with no Start control. Preview startup failure is independent: the recording workflow remains available and the Monitor route shows preview recovery guidance.

The desktop event-loop owner retains `RecorderRuntime`, the preview `Bridge`, and `LogGuard`. On normal loop destruction it requests recorder shutdown, interrupts in-progress readiness or finalization, stops or kills and reaps children, joins the management thread, stops and reaps MediaMTX, and then drops the log guard so queued JSON events flush.

`ReadyApp` owns the single recorder event receiver and one `Signal<Workflow>` above the router. Root-scoped session and analysis futures own work across awaits, so changing between Monitor and Analyze cannot cancel recording finalization or model analysis. Recorder threads communicate with UI state only through the event channel.

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

Monitor keeps exactly two keyed feeds mounted. It displays preview failure, analysis inclusion, and recorder status separately. Recorder states are Idle, Starting, Recording, or Reconnecting.

## Session Workflow

Only one recording session may run at a time.

### Start

Start creates an exclusive timestamped staging directory and one camera directory per configured ID. Every camera is sent to `RecorderRuntime`, including cameras initially excluded from analysis. One supervisor per camera starts direct FFmpeg recording equivalent to:

```text
ffmpeg -hide_banner -loglevel info
  -rtsp_transport tcp
  -timeout <LEO_RECORDER_TIMEOUT_SECS in microseconds>
  -i <configured camera URL>
  -map 0:v:0 -an -c:v copy
  -avoid_negative_ts make_zero
  -f matroska
  <camera directory>/.attempt-<uuid>.partial.mkv
```

There is no video transcoding and no audio output. Matroska accepts H.264 or H.265 stream copy and allows useful finalization after interruptions. FFmpeg progress must report at least one output frame and the partial file must be nonempty before that camera is ready. The session becomes Active only after every configured camera is ready and `SessionController` durably creates `events.jsonl` with the start event.

If any initial recorder fails or times out, startup interrupts every attempt, stops or kills and reaps all children, removes the staging directory when cleanup is sound, and returns to Idle with one shared error. It creates neither a completed event log nor a completion marker.

### Active Changes

Participation and whole-second sampling cadence changes are appended to `events.jsonl` before UI state changes. They affect the later sampling schedule, not capture. Metadata write uncertainty faults the session and triggers recorder cleanup.

Each event includes schema version, contiguous sequence number, session UUID, UTC audit time, deterministic session-relative offset, and the operator action. The monotonic session clock determines action offsets.

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
<LEO_DATA_DIR>/
|-- logs/
|   `-- leo.jsonl.<date>
`-- sessions/
    `-- <start-request-UTC-ms>/
        |-- events.jsonl
        |-- recording-complete
        |-- analysis.json
        `-- recordings/
            |-- camera-1/
            |   |-- <segment-start-UTC-ms>.mkv
            |   |-- <later-segment-start-UTC-ms>.mkv
            |   `-- .attempt-<uuid>.partial.mkv
            `-- camera-2/
                `-- <segment-start-UTC-ms>.mkv
```

The directory timestamp is a collision-resistant creation key; the UUID in `events.jsonl` is the durable session identity. `recording-complete` is exactly a zero-byte regular file created atomically after Stop finalization. `analysis.json` appears after analysis planning. An active or crashed session can lack the marker and checkpoint.

Discovery scans direct children only and rejects symbolic links. A completed session requires both a valid ended event log and the marker. Finalized segment discovery accepts only direct regular files whose names are numeric UTC milliseconds with exact `.mkv` extension. It ignores partial and unrelated files, probes duration, rejects duplicate starts and same-camera overlap, and sorts by stable camera ID and start time.

Absolute paths are excluded from checkpoint plan identity, so a completed directory can move intact to another local filesystem path. Leo does not perform that move.

## Direct Local Analysis

Analyze operates only on a selected completed session while recording is Idle. `backend::analysis::analyze_session` performs these steps:

1. Require a nonblank checklist, direct session directory, and valid completion marker.
2. Load and validate the ended `events.jsonl`.
3. Discover and FFprobe finalized MKV segments under every camera directory.
4. Replay participation and cadence events into deterministic sample schedules.
5. Derive every uncovered camera interval as a persisted recording-gap warning.
6. Omit samples without media, retain available frames from the other camera, and continue after reconnect gaps.
7. Build five-frame-set batches and write or validate the initial zero-response `analysis.json` checkpoint.
8. Construct the provider only if an incomplete batch remains.
9. Extract requested JPEG bytes directly from local MKVs with FFmpeg, send one structured batch request, and atomically replace the checkpoint after success.
10. Emit each complete durable checkpoint snapshot to the real Workflow callback.

A frame set may contain one or both cameras. An offset with no available frame is omitted. No available frames across the complete plan fails before provider construction. Invalid or overlapping segments fail without replacing a valid prior checkpoint.

The checkpoint stores schema version, session UUID, authoritative checklist, path-independent plan fingerprint, total batches, recording-gap warnings, and completed responses. Vector position is the batch number. Resume validates all plan identity and keeps prior responses. A completed checkpoint returns without provider construction.

Frame extraction removes its temporary JPEG after reading the bytes. There are no downloaded clips, temporary MP4s, or persistent JPEGs in the session tree. Provider or checkpoint failures preserve the last durable response prefix for retry.

## Virtual Cameras

One `camera` process represents one local fixture-backed source and supervises one RTSP-only MediaMTX child. The checked-in recipes start:

| Recipe | HTTP | RTSP | Fixture |
| --- | --- | --- | --- |
| `just camera-1` | `127.0.0.1:8080` | `127.0.0.1:8554` | `camera/fixtures/salon-1.mp4` |
| `just camera-2` | `127.0.0.1:8081` | `127.0.0.1:8555` | `camera/fixtures/salon-2.mp4` |

Each RTSP stream is available at `/axis-media/media.amp`. The HTTP process exposes health and a narrow PTZ-shaped development endpoint; PTZ commands validate inputs but do not alter fixture video. Ctrl-C, HTTP failure, or MediaMTX exit stops the process and cleans up its child and temporary configuration.

## Development Recipes

Enter `nix develop` in each of three terminals, then start the complete local desktop workflow:

```bash
just camera-1
just camera-2
just app
```

`just vlc` inspects camera 1 independently. `just css` regenerates the app's checked-in Tailwind CSS and daisyUI output.

The five local media checks must be selected by these exact test names; never run a blanket ignored suite:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
nix develop --command cargo test -p backend analysis::facade::tests::full_local_ffmpeg_and_mock_model_analysis_uses_pre_and_post_gap_segments -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream fixture_streams_h264_to_two_readers_and_stops_cleanly -- --ignored --exact
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_records_playable_mkv -- --ignored --exact --nocapture
nix develop --command cargo test -p camera --test rtsp_stream host_recorder_reconnects_into_a_second_segment -- --ignored --exact --nocapture
```

The last four commands are also exposed as `just test-local-analysis`, `just test-camera-stream`, `just test-host-recording`, and `just test-host-reconnect`.

## Paid-Test Gates

The only paid workflow test is absent unless Cargo feature `paid-openai-test` is enabled, remains ignored with an explicit cost warning, and begins with an assertion requiring `LEO_RUN_PAID_OPENAI_TEST=1` before constructing temporary storage, recorder runtime, Workflow, session, or provider. It uses one short local MKV and applies backend checkpoints through the real Workflow callback.

The safe verification is compile-only:

```bash
cargo test -p app --features paid-openai-test paid_openai_workflow::paid_openai_analyzes_one_local_application_session --no-run
```

Do not set `LEO_RUN_PAID_OPENAI_TEST=1`, execute the paid test, or send an external provider request without separate explicit approval. Normal suites and the five exact local media checks do not perform paid work.

## Logging And Sensitive Data

The app emits compact human-readable logs to stderr and daily JSON lines to `<LEO_DATA_DIR>/logs/leo.jsonl.<date>`. `RUST_LOG` controls both with `info` as the fallback. The retained nonblocking writer guard flushes during normal desktop shutdown.

Structured events cover configuration, preview startup, recorder attempts and cleanup, workflow transitions, discovery skips, gap planning, checkpoint saves, and analysis completion or failure. Logs must not contain API keys, RTSP credentials or full URLs, checklists, prompts, image bytes, or model request bodies.

## Current Limits

- The app process owns recording. A hard crash, force quit, laptop sleep, or power loss can leave FFmpeg children or partial MKVs; orphan cleanup, a recorder daemon, active-session recovery, and recorder reattachment are not implemented.
- Normal shutdown is supervised, but recording does not survive desktop-process loss.
- `LEO_DATA_DIR` may point to an external SSD that is already mounted. Device discovery, identity checks, mounting, capacity monitoring, eject handling, and physical-SSD acceptance are not implemented.
- Automatic retention, rotation, deletion, export, storage forecasting, and playback are not implemented.
- Physical camera acceptance and timeout calibration remain separate hardware work. Current acceptance uses local virtual cameras and fixtures only.
- Camera discovery, Settings UI, packaged media executables, multiple concurrent sessions or analyses, and analysis cancellation are not implemented.
