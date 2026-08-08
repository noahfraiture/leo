# Desktop RTSP Preview Bridge Design

Date: 2026-07-26

Issue: [#47](https://github.com/noahfraiture/leo/issues/47)

Parent issue: [#38](https://github.com/noahfraiture/leo/issues/38)

## Goal

Prove that the real Dioxus desktop app can render one virtual camera with sub-second-capable WebRTC through an app-owned MediaMTX process.

The first increment proves this path:

```text
virtual camera
      |
      | RTSP/H.264
      v
app-owned MediaMTX v1.18.2
      |
      | loopback WHEP/WebRTC
      v
Dioxus <video>
```

MediaMTX continues to own RTSP, WHEP, ICE, reconnection, and media transport. Dioxus owns the video element and uses its document interop API to connect MediaMTX's browser reader to that element.

## Scope

- One hardcoded virtual-camera RTSP source known when the app starts.
- One app-owned MediaMTX child process.
- One generated MediaMTX path and one reusable `CameraFeed` component.
- WebRTC services bound to loopback.
- In-memory preview metadata.
- Basic bridge startup errors, source errors, and deterministic child cleanup.
- Development through the Nix shell, which provides MediaMTX v1.18.2.

Multi-camera layout, runtime camera registration, physical-camera authentication, controls, detailed health, automatic sidecar restart, recording, and packaged-app delivery remain in #38 or focused follow-up issues. This increment does not close #38.

## Why The Player Needs Document Interop

Dioxus Desktop 0.7.9 does not expose WHEP, `RTCPeerConnection`, `MediaStream`, or the video element's `srcObject` as Rust APIs. Its video element supports ordinary attributes such as `src`, but browsers do not play WHEP through that attribute.

An HTTP iframe is not viable. Dioxus Desktop intercepts HTTP and HTTPS navigation before its user navigation handler, and Wry applies that policy to iframe navigation on macOS. The MediaMTX player URL would therefore be cancelled or opened externally.

Dioxus does provide the minimum required bridge:

- `document::Script` loads MediaMTX's own `reader.js` from the verified sidecar.
- `document::eval` runs a small static interop script after the video element mounts.
- `Eval::send` passes structured preview configuration from Rust without interpolating values into JavaScript source.
- `Eval::recv` returns ready and error events to Rust.
- `use_drop` tells the interop script to close its reader when `CameraFeed` unmounts.

The app does not vendor `reader.js` and does not implement WHEP, SDP, ICE, retry logic, or decoding. The only app-owned JavaScript creates `MediaMTXWebRTCReader`, assigns the received stream to `video.srcObject`, and relays lifecycle events through Dioxus.

## Data Types

At startup the app has one source:

```text
CameraSource {
    name,
    rtsp_url,
}
```

The bridge returns cloneable UI state:

```text
PreviewState::Ready {
    feeds: Vec<PreviewFeed>,
    reader_script_url,
    local_user,
    local_password,
}

PreviewState::Unavailable {
    message,
}
```

Each `PreviewFeed` contains a camera name, generated DOM ID, and loopback WHEP URL. It never contains the source RTSP URL or camera credentials.

Generated path and DOM IDs use the source index, such as `camera-0`. Operator-provided names are display text only and never become configuration keys or element IDs.

## Bridge Startup

Before launching Dioxus, the app:

1. Runs `mediamtx --version`, trims whitespace, and requires the exact output `v1.18.2`.
2. Generates a cryptographically random process-local MediaMTX password for the fixed local user `app-preview`.
3. Safely quotes the RTSP source URL and local password as YAML string values.
4. Writes a `0600` temporary MediaMTX configuration.
5. Starts `mediamtx` from `PATH` with that configuration.
6. Waits up to five seconds for `127.0.0.1:8889`, checking for child exit before every connection attempt.
7. Builds `PreviewState::Ready` only after the listener is reachable.

This increment deliberately requires `nix develop`; locating or bundling MediaMTX for a packaged desktop app is deferred until desktop distribution work begins.

The source is on demand, so an unavailable camera does not prevent bridge readiness. Source errors appear in `CameraFeed` and MediaMTX retries them through its reader flow.

## MediaMTX Configuration

The generated configuration explicitly disables every unused server and recording:

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

authInternalUsers:
  - user: app-preview
    pass: GENERATED_PROCESS_LOCAL_PASSWORD
    ips: [127.0.0.1, '::1']
    permissions:
      - action: read
        path: camera-0

paths:
  camera-0:
    source: SAFELY_QUOTED_RTSP_URL
    sourceOnDemand: true
    sourceOnDemandStartTimeout: 10s
    sourceOnDemandCloseAfter: 10s
    rtspTransport: tcp
    record: false
```

Wildcard CORS is acceptable only because the WebRTC listeners are loopback-only and every read requires the random process-local credential. Camera credentials remain in Rust and the `0600` temporary configuration; JavaScript receives only the local bridge credential.

`sourceOnDemandCloseAfter` means MediaMTX can retain the RTSP source for up to ten seconds after the final preview reader disconnects. The design does not claim immediate source shutdown.

The app never prints the generated configuration or source URL. MediaMTX output remains visible for development; authenticated physical-camera logging must be reviewed when physical authentication enters scope.

## Dioxus Launch And Child Ownership

`dioxus::launch` never returns, so cleanup cannot run after it. The app instead uses `LaunchBuilder`:

1. Start the bridge and separate its cloneable `PreviewState` from the child owner.
2. Pass `PreviewState` through `LaunchBuilder::with_context`.
3. Put `Option<Bridge>` in `dioxus_desktop::Config::with_custom_event_handler`.
4. On `Event::LoopDestroyed`, take the bridge, terminate MediaMTX, and wait for it to exit.
5. Give `Bridge` the same kill-and-wait behavior in `Drop` as a fallback for startup errors and unwinding.

If bridge startup fails, the app still launches with `PreviewState::Unavailable` and renders the actionable error. No child owner is installed.

The bridge keeps the temporary configuration alive for exactly as long as the child. Normal app closure stops and reaps the child before dropping and removing the file. `SIGKILL` cannot run cleanup; `0600` permissions limit exposure if the operating system leaves the temporary file behind.

## CameraFeed

`CameraFeed` receives one `PreviewFeed` and the local reader configuration. It renders:

```html
<video id="camera-0-video" autoplay muted playsinline></video>
<p role="status">current reader error, when present</p>
```

The app uses `document::Script` once to load:

```text
http://127.0.0.1:8889/camera-0/reader.js
```

After mount, `CameraFeed` starts one static `document::eval` program. Rust sends the element ID, WHEP URL, local user, and local password through `Eval::send`. The program waits a bounded time for `MediaMTXWebRTCReader` to load, then:

- creates the reader with the WHEP URL and local Basic authentication;
- sends reader errors to Rust with `dioxus.send`;
- assigns the first received stream to `video.srcObject`;
- reports readiness to Rust;
- waits for a close command from Rust and calls `reader.close()`.

Rust receives status messages and updates the error paragraph through normal Dioxus state. `use_drop` sends the close command when the component unmounts. Values are serialized through Dioxus channels rather than inserted into JavaScript source or DOM attributes.

MediaMTX v1.18.2's built-in `disablepictureinpicture` query option is not used because that release writes the wrong case-sensitive DOM property. Picture-in-picture policy is deferred with other polished player controls.

## Errors And Recovery

Bridge startup distinguishes:

- missing `mediamtx` executable;
- unsupported MediaMTX version;
- temporary configuration creation or write failure;
- child spawn failure;
- MediaMTX exit before readiness, including port conflicts and invalid configuration;
- WebRTC listener readiness timeout;
- failure to terminate or wait for the child during cleanup.

Startup errors use `thiserror` in `preview/error.rs` and become one actionable `PreviewState::Unavailable` message for the UI. Cleanup errors occur while the app is exiting and are written to stderr instead.

Feed-level errors include failure to load `reader.js`, unavailable source, unsupported codec, and failed WebRTC connection. `CameraFeed` shows the latest reader error. MediaMTX's reader retries source and WebRTC failures; the app does not duplicate that retry loop.

Unexpected MediaMTX exit after startup leaves feeds showing connection errors. Automatic sidecar restart and independent bridge health belong to #38.

## Validation

### Unit Tests

- Render the exact WebRTC-only configuration and verify every unused server plus recording is disabled.
- Verify RTSP URLs and generated passwords are safely quoted.
- Verify generated path, DOM, reader-script, and WHEP URLs for each source index.
- Verify read permissions are limited to generated paths.
- Verify preview metadata contains no RTSP source URL or camera credential.
- Verify the temporary configuration mode is `0600` on Unix.
- Exercise missing executable, version mismatch, early exit, timeout, live-child cleanup, and already-exited child cleanup with fake child commands and temporary listeners.

### Desktop Integration Check

The acceptance check runs the actual macOS Dioxus/WKWebView application, not a standalone Chromium page:

1. Start the issue #22 virtual camera with the committed H.264 fixture.
2. Start the desktop app through `nix develop`.
3. Verify `CameraFeed` reaches ready state and plays across at least two fixture loops.
4. After warm-up, inspect `RTCRtpReceiver.getStats()` through the existing Dioxus eval channel and require average jitter-buffer delay below one second on loopback.
5. Start the app while the virtual camera is unavailable, then start the camera and verify the visible feed recovers without restarting the app.
6. Inspect rendered DOM and preview request URLs and verify they contain neither the RTSP source URL nor camera credentials.
7. Close the app and verify MediaMTX exits, TCP port 8889 and UDP port 8189 are released, and the temporary configuration disappears.

This check proves the Dioxus-to-WebRTC boundary and bounds the browser's loopback jitter-buffer delay. End-to-end control-to-preview latency belongs with fixture switching in #45. Physical Axis compatibility, multi-camera recovery, automatic sidecar restart, authentication UX, and richer status presentation remain completion criteria of #38 or later focused issues.

## Deferred Work

- #38 builds the multi-camera overview, focused view, controls, independent health presentation, warnings, and sidecar recovery around `CameraFeed`.
- #45 adds virtual-camera fixture switching without changing the source RTSP URL.
- Desktop distribution work bundles or otherwise locates the exact MediaMTX binary outside the Nix shell.
- Physical-camera integration adds credential storage, authenticated-source log validation, and browser-compatible H.264 profile checks.
