# Operational Pilot

Use this runbook on the intended Apple Silicon macOS host and SSD. Shell blocks run from `/Users/noah/Projects/leo/app`; replace every `<...>` value before running a block. Keep credential-bearing RTSP URLs out of the evidence directory and stop at the first failed gate.

## Acceptance Boundary

Recording acceptance requires reviewed evidence for all of these gates on the intended SSD:

- [ ] Failed start recovers cleanly.
- [ ] Nominal recording completes.
- [ ] Camera reconnect completes.
- [ ] Clean exit and restart rediscover completed sessions.
- [ ] One full expected class-duration soak completes.

Keep provider variables absent throughout recording acceptance. Do not create `analysis.json`; it must remain absent from every pilot session until recording sign-off.

## Site Decisions

Record these site-owned values in the operator record before touching hardware. They cannot be inferred from this repository.

| Decision | Required record |
| --- | --- |
| Cameras | Stable nonzero ID, model, and firmware for each physical camera |
| Streams | Vendor-confirmed RTSP URL for each camera, stored as a secret |
| SSD | Expected volume name, Volume UUID, and filesystem |
| Capacity | Minimum free-space margin that must remain throughout the soak |
| Duration | Full expected class duration |

Do not substitute the fixture URL, frame rate, or capacity assumptions for site decisions.

## Preparation

Create one UTC-named evidence directory and keep its path in every operator shell:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/$PILOT_RUN_ID"
mkdir -p "$PILOT_EVIDENCE"

nix develop --command mediamtx --version 2>&1 | tee "$PILOT_EVIDENCE/mediamtx-version.txt"
nix develop --command ffmpeg -version 2>&1 | tee "$PILOT_EVIDENCE/ffmpeg-version.txt"
nix develop --command ffprobe -version 2>&1 | tee "$PILOT_EVIDENCE/ffprobe-version.txt"

if lsof -nP -iTCP:8889 -sTCP:LISTEN; then
  printf 'TCP port 8889 is in use\n' >&2
  exit 1
fi
if lsof -nP -iUDP:8189; then
  printf 'UDP port 8189 is in use\n' >&2
  exit 1
fi

nix develop --command just test-unit 2>&1 | tee "$PILOT_EVIDENCE/test-unit.log"
nix develop --command just test-e2e 2>&1 | tee "$PILOT_EVIDENCE/test-e2e.log"
```

- [ ] MediaMTX reports `v1.18.2`; FFmpeg and FFprobe versions are recorded.
- [ ] Both test recipes exit zero. The E2E recipe uses fixtures and a loopback model mock.
- [ ] `8889/TCP` and `8189/UDP` were free before the E2E run.

## Camera Bench

Connect one physical camera at a time. Enter URLs without placing credentials in shell history, then collect one bounded RTSP/TCP probe per camera:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"

IFS= read -r -s -p 'Camera 1 RTSP URL: ' CAMERA_1_RTSP
printf '\n'
IFS= read -r -s -p 'Camera 2 RTSP URL: ' CAMERA_2_RTSP
printf '\n'

nix develop --command ffprobe -v error -rtsp_transport tcp -timeout 10000000 \
  -read_intervals '%+12' -select_streams v -count_packets \
  -show_entries 'stream=index,codec_name,nb_read_packets' -of json \
  "$CAMERA_1_RTSP" 2>"$PILOT_EVIDENCE/camera-1.ffprobe.err" \
  | tee "$PILOT_EVIDENCE/camera-1.ffprobe.json"

nix develop --command ffprobe -v error -rtsp_transport tcp -timeout 10000000 \
  -read_intervals '%+12' -select_streams v -count_packets \
  -show_entries 'stream=index,codec_name,nb_read_packets' -of json \
  "$CAMERA_2_RTSP" 2>"$PILOT_EVIDENCE/camera-2.ffprobe.err" \
  | tee "$PILOT_EVIDENCE/camera-2.ffprobe.json"

unset CAMERA_1_RTSP CAMERA_2_RTSP
```

Each 12-second result must contain exactly one video stream, `codec_name` equal to `h264`, and a positive `nb_read_packets`. Do not impose the fixture-specific 15 fps or 150-packet threshold. The nominal app session, not a second bench reader, accepts concurrent preview and recording connections.

## Storage And Preview

