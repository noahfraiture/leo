# Desktop app

The `app` crate is the Dioxus Desktop operator application. Its implemented workflow is live camera monitoring: it supervises a local MediaMTX process, pulls camera RTSP streams, and exposes authenticated loopback WHEP/WebRTC streams to native `<video>` elements in the desktop webview.

See the [architecture document](../docs/architecture.md#desktop-app) for the full startup, data-flow, security and ownership model.

## Run locally

From the workspace root, start the virtual camera:

```bash
just camera
```

Start the app in another terminal:

```bash
just app
```

The app currently expects the development camera at:

```text
rtsp://127.0.0.1:8554/axis-media/media.amp
```

The Nix shell provides the required MediaMTX `v1.18.2`. Preview startup also requires TCP port `8889` and UDP port `8189` to be free.

If the bridge cannot start, the app still opens and the Monitor route displays the startup error. Check the MediaMTX version and `PATH` and the preview ports before restarting the app. A stopped camera is reported later by its feed because RTSP is pulled on demand.

## Styling

[`tailwind.css`](tailwind.css) is the Tailwind CSS and DaisyUI source. Dioxus compiles it while serving the app; [`assets/tailwind.css`](assets/tailwind.css) is generated output and should not be edited directly.

## Checks

Run the app tests from the workspace root:

```bash
nix develop --command cargo test -p app
```

Before merging app changes, also run:

```bash
nix develop --command cargo fmt --all --check
nix develop --command cargo clippy -p app --all-targets --all-features -- -D warnings
```
