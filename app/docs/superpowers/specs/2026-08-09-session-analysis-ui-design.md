# Session And Analysis UI Design

Date: 2026-08-09

Status: Approved

## Goal

Provide the first complete local operator workflow across the existing Monitor and Analyze routes:

```text
two virtual-camera previews
        |
        v
start software session
        |
        | include or exclude cameras from future analysis
        v
stop software session -> events.jsonl
        |
        v
Analyze route -> Synology List v5 and Download v6
        |
        v
FFmpeg frames -> OpenAI batches -> analysis.json
```

Physical and simulated cameras continue recording regardless of session actions. Camera participation affects only which frames future analysis samples. An excluded camera remains visible in the preview and is clearly labelled as excluded from analysis.

This increment must run locally with two separate virtual cameras and the fixture-backed Synology simulator. It also preserves the same supported HTTP contracts needed by a future physical Synology deployment.

## Scope

- Two configured virtual cameras with stable Synology IDs, distinct preview streams, and synchronized finite video fixtures.
- A Monitor-page session lifecycle: start, change camera participation, and stop.
- Durable `events.jsonl` creation under a timestamped local session directory.
- Shared in-memory state for the active session and the most recently completed session during the current app process.
- An Analyze page with a checklist textarea, explicit analysis action, resumable batch progress, and structured results.
- Local Synology List v5 timestamp discovery and Download v6 media retrieval.
- An opt-in simulator launch mode that aligns finite fixture timestamps to the requested session range.
- Automated file, state, client, simulator, and analysis-orchestration tests.
- One explicit local two-camera acceptance flow.

The first increment does not include sampling-interval controls, physical recording controls, session history, completed-session discovery after restart, active-session recovery after restart, a settings page, raw JSON viewers, automatic navigation after Stop, analysis cancellation, camera discovery, permanent extracted media, or packaged-app delivery.

## Two-Camera Configuration

One startup camera definition is the source for both preview identity and session identity. Each configured camera has:

```text
id          stable Synology camera ID
name        operator-facing name
rtsp_url    preview source consumed by the app-owned MediaMTX
```

The local configuration contains:

| ID | Name | Virtual-camera HTTP | Virtual-camera RTSP | Fixture |
| ---: | --- | --- | --- | --- |
| 1 | Salon 1 | `127.0.0.1:8080` | `rtsp://127.0.0.1:8554/axis-media/media.amp` | `videos/salon-1-synced.mp4` |
| 2 | Salon 2 | `127.0.0.1:8081` | `rtsp://127.0.0.1:8555/axis-media/media.amp` | `videos/salon-2-synced.mp4` |

`CameraSource` and `PreviewFeed` carry the stable camera ID. Generated MediaMTX path indices remain preview implementation details and never become domain camera IDs.

Both cameras are initially included in analysis and use a fixed four-second sampling interval. The UI does not expose interval editing in this increment.

Development recipes start each virtual camera independently, then start Synology with camera arguments in ID order and the two-row fixture catalogue. The app renders the two real preview sources instead of cloning one source five times.

## Finite Fixture Time Alignment

The Synology fixture recordings remain finite. The initial catalogue advertises a conservative 24-second duration for both synchronized salon files, bounded by the shorter media file. Replacing those files and catalogue durations permits longer local sessions without changing the app.

Fixed catalogue UTC timestamps cannot overlap a newly created UI session reliably. The simulator therefore gains an opt-in CLI flag:

```text
--align-recordings-to-query
```

This flag is simulator-only configuration. It is not a Synology HTTP parameter, response field, or fixture JSON field. Normal simulator launches retain the existing fixed UTC behavior.

When the flag is enabled and a Recording List request contains a non-zero `fromTime`, the simulator shifts every configured recording by the same amount so the catalogue's earliest recording begins at `fromTime`. Relative recording offsets and finite durations are preserved. Filtering uses the shifted bounds, and List v5 emits those shifted `startTime` and `stopTime` values. List v6 continues to emit its documented metadata-only schema.

Pagination remains deterministic because every page uses the same request bounds. A later session can use a different `fromTime` and receives the same finite fixtures aligned to that session. A session extending beyond the shortest shifted fixture still fails with missing recording coverage; the simulator does not loop or invent media.

The local `just synology` recipe enables this flag. API contract tests and ordinary custom launches leave it disabled.

## Recording Client Contract

The app recording client must match the simulator's documented split schemas:

- `SYNO.SurveillanceStation.Recording.List` version 5 supplies `data.events[]` with `id`, `cameraId`, `dsId`, `startTime`, and `stopTime` for deterministic UTC coverage.
- `SYNO.SurveillanceStation.Recording.Download` version 6 supplies full or bounded MP4 bytes.
- List version 6 contains `data.recordings[]` metadata without timestamps and is not needed by this increment.

