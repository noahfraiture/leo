# Virtual Camera RTSP Stream Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one virtual camera expose a prepared H.264 MP4 fixture continuously at an Axis-compatible RTSP URL through a supervised MediaMTX child process.

**Architecture:** The binary entrypoint composes two sibling services: the existing Axum HTTP/VAPIX server and a new `rtsp` module that owns MediaMTX. The camera domain remains unaware of process and media-server details. MediaMTX configuration and lifecycle stay behind one concrete `rtsp::Server`; no trait, provider abstraction, or reusable media framework is added.

**Tech Stack:** Rust 2024, Tokio, Axum, Clap, thiserror, MediaMTX 1.18.2, FFmpeg/FFprobe, Nix, just.

## Global Constraints

- This plan implements GitHub issue #22 only.
- The public stream path is `/axis-media/media.amp`.
- The initial stream is anonymous H.264 over RTP/RTSP/TCP only.
- The camera accepts one video file and loops it continuously.
- FFmpeg prepares and validates fixtures but is not launched by the camera.
- The camera does not implement RTSP, RTP, RTCP, decoding, encoding, or media transforms.
- The camera does not restart MediaMTX after a runtime failure.
- App rendering, Synology ingestion, fixture switching, authentication, audio, H.265, UDP, and multicast remain out of scope.
- No externally public Rust API is added; the `camera` package remains a binary crate.
- Cross-module items use `pub(crate)`. Implementation helpers and fields remain private or `pub(super)`.
- Non-trivial errors use `thiserror` and live in the owning module's `error.rs`.
- Do not modify unrelated Dioxus worktree changes.

---

## Locked File Structure

```text
app/
|-- Cargo.toml
|-- flake.nix
|-- justfile
|-- camera/
|   |-- Cargo.toml
|   |-- fixtures/
|   |   \-- default.mp4
|   |-- src/
|   |   |-- main.rs
|   |   |-- error.rs
|   |   |-- cli.rs
|   |   |-- http.rs
|   |   |-- camera/
|   |   |   |-- mod.rs
|   |   |   \-- error.rs
|   |   |-- rtsp/
|   |   |   |-- mod.rs
|   |   |   |-- error.rs
|   |   |   \-- mediamtx.rs
|   |   \-- vapix/
|   |       |-- mod.rs
|   |       |-- error.rs
|   |       \-- ptz.rs
|   \-- tests/
|       \-- rtsp_stream.rs
\-- synology/
    \-- Cargo.toml
```

### Deleted Or Moved Files

- Delete `camera/src/cli/error.rs`; it is empty and no CLI-specific runtime error exists.
- Replace `camera/src/cli/mod.rs` and `camera/src/cli/cli.rs` with the single file `camera/src/cli.rs`.
- Rename `camera/src/server.rs` to `camera/src/http.rs`; `server` becomes ambiguous once the process owns HTTP and RTSP servers.
- Move the contents of `camera/src/camera/camera.rs` into `camera/src/camera/mod.rs` and delete `camera/src/camera/camera.rs`.
- Remove the unused `Status`, `Stream`, `VideoQuality`, and `Codec` types. MediaMTX owns stream lifecycle; these placeholders must not imply otherwise.

## Module Dependency Rules

```text
main
|-- cli
|-- error
|-- camera
|-- http
|   |-- camera
|   \-- vapix
\-- rtsp

vapix --> camera
rtsp  --> std + tokio + serde_json
camera -/-> rtsp
vapix  -/-> rtsp
```

- `camera` contains only virtual-device state and command validation.
- `vapix` translates HTTP requests into `camera` operations.
- `http` owns Axum listener/router construction.
- `rtsp` owns MediaMTX configuration, process state, readiness, and cleanup.
- `main` is the only module allowed to coordinate HTTP, RTSP, and shutdown.
- Future fixture switching in #45 may add a narrow command channel between process composition and `rtsp`; it must not put `tokio::process::Child` into `Camera`.

## Visibility Rules

