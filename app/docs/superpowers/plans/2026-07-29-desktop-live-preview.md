# Desktop Live Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the issue #22 virtual-camera RTSP stream in the real Dioxus desktop app through an app-owned MediaMTX v1.18.2 WebRTC bridge.

**Architecture:** A `preview` module verifies and supervises MediaMTX, owns a protected temporary WebRTC-only configuration, and gives cloneable local playback metadata to Dioxus. `CameraFeed` renders a native video element and uses Dioxus document interop to connect MediaMTX's own `reader.js` to that element; the desktop event loop owns and stops the child.

**Tech Stack:** Rust 2024, Dioxus Desktop 0.7, MediaMTX 1.18.2, WebRTC/WHEP, `thiserror`, `tempfile`, `rand`, DaisyUI/Tailwind CSS.

## Global Constraints

- Require the Nix development shell and exact `mediamtx --version` output `v1.18.2` after trimming whitespace.
- Bind WebRTC HTTP and ICE services only to `127.0.0.1`.
- Keep RTSP source URLs and camera credentials out of preview metadata, DOM attributes, and browser request URLs.
- Use a cryptographically random process-local MediaMTX read password.
- Do not use an HTTP iframe, write a WHEP client, vendor `reader.js`, or add automatic sidecar restart.
- Keep `mod.rs` files minimal and put the `preview` module's non-trivial errors in `preview/error.rs`.
- Keep this implementation to one hardcoded virtual-camera source; do not implement the broader #38 UI.
- Do not commit this implementation plan.

## File Structure

- `app/src/preview/mod.rs`: declare the preview submodules and expose only the bridge-facing types used by `main` and the UI.
- `app/src/preview/config.rs`: render and own the temporary MediaMTX configuration.
- `app/src/preview/error.rs`: define startup, readiness, and cleanup errors with `thiserror`.
- `app/src/preview/bridge.rs`: define source and preview metadata, verify MediaMTX, supervise the child, and implement cleanup.
- `app/src/components/camera/mod.rs`: expose `CameraFeed` without component logic.
- `app/src/components/camera/feed.rs`: render the camera card and run the bounded Dioxus document interop program.
- `app/src/views/monitor/monitor.rs`: select ready or unavailable preview UI from root context.
- `app/src/main.rs`: start the bridge before launch and transfer child ownership to the desktop event loop.
- `Cargo.toml`, `camera/Cargo.toml`, `app/Cargo.toml`: declare direct dependencies in the smallest correct workspace scope.

---

### Task 1: MediaMTX Preview Configuration

**Files:**
- Create: `app/src/preview/mod.rs`
- Create: `app/src/preview/config.rs`
- Create: `app/src/preview/error.rs`
- Create: `app/src/preview/bridge.rs`
- Modify: `app/src/main.rs`
- Modify: `Cargo.toml`
- Modify: `camera/Cargo.toml`
- Modify: `app/Cargo.toml`

**Interfaces:**
- Produces: `CameraSource { name: String, rtsp_url: String }`.
- Produces: `PreviewFeed { name: String, video_id: String, whep_url: String }`.
- Produces: `ReaderConfig { script_url: String, user: String, password: String }`.
- Produces: `PreviewState::Ready { feeds: Vec<PreviewFeed>, reader: ReaderConfig }` and `PreviewState::Unavailable { message: String }`.
- Produces: `ConfigFile::create(sources: &[CameraSource], password: &str) -> Result<ConfigFile, Error>` for Task 2.

- [ ] **Step 1: Declare dependencies and write failing exact-configuration tests**

Move `tempfile = "3"` into `[workspace.dependencies]`, change the camera crate to `tempfile.workspace = true`, and add these app dependencies:

```toml
rand = "0.9"
serde = { workspace = true }
serde_json = { workspace = true }
tempfile = { workspace = true }
thiserror = { workspace = true }
```

Create the module files and add tests in `app/src/preview/config.rs` which construct two sources, including an RTSP URL containing YAML-sensitive characters, then assert:

Declare `mod preview;` in `app/src/main.rs` so the binary test target compiles and runs the module tests.