Set the approved values, inspect the mounted device, and stop before creating anything unless the reported mount point, Volume UUID, and filesystem exactly match the site record. Confirm `df` meets the site-defined free-space margin.

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
export SSD_MOUNT="/Volumes/<exact-volume-name>"
export EXPECTED_VOLUME_UUID="<expected-volume-uuid>"
export EXPECTED_FILESYSTEM="<expected-filesystem>"
export LEO_DATA_DIR="$SSD_MOUNT/leo"
export LEO_CAMERA_CONFIG="/absolute/path/to/pilot-cameras.json"

test -d "$SSD_MOUNT"
test ! -L "$SSD_MOUNT"
diskutil info "$SSD_MOUNT" | tee "$PILOT_EVIDENCE/ssd-info.txt"
df -h "$SSD_MOUNT" | tee "$PILOT_EVIDENCE/ssd-space-before.txt"
```

Create the protected camera configuration at `LEO_CAMERA_CONFIG`. It must have exactly two rows like these, with two unique nonzero IDs, nonblank names, vendor-confirmed RTSP URLs, and positive whole-second sampling cadences:

```json
[
  {
    "id": <camera-1-nonzero-id>,
    "name": "<camera-1-name>",
    "rtspUrl": "rtsp://<camera-1-vendor-confirmed-url>",
    "enabled": true,
    "sampleEveryMs": 1000
  },
  {
    "id": <camera-2-different-nonzero-id>,
    "name": "<camera-2-name>",
    "rtspUrl": "rtsp://<camera-2-vendor-confirmed-url>",
    "enabled": true,
    "sampleEveryMs": 1000
  }
]
```

Only after the identity review passes and the configuration exists, rerun the identity inspection immediately before this block. Reject existing symlinks before creating and probing the direct directories:

```bash
cd /Users/noah/Projects/leo/app
set -eu
: "${SSD_MOUNT:?set SSD_MOUNT after identity review}"
: "${LEO_DATA_DIR:?set LEO_DATA_DIR after identity review}"
: "${LEO_CAMERA_CONFIG:?set LEO_CAMERA_CONFIG}"
test -d "$SSD_MOUNT"
test ! -L "$SSD_MOUNT"
diskutil info "$SSD_MOUNT" > /dev/null

for directory in "$LEO_DATA_DIR" "$LEO_DATA_DIR/sessions" "$LEO_DATA_DIR/logs"; do
  if [ -e "$directory" ] || [ -L "$directory" ]; then
    test -d "$directory"
    test ! -L "$directory"
  fi
done

mkdir -p "$LEO_DATA_DIR/sessions" "$LEO_DATA_DIR/logs"
test -d "$LEO_DATA_DIR" && test ! -L "$LEO_DATA_DIR"
test -d "$LEO_DATA_DIR/sessions" && test ! -L "$LEO_DATA_DIR/sessions"
test -d "$LEO_DATA_DIR/logs" && test ! -L "$LEO_DATA_DIR/logs"

probe="$LEO_DATA_DIR/.leo-write-probe.$$"
(umask 077; printf 'x' > "$probe")
test -f "$probe"
rm "$probe"

test -f "$LEO_CAMERA_CONFIG"
test ! -L "$LEO_CAMERA_CONFIG"
chmod 600 "$LEO_CAMERA_CONFIG"
```

Launch with explicit paths and timeout, removing all provider and paid-test variables even if the shell loaded them from `.env`:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${LEO_CAMERA_CONFIG:?set LEO_CAMERA_CONFIG}"
: "${LEO_DATA_DIR:?set LEO_DATA_DIR}"

env \
  -u OPENAI_API_KEY \
  -u ANALYSIS_MODEL \
  -u OPENAI_BASE_URL \
  -u LEO_E2E_REAL_OPENAI \
  -u LEO_RUN_PAID_OPENAI_TEST \
  LEO_CAMERA_CONFIG="$LEO_CAMERA_CONFIG" \
  LEO_DATA_DIR="$LEO_DATA_DIR" \
  LEO_RECORDER_TIMEOUT_SECS=10 \
  RUST_LOG=info \
  nix develop --command just app 2>&1 | tee "$PILOT_EVIDENCE/app-console.log"
```

- [ ] Startup reports the reviewed SSD paths and `Session idle`.
- [ ] Both previews show moving video; `preview ready` alone is insufficient.
- [ ] Analyze is disabled because provider configuration is absent.

## Failed Start

Record the current completed-session list and direct session entries before disconnecting anything:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${LEO_DATA_DIR:?set LEO_DATA_DIR}"

