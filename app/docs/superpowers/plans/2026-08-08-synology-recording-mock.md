# Synology Recording Mock Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the fixture-backed Synology simulator with the officially documented Recording catalogue and download contracts needed by Leo.

**Architecture:** Keep the existing anonymous Axum simulator and camera state. Load an immutable recording catalogue at startup, expose both official `List` schemas without combining them, and return fixture-backed MP4 data from `Download`.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio, Serde, JSON, `thiserror`, `tempfile`, and FFmpeg/FFprobe from the Nix development shell.

## Global Constraints

- `SYNO.SurveillanceStation.Recording.List` version 5 returns the documented `data.events` timestamp schema.
- `SYNO.SurveillanceStation.Recording.List` version 6 returns the primary documented `data.recordings` schema without undocumented timestamp fields.
- `SYNO.SurveillanceStation.Recording.Download` version 6 returns raw MP4 bytes without a JSON success envelope.
- `SYNO.API.Info` advertises Recording at `entry.cgi`, minimum version 5, maximum version 6.
- Preserve official parameter casing, API names, method names, versions, JSON envelopes, and Recording error codes.
- Resolve the PDF conflict by implementing both documented schemas; never merge `events` and `recordings` into one response.
- Authentication remains disabled. Accept and ignore `_sid`.
- Support the official GET examples only; POST support is outside this mock slice.
- Treat `fromTime=0` and `toTime=0` as unbounded and use half-open overlap as explicit simulator policy because Synology does not document filter boundaries.
- Keep fixture-local `video` paths private; expose only the logical Synology `filePath`.
- Reuse `camera/fixtures/default.mp4`; do not implement RTSP recording, persistence, retention, or storage rotation.
- Keep `ExternalRecording` behavior unchanged and independent from the immutable fixture catalogue.
- Clone recording metadata before asynchronous file or FFmpeg work; never hold the camera mutex across `.await`.
- Keep visibility `pub(crate)` or narrower, use `thiserror`, and keep `mod.rs` declaration/export-only.
- Do not add Reqwest, ffmpeg-sidecar, a database, authentication state, or a new shared-state abstraction.
- Do not commit this implementation plan.

## Locked File Structure

```text
app/
|-- Cargo.lock
|-- Cargo.toml
|-- README.md
|-- docs/architecture.md
|-- justfile
|-- camera/fixtures/default.mp4
\-- synology/
    |-- Cargo.toml
    |-- fixtures/recordings.json
    \-- src/
        |-- api/
        |   |-- entry.rs
        |   |-- error.rs
        |   |-- info.rs
        |   |-- mod.rs
        |   \-- recording.rs
        |-- camera.rs
        |-- cli.rs
        |-- main.rs
        \-- recording.rs
```

---

### Task 1: Fixture Recording Catalogue

**Files:**
- Create: `synology/src/recording.rs`
- Create: `synology/fixtures/recordings.json`
- Modify: `synology/src/camera.rs`
- Modify: `synology/src/cli.rs`
- Modify: `synology/src/main.rs`
- Modify: `synology/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**

```rust
#[derive(Clone)]
pub(crate) struct Recording {
    pub id: u32,
    pub camera_id: u32,
    pub ds_id: u32,
    pub mount_id: u32,
    pub start_time: u64,
    pub stop_time: u64,
    pub file_path: String,
    pub video_path: PathBuf,
    pub video_codec: u8,
    pub audio_codec: u8,
    pub width: u32,
    pub height: u32,
    pub size_byte: u64,
    pub locked: bool,
}

pub(crate) fn load_catalogue(path: &Path, cameras: &mut [Camera]) -> Result<()>;
```

- [ ] **Step 1: Write failing CLI and catalogue tests**

Cover optional `--recording-catalogue`, successful relative media-path resolution, unknown JSON fields, zero and duplicate IDs, unknown camera IDs, invalid time ranges, missing/non-file/non-MP4 media, zero dimensions, actual `sizeByte`, and `(startTime, id)` sorting.

- [ ] **Step 2: Run focused tests and verify red**

Run `cargo test -p synology`.

Expected: compilation fails because the recording module and CLI field do not exist.

- [ ] **Step 3: Implement the minimal catalogue**

Add `recordings: Vec<Recording>` to `Camera`. Add optional `--recording-catalogue <PATH>`. Deserialize fixture rows with `#[serde(deny_unknown_fields, rename_all = "camelCase")]`, resolve `video` relative to the catalogue, validate the constraints above, compute file size, and attach each sorted recording to its configured camera.

