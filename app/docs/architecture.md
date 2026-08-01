# Leo architecture

Leo is a session-oriented system for recording and analyzing student exercise with several cameras. It is designed to operate locally, keep archival recording independent from the operator laptop, and provide synchronized video and metadata to an analysis pipeline.

This document defines the domain model, component responsibilities and intended workflows. Where a workflow is incomplete, that status is stated in the section that owns it rather than separated into a legacy or future architecture.

## Vocabulary

These distinctions are part of the domain model and should be reflected in code and data structures.

| Term | Definition |
| --- | --- |
| **Session** | Full recording period across all cameras. |
| **Recording** | Raw video file produced by one camera. |
| **Video** | Recording exported to a standard usable format. |
| **Video stream** | Continuous encoded video data. |
| **Frame** | One decoded image. |
| **Frame rate** | Number of frames produced per second. |
| **Sampling rate** | Number of frames selected per second. |
| **Sampling schedule** | Sampling rate changes over time. |
| **Sample** | A selected frame. |
| **Sample sequence** | Ordered samples selected from one recording. |
| **Frame index** | Position of a frame in a recording. |
| **Sample index** | Position of a sample in a sample sequence. |
| **Frame timestamp** | Frame position on the session timeline. |
| **Frame group** | Frames from different recordings associated with the same timestamp. |
| **Frame batch** | Ordered frame groups covering a bounded time range. |
| **Session sequence** | Ordered frame groups covering the full session. |
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
|                            (`synology` currently simulates control state only)
|
|-- preview RTSP ---------> app-owned MediaMTX --> WHEP/WebRTC --> operator UI
|                            (implemented)
|
|-- frames --------------> analysis pipeline
|                            (scaffolding only)
|
`-- VAPIX API <----------> operator app
                             (virtual API exists; app client is not implemented)
```

The Cargo workspace contains three independent Rust binaries:

| Component | Responsibility |
| --- | --- |
| `app` | Dioxus desktop UI, preview bridge, session workflow and analysis orchestration. Preview is implemented; the other workflows remain incomplete. |
| `camera` | Development replacement for one Axis camera, with a VAPIX-shaped HTTP API and fixture-backed RTSP stream. |
| `synology` | Development replacement for a small Surveillance Station API surface, with in-memory camera and recording-control state. |

Each binary runs as a separate process and communicates over sockets. They do not share memory or a database. The app currently consumes only camera RTSP; it does not yet call VAPIX or Synology.

## Ownership boundaries

The boundaries prevent two systems from trying to own the same recording concern.

### Surveillance Station owns

- archival recording
- reliable execution of recording start and stop requests
- archival stream configuration
- reconnection after camera or network interruptions
- recording catalogue and health monitoring
- recording recovery
- storage rotation and retention enforcement

The `synology` crate does not implement those storage concerns. It only simulates API discovery, camera listing and recording-control state.

### Operator app owns

- session workflow and naming
- operator interface
- recording requests sent through the Synology API
- low-resolution previews
- analysis stream consumption
- timestamped custom metadata
- supported PTZ or digital-view controls
- operator presets
- session retention, export and deletion decisions
- downstream processing status
- alarms presented to the operator

Only live preview and its failure presentation are currently connected to the UI.

### Axis VAPIX provides

- camera status and capabilities
- separate preview and analysis streams
- supported PTZ controls
- stream profiles
- snapshots
- overlays
- camera-side events
- advanced settings that Surveillance Station does not expose

The app must request archival recording through Surveillance Station rather than becoming a second archival recorder through VAPIX. It may decide what session data to retain, export or delete, while Surveillance Station enforces storage operations and maintains the recording catalogue. The app and Surveillance Station must not both modify the same recording profile.

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

- a master session control that starts the session clock and requests recording for enabled cameras
- live views for all cameras
- per-camera recording and health status
- warnings with suggested operator actions
- per-camera enable or disable, sampling rate and digital zoom controls
- timestamped notes and bookmarks
- playback of saved videos where needed
- an action to discard a session recording

Only the live preview grid is functional today. Camera status badges, timestamps, selection labels, settings and route sidebars are static presentation.

### Session metadata

Videos and metadata are grouped by session. The metadata model is an ordered list of timestamped events containing:

- session ID
- timestamp
- action
  - sampling rate: camera and rate
  - digital zoom: camera, position and zoom
  - bookmark: note
  - recording: camera and enabled or disabled state

Camera parameters belong to a session and camera, not to one recording file. Sampling-rate events form the sampling schedule used by later processing. Session persistence and event storage are not implemented yet.

### Retention and export

The session workflow must:

- group recordings, exported videos and metadata by session
- keep metadata aligned with the session timeline
- warn when storage is insufficient and propose an operator action
- export recordings to a standard format for manual or offline analysis
- allow the operator to discard a session recording deliberately

Surveillance Station performs the storage operations; the app records the operator's decision and tracks its result. This workflow is not implemented yet.

### Analysis pipeline

The analysis model preserves the distinction between recordings, frames and selected samples:

1. Decode each recording into frames with frame indices and frame timestamps on the shared session timeline.
2. Apply that camera's sampling schedule to produce a sample sequence with its own sample indices.
3. Optionally blur faces during offline processing.
4. Associate samples from different recordings into frame groups by frame timestamp.
5. Order the frame groups into the session sequence.
6. Divide the sequence into bounded frame batches.
7. Analyze each batch with the previous batch context and the exercise checklist.
8. Aggregate the extracted actions and compare them with the expected sequence.

The `app/src/analysis/` module is early scaffolding and is not connected to the Analyze route or a complete pipeline.

Live analysis may consume a dedicated low-resolution camera stream. Offline analysis should use retained recordings so temporary processing failures can be retried without losing frames. Polling VAPIX JPEG snapshots is simpler, but delays produce irregular sampling and missed frames cannot be recovered; it is not equivalent to decoding a recording according to a sampling schedule.

### External integrations

The app will use the Synology API to:

- query camera and recording status
- request recording start and stop
- access the recording catalogue and exports where supported

It will use Axis VAPIX to:

- consume RTSP H.264 or H.265 streams for previews and live analysis
- query camera status and capabilities
- control PTZ or digital cropping where supported
- request snapshots
- select preview or analysis stream profiles

Neither client is implemented in the app yet. The corresponding development simulators define only the narrow behavior described below.

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
| `app/src/analysis/` | Incomplete video and analysis-agent domain scaffolding. |

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

The `synology` crate simulates the Surveillance Station API surface needed for application development. It does not record or proxy video.

Camera definitions come from repeated `--camera <socket-address>` arguments. Argument order assigns IDs starting at `1` and names such as `camera-1`. State is an in-memory `Arc<Mutex<Vec<Camera>>>` and resets when the process stops.

The simulator exposes these GET operations under `/webapi`:

| Endpoint | `api` | `method` | Version | Behavior |
| --- | --- | --- | --- | --- |
| `query.cgi` | `SYNO.API.Info` | `Query` | `1` | Describes the supported APIs. |
| `entry.cgi` | `SYNO.SurveillanceStation.Camera` | `List` | `9` | Lists configured cameras and their reachability. |
| `entry.cgi` | `SYNO.SurveillanceStation.ExternalRecording` | `Record` | `2` | Sets an in-memory recording flag for one reachable camera. |

Reachability is a TCP connection attempt with a 250 ms timeout. It does not call `/health`, identify the camera, or inspect RTSP. Recording requests do not contact the camera or create a recording; they only mutate simulator state. API failures use Synology-style JSON envelopes and generally return HTTP `200 OK`.

The simulator has no authentication, persistence, recording catalogue, export, storage accounting, dynamic camera management or graceful shutdown. It should be bound only to a trusted development interface.

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

Archival recording must continue when the operator laptop fails or sleeps. The NAS or VMS therefore owns recording execution and storage, while the laptop remains a replaceable control and preview client. The NAS and PoE switch should have protected power so storage and cameras fail predictably together.

Production deployment requires:

- an isolated camera network where anonymous viewing and PTZ are enabled under `System > Accounts > Anonymous access`
- administrator camera accounts retained for device setup but not received or stored by the app
- authenticated administrative and storage services
- encrypted storage on the NAS and operator laptop
- no externally reachable simulator or preview ports
- explicit recovery before camera-local backup media is erased
- storage monitoring before a session and clear retention or export actions when capacity is low

Specific NAS models, disk capacities, switch models and retention values are deployment decisions rather than code architecture.