find "$LEO_DATA_DIR/sessions" -mindepth 1 -maxdepth 1 -print \
  | LC_ALL=C sort \
  | tee "$PILOT_EVIDENCE/failed-start-sessions-before.txt"
```

1. Disconnect only camera 2 before clicking Start.
2. Click Start and wait for the failure; restore camera 2 afterward.
3. Require recovery to `Session idle`, no new completed session, no completion marker, and no retained staging directory when cleanup is sound.
4. Confirm camera 1's recorder was stopped. The app-owned preview MediaMTX process may remain while Leo is open, but no Leo recording FFmpeg process may remain.

Capture and compare the after-state, then collect a credential-free process list:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${LEO_DATA_DIR:?set LEO_DATA_DIR}"

find "$LEO_DATA_DIR/sessions" -mindepth 1 -maxdepth 1 -print \
  | LC_ALL=C sort \
  | tee "$PILOT_EVIDENCE/failed-start-sessions-after.txt"
cmp "$PILOT_EVIDENCE/failed-start-sessions-before.txt" \
  "$PILOT_EVIDENCE/failed-start-sessions-after.txt"
{ pgrep -lx 'ffmpeg|ffprobe|mediamtx' || true; } \
  | tee "$PILOT_EVIDENCE/failed-start-processes.txt"
```

`Session faulted`, a failed comparison, any retained new directory, or any Leo-owned recorder process fails the gate. Preserve the directory and all evidence without deleting or repairing it.

## Nominal Session

1. Start a fresh session and require both previews moving, `Session active`, and two `Recording` statuses.
2. Change camera 1 cadence to 2 seconds once.
3. Exclude camera 2 from analysis, wait 15 seconds, then include it again. Both cameras must keep previewing and recording.
4. Continue until at least two minutes have elapsed, then Stop normally and wait for `Session idle`.
5. Open Analyze only to confirm completed-session discovery; do not start analysis. Record the displayed session directory.

Set the nominal values in a second operator shell, then run the artifact block below:

```bash
cd /Users/noah/Projects/leo/app
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
export LEO_DATA_DIR="/Volumes/<exact-volume-name>/leo"
export CAMERA_1_ID="<camera-1-nonzero-id>"
export CAMERA_2_ID="<camera-2-different-nonzero-id>"
export SESSION_LABEL="nominal"
export EXPECTED_EVENT_LINES=5
export SESSION_DIR="$LEO_DATA_DIR/sessions/<displayed-start-request-UTC-ms>"
```

Use this same artifact block for nominal, reconnect, and soak sessions after setting their section-specific values:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${CAMERA_1_ID:?set CAMERA_1_ID}"
: "${CAMERA_2_ID:?set CAMERA_2_ID}"
: "${SESSION_LABEL:?set SESSION_LABEL}"
: "${EXPECTED_EVENT_LINES:?set EXPECTED_EVENT_LINES}"
: "${SESSION_DIR:?set SESSION_DIR}"
case "$CAMERA_1_ID" in 0|*[!0-9]*) exit 1 ;; esac
case "$CAMERA_2_ID" in 0|*[!0-9]*) exit 1 ;; esac
test "$CAMERA_1_ID" != "$CAMERA_2_ID"

test -d "$SESSION_DIR" && test ! -L "$SESSION_DIR"
test -f "$SESSION_DIR/events.jsonl" && test ! -L "$SESSION_DIR/events.jsonl"
test -f "$SESSION_DIR/recording-complete" && test ! -L "$SESSION_DIR/recording-complete"
test "$(stat -f '%z' "$SESSION_DIR/recording-complete")" -eq 0
test "$(wc -l < "$SESSION_DIR/events.jsonl")" -eq "$EXPECTED_EVENT_LINES"
test ! -e "$SESSION_DIR/analysis.json"

stat -f 'size=%z path=%N' "$SESSION_DIR/recording-complete" \
  | tee "$PILOT_EVIDENCE/$SESSION_LABEL-completion-marker.txt"
nl -ba "$SESSION_DIR/events.jsonl" \
  | tee "$PILOT_EVIDENCE/$SESSION_LABEL-events-numbered.txt"
cp "$SESSION_DIR/events.jsonl" "$PILOT_EVIDENCE/$SESSION_LABEL-events.jsonl"