- No item is declared `pub` for external consumers.
- `cli::Args`, `cli::parse_args`, `http::serve`, `rtsp::Server`, and module error types are `pub(crate)` because sibling modules use them.
- `http::router`, MediaMTX configuration rendering, temporary-file ownership, readiness polling, and all struct fields are private.
- `rtsp::mod.rs` is a facade only; it re-exports `Server` and `Error` and contains no lifecycle logic.
- Trivial data needed by `main` remains in crate-visible `Args` fields, following the workspace convention against trivial getters.

## Public And Crate-Visible Interfaces

### `camera/src/cli.rs`

```rust
pub(crate) struct Args {
    pub(crate) address: SocketAddr,
    pub(crate) rtsp_address: SocketAddr,
    pub(crate) video: PathBuf,
}

pub(crate) fn parse_args() -> Args;
```

`Args` remains the only CLI data structure. Do not create a second configuration type with identical fields.

### `camera/src/http.rs`

```rust
pub(crate) async fn serve(camera: Camera, address: SocketAddr) -> std::io::Result<()>;

fn router(camera: Camera) -> Router;
```

`serve` binds HTTP and runs Axum. It does not start, stop, inspect, or depend on MediaMTX. Existing router tests remain colocated in this file.

### `camera/src/rtsp/mod.rs`

```rust
pub(crate) use error::Error;
pub(crate) use mediamtx::Server;
```

No `RtspServer` trait is introduced. There is one implementation and one process.

### `camera/src/rtsp/mediamtx.rs`

```rust
pub(crate) struct Server {
    // private Child and runtime-config ownership
}

impl Server {
    pub(crate) async fn start(
        address: SocketAddr,
        video: PathBuf,
    ) -> Result<Self, Error>;

    pub(crate) async fn wait(&mut self) -> Result<(), Error>;

    pub(crate) async fn stop(self) -> Result<(), Error>;
}
```

Significant private helpers in this file are:

```rust
struct ConfigFile;

impl ConfigFile {
    fn create(address: SocketAddr, video: &Path) -> Result<Self, Error>;
    fn path(&self) -> &Path;
}

fn render_config(address: SocketAddr, video: &Path) -> Result<String, Error>;

async fn wait_until_ready(
    child: &mut Child,
    address: SocketAddr,
) -> Result<(), Error>;
```

Responsibilities are fixed:

- `ConfigFile` creates one permission-restricted file in the operating-system temporary directory and deletes it on drop.
- `render_config` emits RTSP-only MediaMTX configuration with a safely quoted canonical video path.
- `start` validates the fixture, creates configuration, spawns MediaMTX with kill-on-drop enabled, and waits for readiness.
- `wait` treats every child exit as unexpected while the camera is running.
- `stop` terminates and waits for the child and consumes `Server`.
- `Drop` is only the last-resort kill and configuration cleanup path; normal shutdown calls `stop`.

Keep configuration and lifecycle in this one file for #22. Split `config.rs` only if #45 makes configuration mutation independently complex.

### `camera/src/rtsp/error.rs`

`rtsp::Error` is `pub(crate)` and owns these categories:

- invalid, missing, or non-file fixture path
- non-UTF-8 fixture path when rendering MediaMTX configuration
- configuration value serialization failure
- temporary configuration create or write failure
- MediaMTX executable missing or spawn failure
- MediaMTX exit before readiness
- five-second RTSP readiness timeout
- MediaMTX wait or stop failure
- unexpected MediaMTX runtime exit

Do not convert these errors into `std::io::Error`; retain operation-specific context.

### `camera/src/error.rs`

The process-level `Error` is `pub(crate)` and contains only:

- `Http`, wrapping the Axum listener/server `std::io::Error`
- `ShutdownSignal`, wrapping `tokio::signal::ctrl_c` failure
- `Rtsp`, transparently wrapping `rtsp::Error`

Domain and VAPIX errors stay in their current modules.

### `camera/src/main.rs`

```rust
#[tokio::main]
async fn main() -> Result<(), error::Error>;

async fn run(args: cli::Args) -> Result<(), error::Error>;
```

`run` performs process composition in this order:

1. Construct `Camera`.
2. Start `rtsp::Server`; do not expose HTTP health before this succeeds.
3. Start `http::serve`.
4. Select between HTTP completion, MediaMTX exit, and Ctrl-C.
5. On Ctrl-C or HTTP completion, stop MediaMTX before returning.
6. On MediaMTX exit, cancel HTTP by dropping its future and return the RTSP error.