Use this committed fixture row:

```json
{
  "id": 1,
  "cameraId": 1,
  "dsId": 0,
  "mountId": 0,
  "startTime": 1786147200,
  "stopTime": 1786147205,
  "filePath": "20260808AM/camera-1-1786147200.mp4",
  "video": "../../camera/fixtures/default.mp4",
  "videoCodec": 3,
  "audioCodec": 0,
  "width": 1280,
  "height": 720,
  "locked": false
}
```

Keep `filePath` and `video` separate. Change `main` to return an error type capable of reporting catalogue errors, then load the catalogue before starting Axum.

- [ ] **Step 4: Verify and commit Task 1**

Run `cargo fmt --all --check` and `cargo test -p synology`.

Commit only implementation files with `feat(synology): add fixture recording catalogue`. Do not stage this plan.

---

### Task 2: Recording Discovery and List Versions 5 and 6

**Files:**
- Create: `synology/src/api/recording.rs`
- Modify: `synology/src/api/mod.rs`
- Modify: `synology/src/api/entry.rs`
- Modify: `synology/src/api/info.rs`
- Modify: `synology/src/api/error.rs`

**Interfaces:**

Add raw optional `String` query fields for `offset`, `limit`, `cameraIds`, `fromTime`, `toTime`, `dsId`, `mountId`, `id`, `offsetTimeMs`, and `playTimeMs`. Parse them inside the Recording handler so API/method/version error precedence remains `102`, `103`, `104`, then method-specific `401`.

- [ ] **Step 1: Write failing API discovery and full-response tests**

Add exact JSON assertions for API discovery, v5 `events`, and v6 `recordings`. Cover camera, storage, and half-open time filtering; pagination after filtering; an offset beyond total; malformed fields; ignored `_sid`; and existing error precedence.

The v6 item fields are `id`, numeric `videoCodec`, numeric `audioCodec`, `height`, `width`, `cameraId`, string `cameraName`, `sizeByte`, string `filePath`, and `locked`. Do not add timestamps.

The v5 event fields are `archId`, string `audioCodec`, empty `bookmark`, `bookmarkCount`, `cameraId`, `dsId`, `folder`, `id`, `imgHeight`, numeric `imgWidth`, `startTime`, `stopTime`, and string `videoCodec`. Do not add example-only `event_size_bytes`.

- [ ] **Step 2: Run focused tests and verify red**

Run `cargo test -p synology`.

Expected: compilation or assertions fail because Recording is not advertised or dispatched.

- [ ] **Step 3: Implement discovery, parsing, filtering, and both serializers**

Advertise `entry.cgi` with versions 5 through 6. Dispatch `List` versions 5 and 6 and reserve `Download` version 6 for Task 3. Filter with:

```text
(fromTime == 0 || recording.stopTime > fromTime)
&& (toTime == 0 || recording.startTime < toTime)
```

Sort by `(startTime, cameraId, id)`, then paginate. `total` is the filtered count before pagination. Omitted `limit` returns all remaining results. `offset` past the end succeeds with an empty page.

Use formal-schema numeric codecs in v6. Use the official example's obvious string types for `cameraName` and `filePath`. Use formal-schema timestamp integers and image dimensions in v5 despite PDF type typos.

- [ ] **Step 4: Verify and commit Task 2**

Run `cargo fmt --all --check` and `cargo test -p synology`.

Commit only implementation files with `feat(synology): mock recording catalogue APIs`. Do not stage this plan.

---

### Task 3: Recording Download Version 6

**Files:**
- Modify: `synology/src/api/mod.rs`
- Modify: `synology/src/api/entry.rs`
- Modify: `synology/src/api/recording.rs`
- Modify: `synology/src/api/error.rs`
- Modify: `synology/Cargo.toml`
- Modify: `Cargo.lock`

**Contract:**