for camera_id in "$CAMERA_1_ID" "$CAMERA_2_ID"; do
  camera_dir="$SESSION_DIR/recordings/camera-$camera_id"
  manifest="$PILOT_EVIDENCE/$SESSION_LABEL-camera-$camera_id-segments.txt"
  test -d "$camera_dir" && test ! -L "$camera_dir"
  : > "$manifest"
  count=0
  for file in "$camera_dir"/*.mkv; do
    test -f "$file" || continue
    test ! -L "$file"
    name="${file##*/}"
    stem="${name%.mkv}"
    case "$stem" in
      ''|*[!0-9]*) exit 1 ;;
    esac
    count=$((count + 1))
    printf '%s\n' "$file" >> "$manifest"
    nix develop --command ffprobe -v error -select_streams v -count_packets \
      -show_entries 'stream=index,codec_name,nb_read_packets:format=format_name,start_time,duration' \
      -of json "$file" 2> "$PILOT_EVIDENCE/$SESSION_LABEL-camera-$camera_id-$stem.ffprobe.err" \
      > "$PILOT_EVIDENCE/$SESSION_LABEL-camera-$camera_id-$stem.ffprobe.json"
  done
  test "$count" -ge 1
done

for partial in "$SESSION_DIR"/recordings/camera-*/.attempt-*.partial.mkv; do
  test ! -e "$partial"
done
printf 'no partial attempts\n' > "$PILOT_EVIDENCE/$SESSION_LABEL-no-partials.txt"
```

For every generated FFprobe JSON, require exactly one video stream, H.264 codec, Matroska format, positive packet count, and positive duration. FFprobe must exit zero. For nominal, the five numbered events must be start, cadence change, camera 2 exclusion, camera 2 inclusion, and end, in that order. Any partial attempt or `analysis.json` fails the gate.

## Reconnect Session

Keep `LEO_RECORDER_TIMEOUT_SECS=10`; do not change it for this run.

1. Start a fresh session and wait for both cameras to report `Recording`.
2. Disconnect only camera 2 after useful media has accumulated. Camera 1 must remain `Recording` while camera 2 shows `Reconnecting`.
3. Restore camera 2 at the same URL and require it to return to `Recording`.
4. Record UTC and epoch-second timestamps for disconnect, detection, restore, and recovery; calculate detection and recovery latency.
5. Stop normally, confirm completed-session discovery, and rerun the Nominal Session artifact block with `SESSION_LABEL=reconnect`, `EXPECTED_EVENT_LINES=2`, and the fresh `SESSION_DIR`.

Use this helper at each of the four moments, not all at once:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
record_time() {
  date -u "+event=$1 utc=%Y-%m-%dT%H:%M:%SZ epoch_s=%s" \
    | tee -a "$PILOT_EVIDENCE/reconnect-timings.txt"
}
```

Run `record_time camera_2_disconnected`, `record_time reconnecting_observed`, `record_time camera_2_restored`, and `record_time recording_observed` at their respective moments.

Camera 2 must have at least two playable finalized numeric segments. In filename order, calculate and record for each adjacent pair:

```bash
cd /Users/noah/Projects/leo/app
set -eu
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${CAMERA_2_ID:?set CAMERA_2_ID}"
test "$(wc -l < "$PILOT_EVIDENCE/reconnect-camera-$CAMERA_2_ID-segments.txt")" -ge 2
```

```text
media_span_ms = ceil(format.duration * 1000) - floor(format.start_time * 1000)
segment_end_utc_ms = numeric_filename_ms + media_span_ms
next_numeric_filename_ms >= segment_end_utc_ms
```

Use the FFprobe JSON as inputs and save the calculation as `reconnect-overlap.txt`. Any overlap, camera 1 interruption, session fault, or fatal recorder event fails the gate.

## Soak Session

1. Start a fresh session with non-sensitive content and record for one full site-defined class duration.
2. Require both previews to stay live, both recorder statuses to stay `Recording`, and the site-defined SSD free-space margin to remain available. Record `df -h "$SSD_MOUNT"` at start, periodically, and before Stop.
3. Stop normally and rerun the Nominal Session artifact block with `SESSION_LABEL=soak`, `EXPECTED_EVENT_LINES=2`, and the fresh `SESSION_DIR`.
4. Close Leo normally only after it returns to idle. Do not eject the SSD.
5. Require no Leo-owned FFmpeg, FFprobe, or MediaMTX process to survive shutdown.

