# Leo architecture

Leo is a session-oriented system for recording and analyzing student exercise with several cameras. It is designed to operate locally, keep archival recording independent from the operator laptop, and provide synchronized video and metadata to an analysis pipeline.

This document defines the domain model, component responsibilities and intended workflows. Where a workflow is incomplete, that status is stated in the section that owns it rather than separated into a legacy or future architecture.

## Vocabulary

These distinctions are part of the domain model and should be reflected in code and data structures.

| Term | Definition |
| --- | --- |
| **Session** | Software-defined exercise interval aligned with continuously recorded camera media. |
| **Recording** | Raw camera media continuously archived and catalogued by Surveillance Station. |
| **Video** | One catalogued recording segment with camera and UTC bounds used for sampling. |
| **Video stream** | Continuous encoded video data. |
| **Frame** | One decoded image. |
| **Frame rate** | Number of frames produced per second. |
| **Sampling rate** | Number of frames selected per second. |
| **Sampling schedule** | Sampling rate changes over time. |
| **Sample** | A selected frame. |
| **Sample sequence** | Ordered samples for one camera across its catalogued recording segments. |
| **Frame index** | Position of a frame in a recording. |
| **Sample index** | Position of a sample in a sample sequence. |
| **Frame timestamp** | Frame position on the session timeline. |
| **Frame set** | Available camera samples associated with the same session timestamp. |
| **Frame batch** | Ordered frame sets covering a bounded time range. |
| **Session sequence** | Ordered frame sets covering the full session. |
| **Preview** | Live stream shown to the operator; it is not the archival recording. |
| **Virtual camera** | The `camera` process used in development instead of a physical Axis camera. |
| **Synology simulator** | The `synology` process that implements a narrow subset of the Surveillance Station API. |
| **MediaMTX** | External media server supervised by the virtual camera and desktop app for different purposes. |
| **RTSP** | Camera-facing protocol used to transport encoded video streams. |
| **WHEP/WebRTC** | Loopback protocol used to deliver previews to the desktop webview. |

## Goals

The deployment is expected to use approximately three to five AXIS P3278-LV cameras. The system must:

- record locally without depending on the internet
- record reliably for an entire training day
- continue recording if the operator laptop crashes or sleeps
- allow an operator to monitor all cameras
- support camera selection, digital zoom, quality profiles and annotations
- provide live frames to an AI analysis pipeline
- retain original high-quality recordings
- detect failures and recording gaps quickly
- make recordings accessible programmatically
- require little or no manual post-production
- allow exported or processed data to be taken home

This is not a traditional surveillance deployment. Surveillance Station provides reliable recording infrastructure, while Leo adds session workflow, metadata and analysis.

## System architecture

The same responsibility boundaries apply to physical deployment and local development. Physical Axis and Synology services are represented during development by the `camera` and `synology` binaries.

```text
Camera source
(Axis camera or virtual camera)
|
|-- high-quality stream --> Synology Surveillance Station --> archival recordings
|                            | (continuous on physical hardware;
|                            |  fixture-backed in the simulator)
|                            `-- catalogue/download --> app analysis pipeline
|                                                         (backend implemented; UI not wired)
|
|-- preview RTSP ---------> app-owned MediaMTX --> WHEP/WebRTC --> operator UI
|                            (implemented)
|
`-- VAPIX API <----------> operator app
                             (virtual API exists; app client is not implemented)
```

The Cargo workspace contains three independent Rust binaries:

| Component | Responsibility |
| --- | --- |
| `app` | Dioxus desktop UI, preview bridge, JSONL session metadata, Synology recording access and offline analysis orchestration. Only preview is wired to the UI. |
| `camera` | Development replacement for one Axis camera, with a VAPIX-shaped HTTP API and fixture-backed RTSP stream. |
| `synology` | Development replacement for a small Surveillance Station API surface, with in-memory camera/control state and a fixture-backed recording catalogue. |

