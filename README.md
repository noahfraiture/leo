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

On first launch, `just app` opens Settings. Add the fixture cameras with RTSP URLs `rtsp://127.0.0.1:8554/axis-media/media.amp` and `rtsp://127.0.0.1:8555/axis-media/media.amp`, save, and restart Leo to activate them. If recording settings cannot load, Leo opens Settings with recovery guidance. Invalid monitoring or analysis settings leave recording available.

## Application Settings

Leo owns production configuration in Settings and one platform settings file:

| Setting | Purpose |
| --- | --- |
| Cameras | Generated immutable ID, name, RTSP URL, initial analysis inclusion, and a named initial monitoring profile for each camera. |
| Data root | Optional parent for `sessions/` and `logs/`; blank uses the platform default. |
| Recorder timeout | Initial all-camera readiness and bounded RTSP I/O timeout in seconds. |
| Monitoring profiles | Named sampling intervals in positive milliseconds. Choose profiles per camera or apply one to all cameras, before or during a session. Capture is always continuous. |
| Analysis profiles | Model, maximum images and time span per prompt, overlapping frame sets, optional image resizing, image detail, and output-token limit. Choose a profile when starting analysis. |
| Provider credentials | OpenAI API key and optional base URL. Models belong to analysis profiles. |
| Log level | `error`, `warn`, `info`, `debug`, or `trace` for stderr and JSON logs. |

| Platform | Settings file | Default data root |
| --- | --- | --- |
| macOS | `~/Library/Application Support/Leo/settings.json` | `~/Library/Application Support/Leo/data/` |
| Linux | `${XDG_CONFIG_HOME:-$HOME/.config}/leo/settings.json` | `${XDG_DATA_HOME:-$HOME/.local/share}/leo/` |

Save is allowed while recording or analysis is active, but it never changes or interrupts the active runtime. Restart Leo to apply saved changes. Changing the data root does not move old sessions.

The initial monitoring profile samples every 1,000 ms. The initial analysis profile allows 16 images, a 7,000 ms span, and two overlapping frame sets, with original image dimensions and provider-default detail. Its model is blank until configured. These are editable starting values, not evaluated recommendations.

Monitoring and analysis definitions use stable IDs and unique nonblank names. The session snapshots monitoring definitions; analysis snapshots its selected profile. Resume uses the saved profile even after Settings changes. Use "New analysis" and confirm discarding the old checkpoint to choose a different profile for that session.

If monitoring metadata cannot be saved, camera recording continues, controls show the last saved selections, and Stop still works. After Stop, retained recordings with incomplete metadata appear in Analyze with an option to open their folder. A new recording session is allowed when media finalization succeeded. Actual recorder failures still require recovery.

This version requires settings schema 3, event schema 2, and checkpoint schema 3. There is no migration of old metadata; existing recording files are retained.

## Development

```bash
just test-unit
just test-e2e
just css
```

The normal checks are free and use local fixtures and a mock provider. Real-provider checks are paid, separately gated test processes documented in [`docs/validation.md`](docs/validation.md).