```rust
assert!(contents.contains("rtsp: false\n"));
assert!(contents.contains("webrtcAddress: 127.0.0.1:8889\n"));
assert!(contents.contains("webrtcLocalUDPAddress: 127.0.0.1:8189\n"));
assert!(contents.contains("webrtcIPsFromInterfaces: false\n"));
assert!(contents.contains("webrtcAdditionalHosts: [127.0.0.1]\n"));
assert!(contents.contains("      - action: read\n        path: camera-0\n"));
assert!(contents.contains("      - action: read\n        path: camera-1\n"));
assert!(contents.contains(&format!(
    "    source: {}\n",
    serde_json::to_string(&sources[1].rtsp_url).unwrap()
)));
assert!(!contents.contains("record: true"));
```

Also add tests proving the temporary file exists until drop, disappears after drop, and has mode `0600` on Unix.

- [ ] **Step 2: Run the focused tests and verify red**

Run: `cargo test -p app preview::config::tests -- --nocapture`

Expected: compilation fails because `ConfigFile`, `CameraSource`, or configuration rendering is not implemented.

- [ ] **Step 3: Implement the minimum exact configuration renderer**

Use `serde_json::to_string` for YAML-safe scalar quoting. Render all paths from their source index and include exactly these global settings:

```yaml
logDestinations: [stdout]
api: false
metrics: false
pprof: false
playback: false
rtsp: false
rtmp: false
hls: false
webrtc: true
webrtcAddress: 127.0.0.1:8889
webrtcAllowOrigins: ['*']
webrtcLocalUDPAddress: 127.0.0.1:8189
webrtcLocalTCPAddress: ''
webrtcIPsFromInterfaces: false
webrtcAdditionalHosts: [127.0.0.1]
srt: false
```

Give `app-preview` the generated password and one read permission per generated path. Give each path `sourceOnDemand: true`, ten-second start and close durations, `rtspTransport: tcp`, and `record: false`.

- [ ] **Step 4: Add metadata tests which prove source secrecy**

In `bridge.rs`, write a failing test around a pure metadata builder:

```rust
let source = CameraSource {
    name: "Workshop".into(),
    rtsp_url: "rtsp://camera-user:camera-pass@127.0.0.1/live".into(),
};
let preview = preview_metadata(&[source], "local-password".into());
let serialized = serde_json::to_string(&preview).unwrap();

assert!(serialized.contains("camera-0-video"));
assert!(serialized.contains("http://127.0.0.1:8889/camera-0/whep"));
assert!(!serialized.contains("camera-user"));
assert!(!serialized.contains("camera-pass"));
assert!(!serialized.contains("rtsp://"));
```

Derive only the traits the Dioxus context and component props require: `Clone`, `PartialEq`, `Serialize`, and `Deserialize` where used.

- [ ] **Step 5: Run focused tests and format**

Run: `cargo test -p app preview -- --nocapture`

Expected: all preview configuration and metadata tests pass.

Run: `cargo fmt --all --check`

Expected: exit 0.

- [ ] **Step 6: Commit preview configuration**

```bash
git add app/Cargo.toml app/src/main.rs app/src/preview camera/Cargo.toml Cargo.toml Cargo.lock
git commit -m "feat(app): configure WebRTC preview bridge"
```

### Task 2: MediaMTX Child Supervision

**Files:**
- Modify: `app/src/preview/bridge.rs`
- Modify: `app/src/preview/error.rs`
- Modify: `app/src/preview/mod.rs`

**Interfaces:**
- Consumes: Task 1's `CameraSource`, `PreviewState`, metadata builder, and `ConfigFile`.
- Produces: `start(sources: Vec<CameraSource>) -> Result<(PreviewState, Bridge), Error>`.
- Produces: `Bridge::stop(self) -> Result<(), Error>` and idempotent internal cleanup used by `Drop`.

- [ ] **Step 1: Write failing version and readiness tests**

Add focused tests for helpers with these assertions:

```rust
assert_eq!(validate_version(b"v1.18.2\n"), Ok(()));
assert!(matches!(
    validate_version(b"v1.18.1\n"),
    Err(Error::UnsupportedVersion(version)) if version == "v1.18.1"
));
```