No `Application`, `Runtime`, `ServiceManager`, or dependency-injection struct is introduced.

## Error Ownership

| Error type | Owns | Must not own |
|---|---|---|
| `camera::CameraError` | Camera command validation | HTTP or process failures |
| `vapix::PtzError` | VAPIX request parsing and response conversion | MediaMTX failures |
| `rtsp::Error` | Fixture, config, MediaMTX process, RTSP readiness | Axum or camera commands |
| `crate::error::Error` | Top-level service composition | Protocol response formatting |

## Test Ownership

- `camera/src/cli.rs`: CLI parse tests.
- `camera/src/http.rs`: existing Axum/VAPIX router tests.
- `camera/src/camera/mod.rs`: camera command validation tests.
- `camera/src/rtsp/mediamtx.rs`: configuration rendering, path validation, temporary-file cleanup, and error-category unit tests.
- `camera/tests/rtsp_stream.rs`: black-box process and media compatibility test using the built camera binary and FFprobe.

The black-box test is marked ignored because it requires MediaMTX and FFmpeg from the Nix shell. `just test-camera-stream` runs it explicitly. Normal `cargo test --workspace` remains independent of external executables.

---

### Task 1: Normalize Camera Module Boundaries

**Files:**
- Create: `camera/src/cli.rs`
- Create: `camera/src/http.rs`
- Modify: `camera/src/main.rs`
- Modify: `camera/src/camera/mod.rs`
- Delete: `camera/src/cli/mod.rs`
- Delete: `camera/src/cli/cli.rs`
- Delete: `camera/src/cli/error.rs`
- Delete: `camera/src/server.rs`
- Delete: `camera/src/camera/camera.rs`

**Interfaces:**
- Preserves the existing `cli::Args { address }` and `cli::parse_args` behavior while flattening the module; the RTSP fields are added atomically with process composition in Task 3.
- Produces `http::serve` with the signature locked above.
- Preserves the existing VAPIX router and response behavior.
- Removes dead stream/status placeholders rather than adapting them to MediaMTX.

- [ ] Move CLI parsing into the single top-level `cli.rs` and preserve parse coverage for `--address`.
- [ ] Rename the Axum module to `http`, rename its blocking entrypoint to `serve`, and keep router construction private.
- [ ] Collapse the primary `Camera` type into `camera/mod.rs`, retain `camera/error.rs`, and remove unused status/stream types.
- [ ] Keep `main` compiling temporarily with HTTP-only behavior while using the new names.
- [ ] Run `cargo fmt --all --check` and `cargo test -p camera`; expect all camera tests to pass.
- [ ] Commit only this structural change with `refactor(camera): clarify service modules`.

### Task 2: Add The Prepared Fixture Contract

**Files:**
- Create: `justfile`
- Create: `camera/fixtures/default.mp4`
- Modify: `flake.nix`

**Interfaces:**
- Produces `just prepare-camera-video INPUT OUTPUT`.
- Produces a five-second or longer deterministic `camera/fixtures/default.mp4` matching the approved H.264 contract.
- Makes `mediamtx`, `ffmpeg`, and `ffprobe` available in the Nix development shell.

- [ ] Add `mediamtx` and `ffmpeg` to the existing Nix shell without adding a new flake input.
- [ ] Add one `just` recipe containing the approved normalization command; do not add a wrapper script for one command.
- [ ] Generate the repository fixture from FFmpeg's synthetic test source so no external footage or license metadata is required.
- [ ] Run `ffprobe` for codec, profile, resolution, frame rate, pixel format, B-frame count, audio tracks, and duration.
- [ ] Verify H.264 Baseline, 1280x720, 15 fps, `yuv420p`, zero B-frames, no audio, and at least five seconds duration.
- [ ] Commit the environment, recipe, and fixture together with `feat(camera): add RTSP video fixture`.

### Task 3: Add And Compose The MediaMTX RTSP Server

**Files:**
- Create: `camera/src/rtsp/mod.rs`
- Create: `camera/src/rtsp/error.rs`
- Create: `camera/src/rtsp/mediamtx.rs`
- Create: `camera/src/error.rs`
- Modify: `Cargo.toml`
- Modify: `camera/Cargo.toml`
- Modify: `camera/src/cli.rs`
- Modify: `camera/src/main.rs`
- Modify: `synology/Cargo.toml`