`SynologyClient::list_videos` switches from the incompatible v6 timestamp assumption to List v5. It retains outward second rounding, pagination, deterministic sorting, error-envelope handling, and conversion to UTC milliseconds.

The local fixtures use globally unique recording IDs, `dsId = 0`, and `mountId = 0`; Download can use the documented default mount. Multi-DS and multi-mount identity reconciliation remains deferred until physical hardware validates the required join.

## Session Storage

The first session root is:

```text
./sessions
```

Start creates a new directory named with the current UTC milliseconds:

```text
sessions/<UTC-milliseconds>/events.jsonl
```

The timestamp is a directory key, not the durable session identity. `SessionController` still generates the UUID stored in every event. Directory creation and `SessionController::create` must fail rather than reuse an existing session path.

Analysis writes beside the event log:

```text
sessions/<UTC-milliseconds>/analysis.json
```

The UI displays the current session directory and whether each expected durable file exists. Automated tests, rather than an in-app raw JSON viewer, verify exact schemas and event ordering. Temporary downloaded clips and JPEGs remain private batch-local files and are never displayed as session artifacts.

## Shared Session State

`App` provides one reactive session state above the router so Monitor and Analyze share the same current-process workflow. It retains only:

- the active `SessionController`, when a session is running;
- the active `events.jsonl` path;
- current participation state for the two configured cameras;
- the most recently completed `events.jsonl` path;
- operator-visible status or error text.

No session index is persisted. Restarting the app forgets the latest completed path, although the files remain on disk. Filesystem discovery and session history are separate follow-up work.

Starting a new session clears the previous in-memory completed-session selection but never removes its directory. This keeps the first workflow single-session: Analyze is unavailable while the replacement session is active and selects that replacement only after it stops.

State changes follow durable writes:

1. Start creates the directory and controller before the UI becomes active.
2. A participation toggle calls `SessionController::apply` before changing the displayed participation state.
3. Stop appends `EndSession`, releases the active controller, and retains the completed path.
4. Analyze independently calls `Session::load` so malformed or incomplete logs never reach planning.

After Stop, the app remains on Monitor. The operator navigates through the existing navbar when ready to analyze.

## Monitor Route

The Monitor sidebar has three states:

### Idle

- `Start session` action.
- Both configured camera names and their initial included state.
- Any previous completed-session path from this app run.

### Active

- Clear active-session status and elapsed duration.
- One participation toggle per camera.
- `Stop session` action.
- Current session directory.

The toggle label describes analysis participation rather than recording. Preview and Synology recording continue unchanged.

### Completed

- Confirmation that the session stopped, or a validation error if its completed log could not be reloaded.
- `events.jsonl` path and existence status.
- Guidance to use the existing Analyze navbar route.
- Ability to start another session, replacing only the in-memory latest-session selection.

Every camera card remains mounted and streaming. While a session is active, an excluded camera card is visually muted and displays `Excluded from analysis`. Existing static `Selected`, camera number, and timestamp presentation must not claim state that is not real.

The controls preserve the existing Tailwind CSS and daisyUI visual language. Buttons and participation controls use semantic elements with visible labels, status and error text use appropriate live-region roles, keyboard operation remains available, and the sidebar and camera grid remain usable at narrow desktop-window widths.

## Analyze Route

With no completed session in the current process, Analyze shows concise guidance to start and stop one from Monitor.

With a completed session, Analyze shows:

- the session directory;
- `events.jsonl` validation status;
- a required checklist textarea;
- an explicit `Analyze` or `Resume analysis` action;
- completed and total batch counts;
- current status or retryable error;
- observations aggregated from completed batches;
- the latest cumulative sequence summary;
- the latest cumulative checklist status and notes;
- the `analysis.json` path and existence status.

Opening the route never spends model tokens. Analysis starts only after the operator presses the explicit action with a non-empty checklist.

One Dioxus asynchronous task owns an analysis run. It:

1. Reloads `Session` from `events.jsonl`.
2. Builds `SynologyClient` from `LEO_SYNOLOGY_URL`, defaulting to `http://127.0.0.1:5000`.
3. Builds `OpenAiAgent` from `OPENAI_API_KEY`, `ANALYSIS_MODEL`, and optional `OPENAI_BASE_URL`.
4. Calls `Analyzer::resume` with a fixed batch size of five frame sets.
5. Initializes display state from any checkpointed responses.
6. Calls `analyze_next` sequentially until all batches are complete.
7. Updates progress and results after every durable checkpoint.