Spawn the current test executable as a bounded live child, then test that readiness succeeds when a temporary TCP listener exists, reports an exited child, and times out within one second when no listener appears. Follow the existing test-child pattern in `camera/src/rtsp/mediamtx.rs` rather than adding a process abstraction.

- [ ] **Step 2: Run focused supervision tests and verify red**

Run: `cargo test -p app preview::bridge::tests -- --nocapture`

Expected: compilation fails because version validation, readiness polling, and `Bridge` cleanup are absent.

- [ ] **Step 3: Implement startup and readiness**

Implement synchronous startup before Dioxus launch:

```rust
pub(crate) fn start(
    sources: Vec<CameraSource>,
) -> Result<(PreviewState, Bridge), Error> {
    verify_version("mediamtx")?;
    let password = random_password();
    let config = ConfigFile::create(&sources, &password)?;
    let state = preview_metadata(&sources, password);
    let mut child = Command::new("mediamtx")
        .arg(config.path())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(Error::spawn)?;

    wait_until_ready(&mut child, WEBRTC_HTTP_ADDRESS, READINESS_TIMEOUT)?;
    Ok((state, Bridge::new(child, config)))
}
```

Poll every 50 ms, check `Child::try_wait` before each loopback connection attempt, and return a five-second timeout. If readiness fails, terminate and wait for the child before returning the startup error.

- [ ] **Step 4: Write failing cleanup tests**

Construct `Bridge` around live and already-exited test children. Assert `stop` returns, the process no longer exists, and dropping a live bridge performs the same cleanup. Store the child as `Option<Child>` so explicit stop and `Drop` share one idempotent cleanup path.

- [ ] **Step 5: Implement kill-and-wait cleanup**

For a live child, call `kill` and then `wait`. For an already-exited child, skip `kill` and still call `wait`. `Bridge::stop` reports errors; `Drop` writes cleanup errors to stderr because the UI is no longer available.

- [ ] **Step 6: Run app tests and strict checks**

Run: `cargo test -p app preview -- --nocapture`

Expected: all preview tests pass.

- [ ] **Step 7: Commit bridge supervision**

Commit only the process-supervision changes:

```bash
git add app/src/preview
git commit -m "feat(app): supervise MediaMTX preview process"
```

### Task 3: Dioxus WebRTC CameraFeed

**Files:**
- Modify: `app/src/components/camera/mod.rs`
- Create: `app/src/components/camera/feed.rs`
- Modify: `app/src/views/monitor/monitor.rs`
- Modify: `app/src/main.rs`

**Interfaces:**
- Consumes: Task 2's `start`, `Bridge`, `CameraSource`, `PreviewState`, `PreviewFeed`, and `ReaderConfig`.
- Produces: `CameraFeed(feed: PreviewFeed, reader: ReaderConfig) -> Element`.

- [ ] **Step 1: Replace the static camera with a compiling component shell**

Move the existing card markup into `feed.rs`, rename the component `CameraFeed`, and make the title use `feed.name`. Add the real video and accessible status elements:

```rust
video {
    id: feed.video_id.clone(),
    class: "h-full w-full object-cover",
    autoplay: true,
    muted: true,
    playsinline: true,
}
if let Some(message) = error() {
    p {
        class: "absolute inset-0 flex items-center justify-center bg-base-300/90 p-4 text-center",
        role: "status",
        "{message}"
    }
}
```

Run: `cargo check -p app`

Expected: pass with the static component shell before document interop is added.

- [ ] **Step 2: Add the bounded Dioxus eval program**

Use `document::Script` with `reader.script_url`, then start one static `document::eval` program from `use_effect`. Send a `serde_json::Value` containing `video_id`, `whep_url`, `user`, and `password` through `Eval::send`; never format those values into JavaScript source.

The static program must:

```javascript
const config = await dioxus.recv();
const deadline = Date.now() + 5000;
while (!window.MediaMTXWebRTCReader && Date.now() < deadline) {
  await new Promise((resolve) => setTimeout(resolve, 50));
}
if (!window.MediaMTXWebRTCReader) {
  dioxus.send("MediaMTX reader failed to load");
  return;
}
const video = document.getElementById(config.video_id);
const reader = new MediaMTXWebRTCReader({
  url: config.whep_url,
  user: config.user,
  pass: config.password,
  onError: (error) => dioxus.send(error),
  onTrack: (event) => {
    video.srcObject = event.streams[0];
    dioxus.send(null);
  },
});
await dioxus.recv();
reader.close();
video.srcObject = null;
```