Each binary runs as a separate process and communicates over sockets. They do not share memory or a database. The running UI currently consumes only camera RTSP; the session, Synology and analysis backends are implemented but are not wired to the Analyze route, and the app does not yet call VAPIX.

## Ownership boundaries

The boundaries prevent two systems from trying to own the same recording concern.

### Surveillance Station owns

- archival recording
- continuous recording execution independent of session actions
- archival stream configuration
- reconnection after camera or network interruptions
- recording catalogue, media download and health monitoring
- recording recovery
- storage rotation and retention enforcement

The `synology` crate does not implement those storage concerns. It simulates API discovery, camera listing, recording-control state, and fixture-backed recording catalogue and download responses.

### Operator app owns

- session workflow and naming
- operator interface
- durable JSONL session events
- software-only camera participation and sampling schedules
- low-resolution previews
- analysis planning, orchestration and checkpoints
- temporary batch-local processing media
- timestamped custom metadata
- supported PTZ or digital-view controls
- operator presets
- operator retention, export and deletion decisions
- downstream processing status
- alarms presented to the operator

Only live preview and its failure presentation are currently connected to the UI. The session and analysis backends are not yet connected to operator controls.

### Axis VAPIX provides

- camera status and capabilities
- separate preview and analysis streams
- supported PTZ controls
- stream profiles
- snapshots
- overlays
- camera-side events
- advanced settings that Surveillance Station does not expose

The app must not become a second archival recorder through VAPIX. It accesses media through supported Surveillance Station catalogue and download APIs rather than internal NAS files. Surveillance Station maintains continuous recording, storage, catalogue and retention execution; the app records operator decisions and orchestrates downstream analysis.

## Desktop app

The `app` crate is the main operator-facing component. It uses Dioxus Desktop for routing and UI, and supervises a local MediaMTX process to adapt camera RTSP streams into WebRTC streams the embedded webview can play.

### Startup and shutdown

`app/src/main.rs` currently defines one development camera named `Workshop` at `rtsp://127.0.0.1:8554/axis-media/media.amp`. Persisted camera configuration and discovery have not replaced this source yet.

Before launching Dioxus, the app starts the preview bridge:

1. Reserve TCP port `8889` and UDP port `8189` so occupied ports fail early.
2. Verify that `mediamtx` is on `PATH` and reports exactly `v1.18.2`.
3. Generate a temporary MediaMTX configuration with anonymous, read-only access limited to loopback and the generated camera paths.
4. Start MediaMTX and wait up to five seconds for a WHEP readiness response.
5. Convert camera sources into browser-safe `PreviewState` metadata.

Bridge startup failure does not terminate the UI. The app provides `PreviewState::Unavailable` to Dioxus and the Monitor route displays the error with recovery guidance. A camera may still be unavailable after the bridge starts because RTSP sources are pulled on demand; that failure is reported by the individual feed.

The Dioxus event-loop handler owns normal shutdown. When the loop is destroyed, it kills and reaps MediaMTX. `Bridge::drop` provides fallback cleanup, and the temporary configuration exists only while the bridge owns it.

After readiness, the bridge retains the MediaMTX child for cleanup but does not monitor or restart it. An unexpected MediaMTX exit does not currently change `PreviewState`; its effect appears at the affected feeds.

### Preview data flow

The app-owned MediaMTX process is a loopback-only protocol adapter:

```text
camera RTSP URL
    -> on-demand RTSP/TCP pull
    -> app-owned MediaMTX path (`camera-<index>`)
    -> anonymous loopback WHEP endpoint
    -> MediaMTX `reader.js`
    -> WebRTC MediaStream
    -> Dioxus `<video>` element
```

For each feed, `CameraFeed` loads `reader.js` and starts a JavaScript `MediaMTXWebRTCReader` through Dioxus `document::Eval`. Rust sends feed configuration into the evaluator. JavaScript sends connection errors or a successful-track signal back to Rust, then waits for a shutdown message. Component teardown closes the reader and clears the video's `srcObject`.