**Interfaces:**
- Produces `rtsp::Server` and `rtsp::Error` exactly as locked above.
- Keeps `ConfigFile`, `render_config`, and readiness polling private.
- Consumes MediaMTX from `PATH`; no executable path is added to CLI configuration.
- Produces the private `run(args)` lifecycle without adding a manager abstraction.

- [ ] Promote `serde_json = "1"` to workspace dependencies because camera configuration quoting and Synology tests both use it.
- [ ] Reference `serde_json` with `workspace = true` from camera dependencies and Synology dev-dependencies.
- [ ] Enable Tokio `process`, `time`, and `signal` features for the camera crate.
- [ ] Add failing unit tests for missing fixtures, non-file fixtures, safe quoting of paths containing spaces or `#`, exact RTSP-only settings, and temporary configuration cleanup.
- [ ] Implement `ConfigFile`, `render_config`, and `rtsp::Error` until those tests pass.
- [ ] Add failing lifecycle tests for early process exit and readiness timeout at the private-module level where they do not require the real MediaMTX binary.
- [ ] Implement `Server::start`, `wait`, `stop`, and fallback drop cleanup.
- [ ] Add `rtsp_address: SocketAddr` and `video: PathBuf` to `cli::Args` with parse coverage for the final CLI.
- [ ] Add the process-level thiserror enum with only HTTP, shutdown-signal, and RTSP categories.
- [ ] Start RTSP before HTTP and use `tokio::select!` for HTTP completion, MediaMTX exit, and Ctrl-C.
- [ ] Ensure each branch has one owner for MediaMTX shutdown and that no branch can orphan the child.
- [ ] Preserve the triggering HTTP or RTSP error when cleanup succeeds; if MediaMTX cleanup fails, return the cleanup error because the child may still be live.
- [ ] Run the camera with a missing video, missing `mediamtx`, and occupied RTSP address; verify each fails before `/health` becomes reachable.
- [ ] Run `cargo fmt --all --check`, `cargo clippy -p camera --all-targets -- -D warnings`, and `cargo test -p camera`; expect all checks to pass.
- [ ] Commit with `feat(camera): serve fixture over RTSP`.

### Task 4: Prove The Stream At The Process Boundary

**Files:**
- Create: `camera/tests/rtsp_stream.rs`
- Modify: `justfile`

**Interfaces:**
- Consumes only the camera CLI, `/health`, and `rtsp://.../axis-media/media.amp`.
- Does not import private camera modules or inspect MediaMTX internals.
- Produces `just test-camera-stream` as the explicit external integration check.

- [ ] Add an ignored black-box test that locates the Cargo-built camera binary and repository fixture.
- [ ] Reserve distinct local HTTP and RTSP ports, launch the camera, and bound every readiness wait.
- [ ] Use FFprobe or FFmpeg with RTSP-over-TCP to read longer than two fixture durations.
- [ ] Run two readers concurrently and require both to receive valid H.264 video.
- [ ] Stop the camera and assert that its HTTP and RTSP listeners close; the closed RTSP listener is the observable proof that the MediaMTX child stopped.
- [ ] Make all child cleanup execute on assertion failure through test-owned guards.
- [ ] Add `just test-camera-stream` to run exactly this ignored test inside the Nix shell.
- [ ] Run `cargo test --workspace`; expect normal tests to pass without external media tools.
- [ ] Run `just test-camera-stream`; expect the black-box stream test to pass.
- [ ] Run `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Commit with `test(camera): verify RTSP fixture stream`.

## Final Verification

- [ ] Run `cargo test --workspace` and confirm zero failures.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings` and confirm zero warnings.
- [ ] Run `cargo fmt --all --check` and confirm no formatting diff.
- [ ] Run `just test-camera-stream` and confirm looping plus two concurrent readers.
- [ ] Run `git status --short` and confirm only unrelated pre-existing Dioxus changes remain.
- [ ] Update #22 checkboxes with verified evidence; do not close #45, #46, #44, or #38.
