# Desktop App

The `app` crate is the Dioxus Desktop operator application. It owns one shared `Workflow` above the router, keeps recorder and analysis tasks alive across route changes, and presents the Monitor and Analyze routes. Reusable file, process, session, and analysis logic stays in `backend`.

See [`docs/architecture.md`](../docs/architecture.md) for the complete data flow and ownership boundaries.

## Run Locally

From the workspace root, enter `nix develop` in each terminal. Start both fixture-backed cameras in separate terminals:

```bash
just camera-1
just camera-2
```

Start the app in a third terminal:

```bash
just app
```

The checked-in `cameras.json` points to:

```text
rtsp://127.0.0.1:8554/axis-media/media.amp
rtsp://127.0.0.1:8555/axis-media/media.amp
```

Preview requires `mediamtx v1.18.2`, TCP port `8889`, and UDP port `8189`. The bridge pulls RTSP/TCP into loopback WHEP/WebRTC. Recording does not use the preview bridge: each session recorder connects directly to its configured camera URL. A preview can fail while recording continues, and recorder health does not claim preview health.

## Operator Flow

1. Monitor keeps exactly two previews mounted.
2. Start creates a new staging directory and waits for every configured camera to produce recorded media.
3. Active allows camera selection, analysis inclusion/exclusion, whole-second cadence changes, and Stop. Exclusion never stops preview or recording.
4. A camera interruption finalizes valid media, reports Reconnecting, and attempts a new RTSP/TCP stream-copy segment every second while the other camera continues.
5. Stop writes the end event, stops and reaps every FFmpeg child, validates and renames MKVs, then writes `recording-complete` only when finalization is sound.
6. Analyze refreshes completed sessions, shows persisted recording-gap warnings, and starts or resumes provider work only after an explicit Analyze action.

The root desktop event-loop owner retains `RecorderRuntime`, the preview `Bridge`, and the log guard. On normal window destruction it shuts down the recorder runtime, stops the preview bridge, and flushes logging. Root-scoped session and analysis tasks are independent of route component lifetime.

## Configuration

| Variable | Default | Validation and effect |
| --- | --- | --- |
| `LEO_CAMERA_CONFIG` | `./cameras.json` | Must contain exactly two unique nonzero IDs, nonblank names, `rtsp` URLs, and positive whole-second cadences. |
| `LEO_DATA_DIR` | `./data` | Creates direct `sessions/` and `logs/` directories and keeps each session portable under one root. |
| `LEO_RECORDER_TIMEOUT_SECS` | `10` | Positive bounded initial-readiness and FFmpeg network I/O timeout. |
| `OPENAI_API_KEY` | none | Required with `ANALYSIS_MODEL` before the UI enables provider analysis. |
| `ANALYSIS_MODEL` | none | Required provider model name. |
| `OPENAI_BASE_URL` | provider default | Optional endpoint override consumed when analysis constructs the provider. |
| `RUST_LOG` | `info` | Filter for compact stderr logs and daily JSON files. |

The default data layout is:

```text
data/
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
            |   `-- .attempt-<uuid>.partial.mkv
            `-- camera-2/
                `-- <segment-start-UTC-ms>.mkv
```

`analysis.json` appears after analysis planning. A partial file exists during an attempt and may remain after invalid media or an unclean exit; only numeric `.mkv` files are analyzed. `recording-complete` is a zero-byte marker created after a durable end event and successful recorder finalization.

Analysis probes local MKV segments, derives uncovered time ranges, omits unavailable samples, and continues with recovered post-gap media. It writes an initial zero-response checkpoint and atomically replaces the checkpoint after each successful five-frame-set batch. Temporary JPEGs are removed after use; the session directory should not gain MP4 downloads or persistent JPEGs.

## Styling

[`tailwind.css`](tailwind.css) is the Tailwind CSS and daisyUI source. Regenerate [`assets/tailwind.css`](assets/tailwind.css) from the workspace root with:

```bash
just css
```

## Verification

```bash
cargo test -p app
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The feature-scoped paid test is ignored and has a runtime environment assertion before provider, recorder-runtime, session, or workflow construction. Compile it only unless explicit paid-provider approval is given:

```bash
cargo test -p app --features paid-openai-test paid_openai_workflow::paid_openai_analyzes_one_local_application_session --no-run
```

Never set `LEO_RUN_PAID_OPENAI_TEST=1` or execute that test without separate approval. Normal tests and exact local media checks do not send provider requests.

## Current Limits

- Hard-crash cleanup, orphan detection, active-session recovery, and recording survival across app exit or laptop sleep are not implemented.
- An external SSD must already be mounted and selected through `LEO_DATA_DIR`; discovery, identity checks, capacity monitoring, and safe eject are absent.
- Retention, automatic deletion, export, settings, camera discovery, and video playback are absent.
- Physical-camera behavior and recorder timeout calibration require separate on-hardware acceptance.