RTSP URLs remain in the MediaMTX configuration and are not sent to the webview. The webview receives only loopback WHEP and reader script URLs.

The generated MediaMTX configuration:

- binds WHEP HTTP and WebRTC media to loopback
- grants anonymous read access only from loopback and only to generated camera paths
- stores configuration in a temporary file with mode `0600` on Unix
- pulls RTSP over TCP only when a viewer requests the path
- disables recording, RTSP serving, RTMP, HLS, SRT, metrics, pprof, playback and the MediaMTX API

This limits the local bridge boundary to the operator laptop. Between releasing the reserved ports and MediaMTX binding them, another compatible MediaMTX process could rarely win the handoff and cause a recoverable failed preview; stop the conflicting process and restart the app.

### Operator interface and session workflow

The app has two routes under a shared layout:

| Route | Component | State |
| --- | --- | --- |
| `/` | `Monitor` | Renders available preview feeds or bridge startup guidance. |
| `/analyze` | `Analyze` | Placeholder UI. |

The intended operator workflow includes:

- a master session control that starts and ends the software session clock and event log
- live views for all cameras
- per-camera recording and health status
- warnings with suggested operator actions
- per-camera software participation, sampling rate and digital zoom controls
- timestamped notes and bookmarks
- playback of saved videos where needed
- an action to discard a session recording

Physical cameras and Surveillance Station continue recording regardless of the master session action or participation events. Only the live preview grid is functional in the UI today; camera status badges, timestamps, selection labels, settings and route sidebars are static presentation.

### Session metadata

The app persists session metadata as one newline-terminated `events.jsonl` file. Each ordered event contains a schema version, sequence number, session ID, UTC audit timestamp, deterministic session-relative offset and one action. The implemented actions are session start with the initial camera configuration, camera participation changes, sampling-interval changes and session end.

Participation and interval events affect only software sampling. They never start or stop physical recording. The backend implements durable event writes and completed-session replay; UI controls and reopening an active session after application restart remain deferred. Analysis stores its separate `analysis.json` checkpoint beside the event log.

### Retention and export

The session workflow must:

- align catalogued recording segments and metadata by session time
- keep metadata aligned with the session timeline
- warn when storage is insufficient and propose an operator action
- request supported media downloads for manual or offline processing
- allow the operator to discard a session recording deliberately

Surveillance Station owns the recordings, storage, catalogue, download service and retention execution. The app records the operator's decision and tracks its result. The retention UI workflow is not implemented yet.

### Analysis pipeline

The backend analysis pipeline preserves the distinction between catalogue segments, planned samples and extracted frames:

1. Load the completed session event log and replay each camera's software sampling schedule.
2. List Surveillance Station catalogue segments intersecting the complete session interval.
3. Match every planned sample to exactly one segment, build per-camera sequences and merge them into chronological frame sets.
4. Divide the frame sets into fixed-size batches and resume after the completed checkpoint prefix.
5. For only the current batch, merge required windows per segment and download them into a temporary directory.
6. Extract temporary JPEGs at the planned offsets with FFmpeg and append them directly to the Agent prompt in canonical order.
7. Call the Agent with the checklist and previous complete response, then atomically replace `analysis.json` after success.

Downloaded clips are batch-scoped, and each extracted JPEG file is removed after its bytes are read, so temporary media disappears on every success or failure path. Cross-batch video caching is intentionally absent. If transfer becomes a measured bottleneck, extraction may move to a NAS-side FFmpeg or frame-extraction service. The backend pipeline is implemented but is not wired to the Dioxus Analyze route.

Live analysis may consume a dedicated low-resolution camera stream. Offline analysis should use retained recordings so temporary processing failures can be retried without losing frames. Polling VAPIX JPEG snapshots is simpler, but delays produce irregular sampling and missed frames cannot be recovered; it is not equivalent to decoding a recording according to a sampling schedule.