Duplicate analysis starts are disabled while the task is running. A failed run leaves its checkpoint intact; pressing the action again rebuilds the plan and resumes from the first incomplete batch.

## Backend API Boundaries

The previous backend work narrowed unused APIs. This UI creates real cross-module callers, so only the required contracts become crate-visible:

- `SessionController`, `SessionController::create`, and `SessionController::apply`;
- `OpenAiAgent` and `OpenAiAgent::from_env`;
- `AnalysisResponse`, `Observation`, `ChecklistProgress`, and fields rendered by the UI;
- `Analyzer`, `Analyzer::resume`, `Analyzer::next_batch_index`, and `Analyzer::analyze_next`;
- minimal Analyzer accessors for total batch count and ordered completed responses.

`AnalysisCheckpoint`, `CompletedBatch`, persisted event DTOs, extraction details, and prompt construction remain private. No generic event bus, repository, service trait, or UI-specific duplicate analysis DTO is introduced.

## Errors And Recovery

- Failure to create the session directory or initial event file leaves the UI idle and displays the error.
- Failure to append a participation event leaves the previous toggle state visible.
- Successful `EndSession` always leaves the active state. The completed path remains available even if the immediate validation read fails, and Analyze retries validation from disk.
- Missing or invalid environment configuration prevents analysis before any model request.
- Synology catalogue, recording coverage, download, FFmpeg, model, and checkpoint errors remain distinct and visible.
- Analysis failure keeps completed checkpoint batches and exposes a retry action.
- A session longer than a finite local fixture reports missing recording coverage. It is not silently clamped.
- App termination during an active session may leave an incomplete log; reopening active sessions remains explicitly deferred.

## Testing

### Simulator Tests

- Fixed mode preserves existing UTC filtering and response behavior.
- Alignment mode shifts the earliest recording to non-zero `fromTime` and preserves relative offsets and durations.
- Alignment applies consistently to filtering, pagination, List v5 timestamps, and List v6 metadata selection.
- A session range beyond a finite fixture remains only partially covered rather than being extended.
- Download v6 continues to return the requested finite bytes or clip.

### App Recording Tests

- List requests use version 5 and parse exact `data.events[]` responses.
- UTC bounds round outward and pagination remains deterministic.
- List v6 metadata-only responses are not accepted as timestamped video events.
- Download v6 behavior remains covered.

### Session UI-State Tests

Use ordinary Rust state tests with a temporary session root rather than adding a Dioxus DOM-testing dependency:

- Start creates exactly one timestamped directory and immediate `session_started` line.
- Excluding and re-including camera 2 append ordered participation events.
- Failed appends do not change displayed participation state.
- Stop appends `session_ended`, produces a loadable completed session, and retains the path.
- Starting another session clears only the in-memory latest selection and never overwrites existing files; stopping it selects its new path.

### Analyze Orchestration Tests

- Empty checklist is rejected before client or model construction.
- Existing checkpoint responses initialize progress and rendered results.
- Remaining batches run sequentially and update progress after each save.
- Failure leaves prior responses visible and a later retry resumes correctly.
- Aggregated observations use all completed responses; summary and checklist use the latest response.

### Local Acceptance

The explicit local acceptance flow is:

1. Start virtual cameras 1 and 2 with the synchronized salon fixtures.
2. Start Synology with both camera addresses, the two-row catalogue, and `--align-recordings-to-query`.
3. Start the desktop app and verify two distinct previews.
4. Start a session.
5. Exclude camera 2, wait, then include it again.
6. Stop before the advertised finite fixture duration.
7. Verify the displayed `events.jsonl` exists and contains start, participation changes, and end in order.
8. Navigate to Analyze, enter a short checklist, and start analysis explicitly.
9. Verify progress completes, structured results render, and `analysis.json` exists beside the event log.
10. Verify no downloaded clips or JPEGs remain in the session directory.

The OpenAI step is explicitly operator-triggered and incurs one or more real model requests according to session length and batch count.

## Deferred Work

- Sampling interval controls and configurable initial cadence.
- Session history, file selection, and completed-session discovery after restart.
- Active-session continuation after restart.
- Automatic navigation after Stop.
- Physical Synology authentication and multi-DS or multi-mount identity reconciliation.
- Physical NAS acceptance and response reconciliation.
- Camera discovery and persisted camera configuration.
- Settings UI for endpoints, credentials, models, storage, and batch size.
- Analysis cancellation and concurrent batch processing.
- Raw JSON viewers and native open-folder actions.
- Checkpoint fingerprints for changed events, checklist, recordings, or batch boundaries.
- Bundled FFmpeg and MediaMTX, Nix application packaging, installers, and Windows support.