Spawn one Rust receive loop that deserializes `Option<String>` and updates the status signal. Store the `Eval` handle in a signal and use `use_drop` to send the close command.

- [ ] **Step 3: Wire preview state into Monitor**

Consume `PreviewState` from root context. Render the actionable unavailable message for `PreviewState::Unavailable`; for ready state, render one `CameraFeed` per feed using the existing card layout.

Run: `cargo check -p app`

Expected: pass.

- [ ] **Step 4: Transfer bridge ownership to the Dioxus event loop**

In `main`, hardcode only this initial source:

```rust
CameraSource {
    name: "Workshop".into(),
    rtsp_url: "rtsp://127.0.0.1:8554/axis-media/media.amp".into(),
}
```

Convert startup failure to `PreviewState::Unavailable`, capture `Option<Bridge>` in `Config::with_custom_event_handler`, and on `Event::LoopDestroyed` call `bridge.stop()`. Launch with:

```rust
dioxus::LaunchBuilder::desktop()
    .with_context(preview_state)
    .with_cfg(config)
    .launch(App);
```

- [ ] **Step 5: Run app and workspace checks**

Run: `cargo test -p app`

Expected: all app unit tests pass.

Run: `cargo test --workspace`

Expected: all non-ignored workspace tests pass.

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: exit 0.

Run: `cargo fmt --all --check`

Expected: exit 0.

- [ ] **Step 6: Commit Dioxus playback**

```bash
git add app/src/components/camera app/src/views/monitor/monitor.rs app/src/main.rs
git commit -m "feat(app): render live WebRTC camera preview"
```

### Task 4: Real Desktop Acceptance And History Cleanup

**Files:**
- Modify only files whose acceptance failures expose a real defect; fold fixes into the relevant earlier commit.

**Interfaces:**
- Consumes: the complete bridge and `CameraFeed`.
- Produces: verified issue #47 behavior and two reviewable commits.

- [ ] **Step 1: Validate the generated configuration with MediaMTX v1.18.2**

Run the focused config test, then start MediaMTX once with a generated-equivalent configuration. Expected log lines include `MediaMTX v1.18.2`, `configuration loaded`, and a WebRTC listener on loopback; no unused listener opens.

- [ ] **Step 2: Run the virtual camera and real desktop app**

Start the camera from the Nix shell with HTTP `127.0.0.1:8080`, RTSP `127.0.0.1:8554`, and `camera/fixtures/default.mp4`. Start the app with `cargo run -p app` from the same shell.

Expected: the real macOS Dioxus/WKWebView window reaches ready state and visibly plays beyond ten seconds, covering at least two fixture loops.

- [ ] **Step 3: Exercise source recovery and cleanup**

Start the app before the camera and confirm an error appears. Start the camera without restarting the app and confirm playback recovers. Close the app and verify no MediaMTX child remains, TCP port 8889 and UDP port 8189 are released, and no preview config remains in the temporary directory.

- [ ] **Step 4: Inspect secrecy and jitter-buffer evidence**

Use the Dioxus eval channel during the acceptance run to inspect the rendered DOM, request URLs, and `RTCRtpReceiver.getStats()`. Confirm no RTSP URL or camera credential appears and average `jitterBufferDelay / jitterBufferEmittedCount` remains below one second after warm-up.

- [ ] **Step 5: Run final verification from a clean tree**

Run: `cargo test --workspace`

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Run: `cargo fmt --all --check`

Run: `just test-camera-stream`

Expected: all commands exit 0; the ignored RTSP external-player test passes when explicitly selected.

- [ ] **Step 6: Review and clean history**

Inspect `git status`, `git diff main...HEAD`, and `git log --oneline main..HEAD`. If acceptance produced fix commits, rebuild or squash them into the matching bridge or playback commit without changing `main`. Verify the resulting tree again after history rewriting.

- [ ] **Step 7: Update issue #47**

Mark only acceptance criteria supported by the completed checks. Keep broader #38 criteria unchanged.
