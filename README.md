# Leo

Leo is a local desktop app for recording and analyzing a student's exercise from zero or more RTSP cameras. It records every configured stream directly to local storage, keeps session metadata beside the media, and analyzes completed sessions on demand.

## Workspace

| Crate | Responsibility |
| --- | --- |
| [`app`](app/) | Dioxus desktop UI and runtime orchestration. |
| [`backend`](backend/) | Recording, sessions, local media processing, and analysis. |
| [`camera`](camera/) | Fixture-backed virtual Axis camera for development and tests. |

See [`docs/architecture.md`](docs/architecture.md) for system design and [`docs/validation.md`](docs/validation.md) for local, provider, and physical-hardware checks.

## Run Locally

The Leo desktop app supports macOS and Linux. Windows is not supported.

The development environment targets Apple Silicon macOS. The two fixture-camera recipes below are one development example, not a deployment camera-count requirement. Run `nix develop` in each of three terminals, then start one process per terminal:

```bash
just camera-1
just camera-2
just app
```

On first launch, `just app` opens Settings. Add the fixture cameras with RTSP URLs `rtsp://127.0.0.1:8554/axis-media/media.amp` and `rtsp://127.0.0.1:8555/axis-media/media.amp`, save, and restart Leo to activate them. Invalid saved settings fail startup and must be fixed or removed before relaunching.

## Application Settings

Leo owns production configuration in Settings and one platform settings file:

| Setting | Purpose |
| --- | --- |
| Cameras | Generated immutable ID, name, RTSP URL, initial analysis inclusion, and whole-second sampling cadence for each camera. |
| Data root | Optional parent for `sessions/` and `logs/`; blank uses the platform default. |
| Recorder timeout | Initial all-camera readiness and bounded RTSP I/O timeout in seconds. |
| Analysis batching | Frame sets per prompt (default `5`) and repeated frame sets between prompts (default `0`). Each frame set can contain one image per camera; overlap repeats images and may increase provider cost. |
| OpenAI API key, model, and base URL | Provider credentials, model, and optional endpoint for explicit analysis. |
| Log level | `error`, `warn`, `info`, `debug`, or `trace` for stderr and JSON logs. |

| Platform | Settings file | Default data root |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Leo/settings.json` | `~/Library/Application Support/Leo/data/` |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/leo/settings.json` | `${XDG_DATA_HOME:-$HOME/.local/share}/leo/` |

Save is allowed while recording or analysis is active, but it never changes or interrupts the active runtime. Restart Leo to apply saved changes. Changing the data root does not move old sessions.

A blank provider key or model disables Analyze. Monitor and completed-session discovery remain available.

## Development

```bash
just test-unit
just test-e2e
just css
```

The normal checks are free and use local fixtures and a mock provider. Real-provider checks are paid, separately gated test processes documented in [`docs/validation.md`](docs/validation.md).
