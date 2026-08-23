# Leo

Leo is a local desktop app for recording and analyzing a student's exercise from two RTSP cameras. It records both streams directly to local storage, keeps session metadata beside the media, and analyzes completed sessions on demand.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`app`](app/) | Dioxus desktop UI and runtime orchestration. |
| [`backend`](backend/) | Recording, sessions, local media processing, and analysis. |
| [`camera`](camera/) | Fixture-backed virtual Axis camera for development and tests. |

See [`docs/architecture.md`](docs/architecture.md) for system design and [`docs/validation.md`](docs/validation.md) for local, provider, and physical-hardware checks.

## Run Locally

The development environment targets Apple Silicon macOS. Run `nix develop` in each of three terminals, then start one process per terminal:

```bash
just camera-1
just camera-2
just app
```

The checked-in `cameras.json` points the app at the two local fixture cameras.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `LEO_CAMERA_CONFIG` | `./cameras.json` | Exactly two camera IDs, names, RTSP URLs, and sampling settings. |
| `LEO_DATA_DIR` | `./data` | Parent directory for session media, metadata, analysis, and logs. |
| `LEO_RECORDER_TIMEOUT_SECS` | `10` | Initial recorder readiness and RTSP I/O timeout. |
| `OPENAI_API_KEY` | none | Required only for explicit provider analysis. |
| `ANALYSIS_MODEL` | none | Provider model used for analysis. |
| `OPENAI_BASE_URL` | provider default | Optional provider endpoint override. |
| `RUST_LOG` | `info` | Console and JSON log filter. |

## Development

```bash
just test-unit
just test-e2e
just css
```

The normal checks use local fixtures and a mock provider. Real-provider checks are paid and require the explicit opt-in documented in [`docs/validation.md`](docs/validation.md).