Support both `/webapi/entry.cgi` and `/webapi/entry.cgi/<filename>`. A successful Download returns raw MP4 data with no JSON success envelope. Missing `id` uses documented default `0`; `mountId` defaults to `0`; `offsetTimeMs` defaults to `0`; omitted `playTimeMs` means the remaining recording duration.

- [ ] **Step 1: Write failing full-download and error tests**

Cover exact fixture bytes for a full download, filename-suffixed routing, raw body rather than JSON, missing ID defaulting to zero, malformed/range errors as `401`, unknown ID/mount as `414`, missing fixture or FFmpeg failure as `400`, and HTTP-200 JSON errors.

- [ ] **Step 2: Run focused tests and verify red**

Run `cargo test -p synology download`.

Expected: tests fail because `Download` and the filename route do not exist.

- [ ] **Step 3: Implement full and partial media responses**

Clone the selected `Recording` while holding the mutex, then release the mutex. Return the original bytes for a complete recording. For a partial range, use `tempfile::TempDir` and `tokio::process::Command` to run:

```bash
ffmpeg -hide_banner -loglevel error -y \
  -ss <offset>ms -i <fixture> -t <duration>ms \
  -map 0:v:0 -an -c:v copy -movflags +faststart <output>.mp4
```

Reject an explicit zero duration, an offset outside the recording, or a requested end beyond the configured recording duration with Recording error `401`. Return `Content-Type: video/mp4` as a simulator convenience, but do not make future clients depend on undocumented headers.

- [ ] **Step 4: Write and run the ignored media acceptance test**

Request `offsetTimeMs=1000&playTimeMs=2000`, write the response body to a temporary MP4, invoke FFprobe, and assert H.264 with duration approximately two seconds. Mark the test ignored because it requires Nix-provided FFmpeg and FFprobe.

Run `nix develop --command cargo test -p synology downloads_partial_recording -- --ignored`.

- [ ] **Step 5: Verify and commit Task 3**

Run `cargo fmt --all --check`, `cargo test -p synology`, and the ignored media test.

Commit only implementation files with `feat(synology): mock recording downloads`. Do not stage this plan.

---

### Task 4: Documentation, Recipes, and Whole-Workspace Verification

**Files:**
- Modify: `justfile`
- Modify: `README.md`
- Modify: `docs/architecture.md`

- [ ] **Step 1: Document exact official coverage and explicit deviations**

Document the List v5/v6 matrix, Download v6, optional filename suffix, fixture catalogue, official PDF sections/pages, anonymous GET-only simulator behavior, half-open filtering policy, and raw MP4 response. State that the simulator does not ingest RTSP, persist recordings, perform retention, or create media.

Keep `ExternalRecording` documented as an independent legacy mock endpoint that Leo's continuous-recording workflow does not use.

- [ ] **Step 2: Add launch and media-test recipes**

Add `just synology` using `synology/fixtures/recordings.json` and `just test-synology-recording` running the ignored partial-download test in the Nix shell.

- [ ] **Step 3: Correct the broader pipeline dependency**

Do not edit or commit the broader implementation plan. Report that its app client must call List v5 for UTC boundaries, may call List v6 for catalogue metadata, must use Download v6, and should join by `(dsId, cameraId, id)` until hardware validates the contract.

- [ ] **Step 4: Run final verification**

Run:

```bash
cargo fmt --all --check
cargo test -p synology
nix develop --command cargo test -p synology downloads_partial_recording -- --ignored
cargo clippy -p synology --all-targets -- -D warnings
cargo test --workspace
```

Expected: all normal tests pass, the ignored downloaded clip is playable H.264 of approximately the requested duration, Synology Clippy has no warnings, and existing Camera/ExternalRecording behavior remains green.

- [ ] **Step 5: Commit documentation and recipes**

Commit only implementation documentation and recipes with `docs(synology): document recording mock`. Do not stage this plan.

## Explicitly Deferred

- Physical NAS response capture and reconciliation.
- Authentication, SID storage, login, and privilege behavior.
- POST request support.
- Real RTSP ingestion and continuous recording creation.
- Retention, storage accounting, recording mutation, bookmarks, CMS servers, and mounts beyond fixture metadata.
- HTTP range requests and media streaming for large files.
- Exact keyframe/sample clipping semantics not documented by Synology.