At the start, periodically during the run, and immediately before Stop, append a timestamped capacity sample and confirm it remains above the approved margin:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
: "${PILOT_EVIDENCE:?set PILOT_EVIDENCE}"
: "${SSD_MOUNT:?set SSD_MOUNT}"
date -u '+utc=%Y-%m-%dT%H:%M:%SZ' | tee -a "$PILOT_EVIDENCE/soak-space.txt"
df -h "$SSD_MOUNT" | tee -a "$PILOT_EVIDENCE/soak-space.txt"
```

Collect the final storage, logs, and first shutdown evidence:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
export SSD_MOUNT="/Volumes/<exact-volume-name>"
export LEO_DATA_DIR="$SSD_MOUNT/leo"

df -h "$SSD_MOUNT" | tee "$PILOT_EVIDENCE/ssd-space-after.txt"
cp "$LEO_DATA_DIR"/logs/leo.jsonl.* "$PILOT_EVIDENCE/"
{ pgrep -lx 'ffmpeg|ffprobe|mediamtx' || true; } \
  | tee "$PILOT_EVIDENCE/post-soak-shutdown-processes.txt"
```

For the clean-restart gate, first confirm those preview ports are free. Relaunch with the same paths and provider-free environment, verify Analyze discovers the completed nominal, reconnect, and soak sessions without clicking Analyze, then close Leo normally:

```bash
cd /Users/noah/Projects/leo/app
set -eu
set -o pipefail
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
export LEO_CAMERA_CONFIG="/absolute/path/to/pilot-cameras.json"
export LEO_DATA_DIR="/Volumes/<exact-volume-name>/leo"

if lsof -nP -iTCP:8889 -sTCP:LISTEN; then exit 1; fi
if lsof -nP -iUDP:8189; then exit 1; fi

env \
  -u OPENAI_API_KEY \
  -u ANALYSIS_MODEL \
  -u OPENAI_BASE_URL \
  -u LEO_E2E_REAL_OPENAI \
  -u LEO_RUN_PAID_OPENAI_TEST \
  LEO_CAMERA_CONFIG="$LEO_CAMERA_CONFIG" \
  LEO_DATA_DIR="$LEO_DATA_DIR" \
  LEO_RECORDER_TIMEOUT_SECS=10 \
  RUST_LOG=info \
  nix develop --command just app 2>&1 \
  | tee "$PILOT_EVIDENCE/clean-restart-console.log"

{ pgrep -lx 'ffmpeg|ffprobe|mediamtx' || true; } \
  | tee "$PILOT_EVIDENCE/post-restart-shutdown-processes.txt"
cp "$LEO_DATA_DIR"/logs/leo.jsonl.* "$PILOT_EVIDENCE/"
```

Both process files must contain no Leo-owned survivor. The clean restart must leave `analysis.json` absent.

## Evidence And Sign-Off

The evidence directory must contain:

- [ ] Every daily `leo.jsonl.<date>` spanning the pilot.
- [ ] Initial and clean-restart console logs.
- [ ] SSD identity and before, periodic, and after free-space output.
- [ ] Both 12-second camera probe outputs and errors.
- [ ] `events.jsonl` and numbered events for each attempted accepted session.
- [ ] FFprobe JSON for every finalized segment.
- [ ] Zero-byte completion-marker checks and no-partial checks.
- [ ] Post-shutdown process lists.
- [ ] Failed-start recovery, start readiness, stop finalization, and reconnect detection/recovery timings, plus the overlap calculation.
- [ ] Operator name, UTC date, site, and pass/fail result for every gate.

Open one focused GitHub issue for every failure, linking only non-sensitive evidence, before pilot sign-off. Do not combine unrelated failures or sign off with an unresolved gate.

## Provider Gate

Recording sign-off does not authorize paid work. Before `just test-paid`, a real-provider desktop E2E, or Analyze with real credentials, obtain a second explicit approval naming the provider, model, target session, and accepted cost.

Paid tests use fixture media. Judge actual pilot-session analysis quality separately through an explicitly approved Analyze action in the app; a paid fixture result is not physical-session acceptance.

## Known Limits

- SSD discovery, identity enforcement, mounting, eject, lifecycle handling, and capacity monitoring are absent.
- Active-session recovery after app crash, force quit, laptop sleep, or power loss is absent.
- Packaged deployment is absent; the pilot uses the Nix development launch.
- Physical-camera timeout calibration is not established.
- Retention, export, deletion, and playback are absent.
- No arbitrary-session validation CLI exists; segment review is manual.
- Concurrent recording sessions and concurrent analyses are unsupported.
