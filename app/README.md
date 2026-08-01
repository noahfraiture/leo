# Leo

Rust workspace for local multi-camera exercise recording and analysis.

## Workspace

| Crate | Role |
| --- | --- |
| [`app`](app/) | Dioxus desktop operator application and RTSP-to-WebRTC preview bridge. |
| [`camera`](camera/src/main.rs) | Virtual Axis camera with a VAPIX-shaped HTTP API and fixture-backed RTSP stream. |
| [`synology`](synology/src/main.rs) | In-memory simulator for a small subset of the Surveillance Station API. |

The processes communicate over network sockets and do not share application state. See [`docs/architecture.md`](docs/architecture.md) for their data flow, lifecycle and ownership boundaries.

## Development

The Nix development shell supplies Rust, Dioxus CLI, MediaMTX, FFmpeg, VLC, Tailwind CSS and Just. The current flake targets Apple Silicon macOS (`aarch64-darwin`).

### Live preview

Start the virtual camera from the workspace root:

```bash
just camera
```

Then start the desktop app in another terminal:

```bash
just app
```

The camera serves HTTP on `127.0.0.1:8080` and RTSP on `127.0.0.1:8554`. The app pulls that RTSP stream through its own loopback MediaMTX process and renders it over WebRTC.

To inspect the camera stream independently:

```bash
just vlc
```

App-specific development notes are in [`app/README.md`](app/README.md).

### Synology simulator

The Synology simulator is independent of the app. With the virtual camera running, start it with:

```bash
nix develop --command cargo run -p synology -- \
  --address 127.0.0.1:5000 \
  --camera 127.0.0.1:8080
```

Repeat `--camera` for additional camera socket addresses. Reachability is a TCP connection check; the simulator does not record video.

### Tests

```bash
nix develop --command cargo test --workspace --all-targets --all-features
```

The real RTSP acceptance test is ignored by the normal suite because it requires external media processes. Run it with `just test-camera-stream`.