### External integrations

The implemented Synology client uses the supported API to:

- open one explicit optional SID login session
- list Recording API version 5 catalogue entries from `data.events[]` with `id`, `cameraId`, `startTime` and `stopTime`
- download bounded recording-relative media ranges with Recording API version 6 into atomically replaced local files

The client currently relies on `List` v5 timestamps and `Download` v6. Optional `List` v6 metadata and composite `(dsId, cameraId, id)` correlation remain deferred until physical NAS responses require them.

It will use Axis VAPIX to:

- consume RTSP H.264 or H.265 streams for previews and live analysis
- query camera status and capabilities
- control PTZ or digital cropping where supported
- request snapshots
- select preview or analysis stream profiles

The Synology recording client is implemented for backend use but is not invoked by the current UI. The Axis client is not implemented. The development Synology simulator provides fixture-backed `List` v5/v6 and `Download` v6 responses but does not create or persist recordings.

### Code boundaries

| Path | Responsibility |
| --- | --- |
| `app/src/main.rs` | Routes, preview startup, Dioxus context and MediaMTX cleanup. |
| `app/src/preview/bridge.rs` | MediaMTX version check, port reservation, readiness, metadata and child lifecycle. |
| `app/src/preview/config.rs` | Temporary loopback MediaMTX configuration. |
| `app/src/preview/error.rs` | Preview startup and lifecycle errors. |
| `app/src/components/camera/feed.rs` | Video card and Rust-to-JavaScript WHEP reader lifecycle. |
| `app/src/views/monitor/` | Preview grid and unavailable-state guidance. |
| `app/src/views/analyze/` | Analysis route placeholder. |
| `app/src/views/navbar.rs` | Shared navigation, sidebar and route body layout. |
| `app/src/session/` | Durable JSONL session events, completed-session replay and software sampling actions. |
| `app/src/recording/` | Supported Surveillance Station catalogue, login and media download client. |
| `app/src/analysis/video/` | Sampling schedules, catalogue-backed sequences, frame sets and FFmpeg extraction. |
| `app/src/analysis/agent/` | Stateless structured model request transport. |
| `app/src/analysis/analyzer/` | Batch-local media materialization, prompt construction and atomic resumable checkpoints. |

## Virtual camera

The `camera` crate is a local Axis-shaped service for development and integration tests. One process represents one camera and supervises one MediaMTX child.

At startup it:

1. Parses the HTTP address, RTSP address and fixture path.
2. Validates and canonicalizes the fixture path.
3. Creates an RTSP-only MediaMTX configuration and starts the child process.
4. Waits up to five seconds for the RTSP TCP listener.
5. Starts the Axum HTTP API only after RTSP is reachable.

The process exits when it receives Ctrl-C, the HTTP server fails, or MediaMTX exits. It then stops MediaMTX and removes the temporary configuration. MediaMTX is not restarted automatically.

The HTTP service exposes:

- `GET /health`, which reports only that the Axum service is running
- `GET /axis-cgi/com/ptz.cgi`, a small VAPIX-compatible PTZ surface

The PTZ surface validates relative pan and tilt values from `-360` to `360` for camera channel `1`. It does not persist position or alter the video. VAPIX command failures use VAPIX-style text responses, generally with HTTP `200 OK`.

The camera-owned MediaMTX serves the supplied fixture at `rtsp://<rtsp-address>/axis-media/media.amp`. The service is anonymous, read-only and TCP-only. Readiness verifies the listener, not that media can be decoded. The ignored `just test-camera-stream` acceptance test performs the stronger FFprobe-based check.

## Synology simulator

The `synology` crate simulates the Surveillance Station API surface used for development. It serves fixture-backed catalogue and download responses but does not record or proxy video or model the physical deployment's continuous archive.

