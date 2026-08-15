# Leo

Leo is a local desktop workflow for recording and analyzing a student's exercise from two RTSP cameras. Recording is scoped to an operator session and stored directly on the host with its metadata and analysis checkpoint.

## Workspace

The Cargo workspace has exactly three crates:

| Crate | Responsibility |
| --- | --- |
| [`app`](app/) | Dioxus desktop UI, two-camera preview, workflow state, runtime ownership, and background tasks. |
| [`backend`](backend/) | Durable session metadata, supervised FFmpeg recording, local segment discovery, and resumable analysis. |
| [`camera`](camera/) | Fixture-backed Axis-shaped virtual camera for local development and media checks. |

See [`docs/architecture.md`](docs/architecture.md) for lifecycle, storage, and failure semantics.

## Local Workflow

The Nix development shell supplies Rust, Dioxus CLI, MediaMTX `v1.18.2`, FFmpeg, FFprobe, Tailwind CSS, daisyUI, and Just. The current flake targets Apple Silicon macOS (`aarch64-darwin`).

From the workspace root, enter `nix develop` in each of three terminals, then run:

```bash
just camera-1
just camera-2
just app
```

The virtual cameras serve RTSP at `127.0.0.1:8554` and `127.0.0.1:8555`. Leo renders both previews through a separate loopback MediaMTX bridge. Start requires both direct FFmpeg RTSP/TCP recorders to receive media, even when a camera is excluded from analysis. Participation and sampling cadence affect analysis only; both cameras continue recording until Stop.

Stop appends the session end, stops and reaps both recorders, validates and finalizes their Matroska stream-copy segments, and then writes the zero-byte `recording-complete` marker. Analyze discovers only marked sessions, reads finalized MKVs directly, skips recording gaps while preserving later media, and saves resumable progress and results in `analysis.json`.

## Configuration

Paths are resolved from the process working directory.

| Variable | Default | Purpose |
| --- | --- | --- |
| `LEO_CAMERA_CONFIG` | `./cameras.json` | Exactly two camera IDs, names, RTSP URLs, initial participation values, and whole-second sampling cadences. |
| `LEO_DATA_DIR` | `./data` | Parent of portable `sessions/` and daily `logs/`. May be an already-mounted local volume. |
| `LEO_RECORDER_TIMEOUT_SECS` | `10` | Positive initial-readiness and RTSP I/O timeout. Reconnect delay is one second; Stop allows five seconds before forced termination. |
| `OPENAI_API_KEY` | none | Required only for explicit provider analysis. |
| `ANALYSIS_MODEL` | none | Required only for explicit provider analysis; no model is hard-coded. |
| `OPENAI_BASE_URL` | provider default | Optional provider endpoint override. |
| `RUST_LOG` | `info` | Console and daily JSON log filter. |

Logs are written to stderr and `<LEO_DATA_DIR>/logs/leo.jsonl.<date>`. Leo does not log credentials, full RTSP URLs, checklists, image bytes, or model request bodies.

## Checks

Normal checks do not execute ignored media or paid tests:

```bash
cargo test --workspace --all-targets
cargo test --workspace --all-targets --all-features --no-run
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Each media command selects one approved ignored test by exact name:

```bash
nix develop --command cargo test -p backend analysis::video::extractor::tests::extracts_fixture_frame_as_jpeg -- --ignored --exact
just test-local-analysis
just test-camera-stream
just test-host-recording
just test-host-reconnect
```

The paid app test may be compiled, never executed, without explicit approval:

```bash
cargo test -p app --features paid-openai-test paid_openai_workflow::paid_openai_analyzes_one_local_application_session --no-run
```

Do not set `LEO_RUN_PAID_OPENAI_TEST=1` or run the ignored paid test unless the user separately approves the provider request and cost.

## Limits

- Normal shutdown owns, interrupts, kills when necessary, reaps, and joins recorder and preview processes. A hard app crash, forced termination, sleep, or power loss can leave partial files or child processes; active-session recovery is not implemented.
- `LEO_DATA_DIR` can target an already-mounted external SSD, but Leo does not discover, validate, mount, eject, or monitor one.
- Disk-capacity monitoring, automatic retention, deletion, export, and playback are not implemented.
- Physical cameras and timeout calibration have not been accepted by the automated local fixtures.
- Only one recording session and one analysis may run at a time.

App-specific operation and styling notes are in [`app/README.md`](app/README.md).