Camera definitions come from repeated `--camera <socket-address>` arguments. Argument order assigns IDs starting at `1` and names such as `camera-1`. State is an in-memory `Arc<Mutex<Vec<Camera>>>` and resets when the process stops.

An optional `--recording-catalogue <path>` is loaded once at startup and attached to those configured cameras. `just synology` loads `synology/fixtures/recordings.json`, whose single five-second H.264, 1280x720, silent recording reads `camera/fixtures/default.mp4`. Each strict JSON row contains `id`, `cameraId`, `dsId`, `mountId`, `startTime`, `stopTime`, logical `filePath`, private fixture-relative `video`, `videoCodec`, `audioCodec`, `width`, `height`, and `locked`; `sizeByte` comes from the media file. IDs must be non-zero and unique, camera references and dimensions must be valid, times must increase, and the resolved media must be a regular MP4 file.

The simulator exposes these operations under `/webapi`:

| Endpoint | `api` | `method` | Version | Behavior |
| --- | --- | --- | --- | --- |
| `query.cgi` | `SYNO.API.Info` | `Query` | `1` | Describes the supported APIs. |
| `entry.cgi` | `SYNO.SurveillanceStation.Camera` | `List` | `9` | Lists configured cameras and their reachability. |
| `entry.cgi` | `SYNO.SurveillanceStation.ExternalRecording` | `Record` | `2` | Sets an in-memory recording flag for one reachable camera. |
| `entry.cgi` | `SYNO.SurveillanceStation.Recording` | `List` | `5` | Returns timestamp-bearing `data.events`. |
| `entry.cgi` | `SYNO.SurveillanceStation.Recording` | `List` | `6` | Returns catalogue metadata in `data.recordings`. |
| `entry.cgi` or `entry.cgi/{filename}` | `SYNO.SurveillanceStation.Recording` | `Download` | `6` | Returns full or partial raw MP4 bytes. |

`ExternalRecording.Record` is an independent legacy mock endpoint. It only mutates an in-memory flag, does not affect the immutable fixture catalogue, and is not used by Leo's continuous-recording workflow. Continuous archival recording remains a responsibility of physical Surveillance Station.

### Recording API contract

The implementation follows Surveillance Station Web API v3.11 while preserving a conflict between its primary Recording section and compatibility appendix:

- Recording [`List` v6](https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/SurveillanceStation/All/enu/Surveillance_Station_Web_API.pdf#page=115), PDF viewer pages 115-117.
- Recording [`Download` v6](https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/SurveillanceStation/All/enu/Surveillance_Station_Web_API.pdf#page=127), PDF viewer pages 127-128.
- [Recording errors](https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/SurveillanceStation/All/enu/Surveillance_Station_Web_API.pdf#page=132), PDF viewer pages 132-133.
- Conflicting [`List` v5 events appendix](https://global.download.synology.com/download/Document/Software/DeveloperGuide/Package/SurveillanceStation/All/enu/Surveillance_Station_Web_API.pdf#page=559), PDF viewer pages 559-561.

The two List versions remain separate rather than merging fields from the conflicting schemas:

| Method and version | Successful response | Intended client use |
| --- | --- | --- |
| `List` v5 | `data` contains `offset`, filtered `total`, current Unix `timestamp`, and `events`. Each event contains `archId`, string `audioCodec`, empty `bookmark`, `bookmarkCount`, `cameraId`, `dsId`, `folder`, `id`, `imgHeight`, `imgWidth`, `startTime`, `stopTime`, and string `videoCodec`. | Required for UTC recording boundaries. |
| `List` v6 | `data` contains the requested `dsId`, filtered `total`, and `recordings`. Each recording contains `id`, numeric `videoCodec`, numeric `audioCodec`, `height`, `width`, `cameraId`, `cameraName`, `sizeByte`, logical `filePath`, and `locked`; it deliberately has no timestamps. | Optional richer catalogue metadata. |
| `Download` v6 | Success is the MP4 body itself, without a JSON success envelope. `id` and `mountId` select the fixture; `offsetTimeMs` and `playTimeMs` optionally select a clip. | Required for full or bounded media retrieval. |

`Download` accepts both `/webapi/entry.cgi` and `/webapi/entry.cgi/<filename>`; the filename suffix is optional and does not participate in lookup. Missing `id`, `mountId`, and `offsetTimeMs` default to `0`, while omitted `playTimeMs` means the configured duration remaining after the offset. A complete request returns the original fixture bytes. A partial request uses FFmpeg stream copy to produce an ephemeral MP4 response. Errors retain Synology's HTTP-200 JSON envelope: Recording code `400` means execution failed, `401` means invalid Recording parameters, and `414` means no recording matched `(mountId, id)`.

The following behavior is intentionally simulator policy rather than a claim about physical NAS behavior:

- **Simulator choice: anonymous authentication.** No login or privilege checks run, and `_sid` is accepted and ignored.
- **Simulator choice: GET-only handling.** Only the documented GET-shaped routes are registered; POST is unsupported.
- **Simulator choice: zero-bound interpretation.** `fromTime=0` and `toTime=0` mean unbounded lower and upper limits.
- **Simulator choice: half-open overlap.** A recording matches when `(fromTime == 0 || stopTime > fromTime) && (toTime == 0 || startTime < toTime)`.
- **Simulator choice: deterministic sorting.** Results sort by `(startTime, cameraId, id)` before pagination; `total` is the filtered count and an offset past it returns an empty page.
- **Simulator choice: strict range rejection.** Download rejects a zero `playTimeMs`, an offset at or beyond the configured duration, overflow, or an end beyond that duration with code `401`; it does not clamp the range.
- **Simulator choice: `Content-Type`.** Successful Download responses use `video/mp4` as a convenience. Clients must not depend on that undocumented header.

Reachability is a TCP connection attempt with a 250 ms timeout. It does not call `/health`, identify the camera, or inspect RTSP. API failures use Synology-style JSON envelopes and generally return HTTP `200 OK`.

The simulator does not ingest RTSP, persist recordings, perform retention, or create media. It only reads pre-existing fixture files; a partial Download's temporary clip is ephemeral response generation, not a recording. It also omits storage accounting, recording mutation, bookmarks, CMS behavior, dynamic camera management, HTTP range streaming, and graceful shutdown. It should be bound only to a trusted development interface.

## Network and time

Development recipes and examples use:

| Service | Address |
| --- | --- |
| Virtual camera HTTP | `127.0.0.1:8080` |
| Virtual camera RTSP | `127.0.0.1:8554` |
| Synology simulator HTTP | `127.0.0.1:5000` |
| App WHEP HTTP | `127.0.0.1:8889` |
| App WebRTC media | `127.0.0.1:8189/UDP` |

A physical deployment connects the cameras and NAS to the PoE switch and assigns static IP addresses, so recording does not require a router. Cameras, Synology, the operator laptop, metadata services and analysis services must use the same clock source. Clock synchronization is more important than issuing every start command in the same millisecond. Store timestamps in UTC internally.

## Reliability and security constraints

Archival recording must continue when the operator laptop fails or sleeps. Physical cameras and Surveillance Station are therefore configured to record continuously; session and sampling actions do not control recording execution. The laptop remains a replaceable metadata, analysis, control and preview client. The NAS and PoE switch should have protected power so storage and cameras fail predictably together.

Production deployment requires:

- an isolated camera network where anonymous viewing and PTZ are enabled under `System > Accounts > Anonymous access`
- administrator camera accounts retained for device setup but not received or stored by the app
- authenticated administrative and storage services
- encrypted storage on the NAS and operator laptop
- no externally reachable simulator or preview ports
- explicit recovery before camera-local backup media is erased
- storage monitoring before a session and clear retention or export actions when capacity is low

Specific NAS models, disk capacities, switch models and retention values are deployment decisions rather than code architecture.
