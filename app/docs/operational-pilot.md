# Operational Pilot

Use Bash on the intended Apple Silicon macOS host. Replace every `<...>` marker before running a block; any nonzero command fails the gate. Keep RTSP URLs only in the protected camera config and prompted shell variables, never in evidence.

## Acceptance Boundary
Recording sign-off requires reviewed failed-start, nominal, reconnect, clean-restart, and full-class-duration soak evidence on the intended SSD. Keep provider variables absent and require `analysis.json` to remain absent until sign-off.

## Site Decisions
Record values the repository cannot infer:

- Exactly two physical Axis cameras: stable nonzero ID, model, firmware, and vendor-confirmed H.264 RTSP URL/profile for each.
- SSD: exact mount path, `VolumeUUID`, and lowercase `FilesystemType` reported by `diskutil info -plist`.
- Minimum free-space margin, full expected class duration in seconds, and maximum acceptable unexpected recording gap in milliseconds.

Store RTSP URLs only in the mode-`600` config outside the evidence directory.

## Preparation
In the main operator shell, establish the workspace, private evidence root, approved paths, and SSD validator. The workspace is the directory containing `Cargo.toml`, `flake.nix`, and `justfile`.

```bash
set -eu
set -o pipefail
umask 077
cd "<path-to-cargo-workspace>"
test -f Cargo.toml && test -f flake.nix && test -f justfile
export LEO_WORKSPACE_ROOT="$PWD"

export PILOT_RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$HOME/LeoPilotEvidence"
chmod 700 "$HOME/LeoPilotEvidence"
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/$PILOT_RUN_ID"
mkdir "$PILOT_EVIDENCE"
test "$(stat -f '%Lp' "$PILOT_EVIDENCE")" = 700

export SSD_MOUNT="/Volumes/<exact-volume-name>"
export EXPECTED_VOLUME_UUID="<exact-VolumeUUID>"
export EXPECTED_FILESYSTEM="<exact-FilesystemType>"
export LEO_DATA_DIR="$SSD_MOUNT/leo"
export LEO_CAMERA_CONFIG="<existing-private-directory>/pilot-cameras.json"
case "$LEO_CAMERA_CONFIG" in "$PILOT_EVIDENCE"/*) exit 1 ;; esac

verify_ssd() {
  local label="$1" plist actual_mount actual_uuid actual_filesystem
  case "$label" in ''|*[!A-Za-z0-9_-]*) return 1 ;; esac
  plist="$(mktemp "$PILOT_EVIDENCE/.diskutil.XXXXXX")"
  diskutil info -plist "$SSD_MOUNT" > "$plist" || { rm -f "$plist"; return 1; }
  actual_mount="$(plutil -extract MountPoint raw -expect string "$plist")" \
    || { rm -f "$plist"; return 1; }
  actual_uuid="$(plutil -extract VolumeUUID raw -expect string "$plist")" \
    || { rm -f "$plist"; return 1; }
  actual_filesystem="$(plutil -extract FilesystemType raw -expect string "$plist")" \
    || { rm -f "$plist"; return 1; }
  rm -f "$plist"
  printf 'MountPoint=%s\nVolumeUUID=%s\nFilesystemType=%s\n' \
    "$actual_mount" "$actual_uuid" "$actual_filesystem" \
    | tee "$PILOT_EVIDENCE/ssd-identity-$label.txt" || return 1
  test -d "$SSD_MOUNT" && test ! -L "$SSD_MOUNT" || return 1
  test "$actual_mount" = "$SSD_MOUNT" || return 1
  test "$actual_uuid" = "$EXPECTED_VOLUME_UUID" || return 1
  test "$actual_filesystem" = "$EXPECTED_FILESYSTEM" || return 1
  df -h "$SSD_MOUNT" | tee "$PILOT_EVIDENCE/ssd-space-$label.txt"
}
```

Record versions, require preview ports to be free, and run only the safe local suites:

```bash
cd "$LEO_WORKSPACE_ROOT"
nix develop --command mediamtx --version 2>&1 | tee "$PILOT_EVIDENCE/mediamtx-version.txt"
nix develop --command ffmpeg -version 2>&1 | tee "$PILOT_EVIDENCE/ffmpeg-version.txt"
nix develop --command ffprobe -version 2>&1 | tee "$PILOT_EVIDENCE/ffprobe-version.txt"
if lsof -nP -iTCP:8889 -sTCP:LISTEN >/dev/null; then exit 1; fi
if lsof -nP -iUDP:8189 >/dev/null; then exit 1; fi
nix develop --command just test-unit 2>&1 | tee "$PILOT_EVIDENCE/test-unit.log"
nix develop --command just test-e2e 2>&1 | tee "$PILOT_EVIDENCE/test-e2e.log"
```

Require MediaMTX `v1.18.2` and both recipes to exit zero.

## Camera Bench
Record both Axis models and firmware, then probe each vendor-confirmed URL for 12 seconds. Restricted JSON is safe to retain; raw stderr is discarded because it can repeat credentials.

```bash
cd "$LEO_WORKSPACE_ROOT"
validate_camera_json() {
  local json="$1" count codec packets
  count="$(plutil -extract streams raw -expect array "$json" 2>/dev/null)" || return 1
  codec="$(plutil -extract streams.0.codec_name raw "$json" 2>/dev/null)" || return 1
  packets="$(plutil -extract streams.0.nb_read_packets raw "$json" 2>/dev/null)" || return 1
  test "$count" -eq 1 || return 1
  test "$codec" = h264 || return 1
  case "$packets" in ''|*[!0-9]*) return 1 ;; esac
  test "$packets" -gt 0 || return 1
}
probe_axis() {
  local url="$2" json="$PILOT_EVIDENCE/camera-$1.ffprobe.json"
  if ! nix develop --command ffprobe -v error -rtsp_transport tcp -timeout 10000000 \
    -read_intervals '%+12' -select_streams v -count_packets \
    -show_entries 'stream=codec_name,nb_read_packets' -of json "$url" \
    > "$json" 2>/dev/null; then
    rm -f "$json"
    return 1
  fi
  validate_camera_json "$json"
}
IFS= read -r -s -p 'Axis camera 1 RTSP URL: ' CAMERA_1_RTSP; printf '\n'
IFS= read -r -s -p 'Axis camera 2 RTSP URL: ' CAMERA_2_RTSP; printf '\n'
trap 'unset CAMERA_1_RTSP CAMERA_2_RTSP' EXIT
probe_axis 1 "$CAMERA_1_RTSP"
probe_axis 2 "$CAMERA_2_RTSP"
unset CAMERA_1_RTSP CAMERA_2_RTSP
trap - EXIT
```

Both probes must pass. Do not apply the fixture-specific 15 fps or 150-packet threshold; the nominal app run accepts concurrent preview and recording readers.

## Storage And Preview
Verify identity immediately before creating any SSD directory. Reject existing symlinks first, write-probe the data root, then create a private config without placing its URL contents in shell history.

```bash
cd "$LEO_WORKSPACE_ROOT"
verify_ssd precreate
for directory in "$LEO_DATA_DIR" "$LEO_DATA_DIR/sessions" "$LEO_DATA_DIR/logs"; do
  if test -e "$directory" || test -L "$directory"; then
    test -d "$directory" && test ! -L "$directory"
  fi
done
mkdir -p "$LEO_DATA_DIR/sessions" "$LEO_DATA_DIR/logs"
for directory in "$LEO_DATA_DIR" "$LEO_DATA_DIR/sessions" "$LEO_DATA_DIR/logs"; do test -d "$directory" && test ! -L "$directory"; done
probe="$LEO_DATA_DIR/.leo-write-probe.$$"
printf x > "$probe" && rm "$probe"

config_parent="${LEO_CAMERA_CONFIG%/*}"
test -d "$config_parent" && test ! -L "$config_parent"
test ! -e "$LEO_CAMERA_CONFIG" && test ! -L "$LEO_CAMERA_CONFIG"
umask 077
install -m 600 /dev/null "$LEO_CAMERA_CONFIG"
"${EDITOR:-vi}" "$LEO_CAMERA_CONFIG"
chmod 600 "$LEO_CAMERA_CONFIG"
plutil -lint "$LEO_CAMERA_CONFIG"
```

Enter exactly two rows with the approved Axis IDs, names, URLs, initial participation, and positive whole-second cadence:

```json
[
  {"id": <axis-1-id>, "name": "<axis-1-name>", "rtspUrl": "rtsp://<vendor-url>", "enabled": true, "sampleEveryMs": 1000},
  {"id": <axis-2-id>, "name": "<axis-2-name>", "rtspUrl": "rtsp://<vendor-url>", "enabled": true, "sampleEveryMs": 1000}
]
```

Define one launch gate. It revalidates SSD identity before every app launch and removes every provider or paid-test variable:

```bash
launch_leo() {
  local label="$1"
  verify_ssd "$label" || return 1
  if lsof -nP -iTCP:8889 -sTCP:LISTEN >/dev/null; then return 1; fi
  if lsof -nP -iUDP:8189 >/dev/null; then return 1; fi
  cd "$LEO_WORKSPACE_ROOT"
  env -u OPENAI_API_KEY -u ANALYSIS_MODEL -u OPENAI_BASE_URL \
    -u LEO_E2E_REAL_OPENAI -u LEO_RUN_PAID_OPENAI_TEST \
    LEO_CAMERA_CONFIG="$LEO_CAMERA_CONFIG" LEO_DATA_DIR="$LEO_DATA_DIR" \
    LEO_RECORDER_TIMEOUT_SECS=10 RUST_LOG=info \
    nix develop --command cargo run --locked -p app --bin app 2>&1 \
    | tee "$PILOT_EVIDENCE/$label-console.log"
}
ps -axo pid=,ppid=,comm= | awk '$3 ~ /(^|\/)(ffmpeg|ffprobe|mediamtx)$/ {print}' \
  | LC_ALL=C sort > "$PILOT_EVIDENCE/processes-before-app.txt"
launch_leo initial
```

While `launch_leo initial` runs, require `Session idle`, reviewed SSD paths, two moving previews, and disabled provider analysis.

## Failed Start
In a second shell, establish the same workspace and private run paths; keep this shell for all session checks:

```bash
set -eu
set -o pipefail
umask 077
cd "<same-cargo-workspace>"
test -f Cargo.toml && test -f flake.nix && test -f justfile
export LEO_WORKSPACE_ROOT="$PWD"
export PILOT_EVIDENCE="$HOME/LeoPilotEvidence/<UTC-run-id>"
test -d "$PILOT_EVIDENCE" && test ! -L "$PILOT_EVIDENCE"
test "$(stat -f '%Lp' "$PILOT_EVIDENCE")" = 700
export LEO_DATA_DIR="/Volumes/<exact-volume-name>/leo"
export CAMERA_1_ID="<axis-1-id>"
export CAMERA_2_ID="<axis-2-id>"
export EXPECTED_CLASS_DURATION_SECS="<full-class-seconds>"
export MAX_RECORDING_GAP_MS="<approved-unexpected-gap-ms>"
media_snapshot() {
  ps -axo pid=,ppid=,comm= | awk '$3 ~ /(^|\/)(ffmpeg|ffprobe|mediamtx)$/ {print}' \
    | LC_ALL=C sort > "$1"
}
# session-resolver-start
snapshot_sessions() {
  test -d "$LEO_DATA_DIR/sessions" && test ! -L "$LEO_DATA_DIR/sessions" || return 1
  find "$LEO_DATA_DIR/sessions" -mindepth 1 -maxdepth 1 -print \
    | LC_ALL=C sort > "$1"
}
resolve_new_session() {
  local label="$1" before="$2" after added removed count session name
  case "$label" in ''|*[!A-Za-z0-9_-]*) return 1 ;; esac
  test -f "$before" && test ! -L "$before" || return 1
  after="$PILOT_EVIDENCE/$label-sessions-after.txt"
  added="$PILOT_EVIDENCE/$label-sessions-added.txt"
  removed="$PILOT_EVIDENCE/$label-sessions-removed.txt"
  snapshot_sessions "$after" || return 1
  LC_ALL=C comm -13 "$before" "$after" > "$added" || return 1
  LC_ALL=C comm -23 "$before" "$after" > "$removed" || return 1
  test ! -s "$removed" || return 1
  count="$(wc -l < "$added")" || return 1
  test "$count" -eq 1 || return 1
  IFS= read -r session < "$added" || return 1
  test "${session%/*}" = "$LEO_DATA_DIR/sessions" || return 1
  test -d "$session" && test ! -L "$session" || return 1
  name="${session##*/}"
  case "$name" in ''|*[!0-9]*) return 1 ;; esac
  printf '%s\n' "$session"
}
# session-resolver-end
```

1. Snapshot direct session entries and media processes, disconnect only Axis camera 2, then press Start.
2. Require recovery to `Session idle`, unchanged completed sessions, no marker or retained staging directory, and camera 1 recorder cleanup.
3. Restore camera 2. A faulted or retained directory fails the gate and must be preserved.

```bash
snapshot_sessions "$PILOT_EVIDENCE/failed-start-sessions-before.txt"
media_snapshot "$PILOT_EVIDENCE/failed-start-processes-before.txt"
```

Perform the drill, restore camera 2, then run:

```bash
snapshot_sessions "$PILOT_EVIDENCE/failed-start-sessions-after.txt"
media_snapshot "$PILOT_EVIDENCE/failed-start-processes-after.txt"
cmp "$PILOT_EVIDENCE/failed-start-sessions-before.txt" \
  "$PILOT_EVIDENCE/failed-start-sessions-after.txt"
comm -13 "$PILOT_EVIDENCE/failed-start-processes-before.txt" \
  "$PILOT_EVIDENCE/failed-start-processes-after.txt" \
  > "$PILOT_EVIDENCE/failed-start-new-processes.txt"
test ! -s "$PILOT_EVIDENCE/failed-start-new-processes.txt"
```

## Nominal Session
1. Run `snapshot_sessions "$PILOT_EVIDENCE/nominal-sessions-before.txt"` immediately before Start, then require both previews moving, `Session active`, and both Axis recorders `Recording`.
2. Change camera 1 cadence to 2 seconds. Exclude camera 2 from analysis for 15 seconds, then include it; both must continue recording.
3. Run at least two minutes, Stop normally, and wait for `Session idle`.
4. Navigate to the Analyze view only to confirm the completed row. Do not press the Analyze or Resume provider action.

Define the executable artifact validator in the inspection shell. Coverage uses `[first.utc_ms, first.utc_ms + last.session_offset_ms)` from `events.jsonl`; each recorder range is `[numeric filename, filename + ceil(format.duration*1000) - floor(format.start_time*1000))` and is clipped to that session interval.

```bash
# acceptance-validator-start
validate_h264_json() {
  local json="$1" count codec packets
  count="$(plutil -extract streams raw -expect array "$json" 2>/dev/null)" || return 1
  codec="$(plutil -extract streams.0.codec_name raw "$json" 2>/dev/null)" || return 1
  packets="$(plutil -extract streams.0.nb_read_packets raw "$json" 2>/dev/null)" || return 1
  test "$count" -eq 1 || return 1
  test "$codec" = h264 || return 1
  case "$packets" in ''|*[!0-9]*) return 1 ;; esac
  test "$packets" -gt 0 || return 1
}
validate_segment_json() {
  local json="$1" format start duration span
  validate_h264_json "$json" || return 1
  format="$(plutil -extract format.format_name raw "$json" 2>/dev/null)" || return 1
  start="$(plutil -extract format.start_time raw "$json" 2>/dev/null)" || return 1
  duration="$(plutil -extract format.duration raw "$json" 2>/dev/null)" || return 1
  case ",$format," in *,matroska,*) ;; *) return 1 ;; esac
  span="$(awk -v s="$start" -v d="$duration" '
    function floor_ms(v,a,f) { split(v,a,"."); f=a[2] "000"; return a[1]*1000 + substr(f,1,3) }
    function ceil_ms(v,a,n) { n=floor_ms(v); split(v,a,"."); if (substr(a[2],4) ~ /[1-9]/) n++; return n }
    BEGIN {
      if (s !~ /^[0-9]+([.][0-9]+)?$/ || d !~ /^[0-9]+([.][0-9]+)?$/ || d+0 <= 0) exit 1
      n=ceil_ms(d)-floor_ms(s); if (n <= 0) exit 1; printf "%.0f\n", n
    }')" || return 1
  printf '%s\n' "$span"
}
# acceptance-validator-end
# coverage-validator-start
validate_coverage() {
  local mode="$1" session_start="$2" session_end="$3" minimum="$4" max_gap="$5" ranges="$6" value
  case "$mode" in strict|reconnect-camera-2) ;; *) return 1 ;; esac
  for value in "$session_start" "$session_end" "$minimum" "$max_gap"; do
    case "$value" in ''|*[!0-9]*) return 1 ;; esac
  done
  test -f "$ranges" && test ! -L "$ranges" || return 1
  awk -v mode="$mode" -v session_start="$session_start" -v session_end="$session_end" \
    -v minimum="$minimum" -v max_gap="$max_gap" '
    BEGIN {
      duration=session_end-session_start
      if (duration <= 0 || duration < minimum || (mode == "strict" && minimum > 0 && max_gap >= minimum)) bad=1
    }
    {
      if (bad) next
      if (NF != 2 || $1 !~ /^[0-9]+$/ || $2 !~ /^[0-9]+$/) { bad=1; next }
      raw_start=$1+0; raw_end=$2+0
      if (raw_end <= raw_start || (have_raw && raw_start < previous_raw_end)) { bad=1; next }
      have_raw=1; previous_raw_end=raw_end
      clipped_start=raw_start < session_start ? session_start : raw_start
      clipped_end=raw_end > session_end ? session_end : raw_end
      if (clipped_end <= clipped_start) next
      if (!have_media) {
        leading=clipped_start-session_start; covered=clipped_end-clipped_start
        previous_end=clipped_end; have_media=1; next
      }
      gap=clipped_start-previous_end
      if (gap < 0) { bad=1; next }
      interior[++gap_count]=gap; covered+=clipped_end-clipped_start; previous_end=clipped_end
    }
    END {
      if (bad || !have_raw || !have_media) exit 1
      trailing=session_end-previous_end
      if (leading > max_gap || trailing > max_gap) exit 1
      for (i=1; i<=gap_count; i++) interior_total+=interior[i]
      intentional=0
      if (mode == "reconnect-camera-2") {
        if (gap_count < 1) exit 1
        intentional_index=1; intentional=interior[1]
        for (i=2; i<=gap_count; i++) if (interior[i] > intentional) { intentional=interior[i]; intentional_index=i }
        if (intentional <= 0) exit 1
        for (i=1; i<=gap_count; i++) if (i != intentional_index && interior[i] > max_gap) exit 1
      } else {
        for (i=1; i<=gap_count; i++) if (interior[i] > max_gap) exit 1
      }
      unexpected=leading+trailing+interior_total-intentional
      if (unexpected > max_gap) exit 1
      if (mode == "strict" && covered < minimum) exit 1
      printf "session_duration_ms=%.0f\ncovered_ms=%.0f\nunexpected_gap_ms=%.0f\nintentional_gap_ms=%.0f\n", duration, covered, unexpected, intentional
    }' "$ranges"
}
# coverage-validator-end
inspect_session() {
  local label="$1" session="$2" events="$3" minimum="$4" policy="$5"
  local first last start_type start_offset session_start end_type end_offset session_end
  local id camera_dir manifest ranges count minimum_segments file name stem json span segment_end mode
  cd "$LEO_WORKSPACE_ROOT"
  case "$policy" in strict|reconnect) ;; *) return 1 ;; esac
  test -n "$minimum" && test -n "$MAX_RECORDING_GAP_MS" || return 1
  case "$minimum:$MAX_RECORDING_GAP_MS" in *[!0-9:]*) return 1 ;; esac
  test -d "$session" && test ! -L "$session" || return 1
  test -f "$session/events.jsonl" && test ! -L "$session/events.jsonl" || return 1
  test -f "$session/recording-complete" && test ! -L "$session/recording-complete" || return 1
  test "$(stat -f '%z' "$session/recording-complete")" -eq 0 || return 1
  test "$(wc -l < "$session/events.jsonl")" -eq "$events" || return 1
  test ! -e "$session/analysis.json" || return 1
  test "$CAMERA_1_ID" != "$CAMERA_2_ID" || return 1
  first="$(mktemp "$PILOT_EVIDENCE/.event-first.XXXXXX")"
  last="$(mktemp "$PILOT_EVIDENCE/.event-last.XXXXXX")"
  awk -v first="$first" -v last="$last" 'NR == 1 { print > first } { final=$0 } END { print final > last }' \
    "$session/events.jsonl" || return 1
  session_start="$(plutil -extract utc_ms raw "$first" 2>/dev/null)" || return 1
  start_offset="$(plutil -extract session_offset_ms raw "$first" 2>/dev/null)" || return 1
  start_type="$(plutil -extract action.type raw "$first" 2>/dev/null)" || return 1
  end_offset="$(plutil -extract session_offset_ms raw "$last" 2>/dev/null)" || return 1
  end_type="$(plutil -extract action.type raw "$last" 2>/dev/null)" || return 1
  rm -f "$first" "$last"
  test -n "$session_start" && test -n "$start_offset" && test -n "$end_offset" || return 1
  case "$session_start:$start_offset:$end_offset" in *[!0-9:]*) return 1 ;; esac
  test "$start_type" = session_started && test "$start_offset" -eq 0 || return 1
  test "$end_type" = session_ended && test "$end_offset" -gt 0 || return 1
  session_end=$((session_start + end_offset))
  test "$session_end" -gt "$session_start" || return 1
  nl -ba "$session/events.jsonl" > "$PILOT_EVIDENCE/$label-events.txt"
  cp "$session/events.jsonl" "$PILOT_EVIDENCE/$label-events.jsonl"
  for id in "$CAMERA_1_ID" "$CAMERA_2_ID"; do
    case "$id" in ''|0|*[!0-9]*) return 1 ;; esac
    camera_dir="$session/recordings/camera-$id"
    test -d "$camera_dir" && test ! -L "$camera_dir" || return 1
    manifest="$PILOT_EVIDENCE/$label-camera-$id-segments.txt"
    ranges="$PILOT_EVIDENCE/$label-camera-$id-ranges.txt"
    : > "$manifest"; count=0
    for file in "$camera_dir"/*.mkv; do
      test -f "$file" || continue
      test ! -L "$file" || return 1
      name="${file##*/}"; stem="${name%.mkv}"
      case "$stem" in ''|*[!0-9]*) return 1 ;; esac
      printf '%s\n' "$stem" >> "$manifest"; count=$((count + 1))
    done
    test "$count" -ge 1 || return 1
    minimum_segments=1
    if test "$policy" = reconnect && test "$id" = "$CAMERA_2_ID"; then minimum_segments=2; fi
    test "$count" -ge "$minimum_segments" || return 1
    LC_ALL=C sort -n -o "$manifest" "$manifest"
    : > "$ranges"
    while IFS= read -r stem; do
      file="$camera_dir/$stem.mkv"
      json="$PILOT_EVIDENCE/$label-camera-$id-$stem.ffprobe.json"
      if ! nix develop --command ffprobe -v error -select_streams v -count_packets \
        -show_entries 'stream=codec_name,nb_read_packets:format=format_name,start_time,duration' \
        -of json "$file" > "$json" 2>/dev/null; then rm -f "$json"; return 1; fi
      span="$(validate_segment_json "$json")" || return 1
      segment_end=$((stem + span))
      printf '%s %s\n' "$stem" "$segment_end" >> "$ranges"
    done < "$manifest"
    mode=strict
    if test "$policy" = reconnect && test "$id" = "$CAMERA_2_ID"; then mode=reconnect-camera-2; fi
    validate_coverage "$mode" "$session_start" "$session_end" "$minimum" \
      "$MAX_RECORDING_GAP_MS" "$ranges" > "$PILOT_EVIDENCE/$label-camera-$id-coverage.txt" \
      || return 1
  done
  for file in "$session"/recordings/camera-*/.attempt-*.partial.mkv; do test ! -e "$file" || return 1; done
  printf 'marker_bytes=0\npartials=0\n' > "$PILOT_EVIDENCE/$label-artifacts.txt"
}
```

After normal completion, resolve the one new direct directory and inspect at least two minutes of per-camera coverage:

```bash
SESSION_DIR="$(resolve_new_session nominal "$PILOT_EVIDENCE/nominal-sessions-before.txt")"
inspect_session nominal "$SESSION_DIR" 5 120000 strict
```

Require numbered events in order: start, cadence change, camera 2 exclusion, camera 2 inclusion, end. Every leading, interior, trailing, and total uncovered interval must stay within `MAX_RECORDING_GAP_MS`; the threshold does not reduce the required 120,000 ms of covered media.

## Reconnect Session
Keep timeout `10`. Snapshot with `snapshot_sessions "$PILOT_EVIDENCE/reconnect-sessions-before.txt"`, Start fresh, disconnect only Axis camera 2 after useful media, require camera 1 to remain `Recording`, camera 2 to show `Reconnecting`, then restore it and require `Recording`.

Record four moments with `date -u '+event=<name> utc=%Y-%m-%dT%H:%M:%SZ epoch_s=%s' | tee -a "$PILOT_EVIDENCE/reconnect-timings.txt"`: disconnect, detection, restore, recovery. Calculate both latencies. Stop normally, navigate to the Analyze view without pressing Analyze or Resume, then run:

```bash
SESSION_DIR="$(resolve_new_session reconnect "$PILOT_EVIDENCE/reconnect-sessions-before.txt")"
inspect_session reconnect "$SESSION_DIR" 2 0 reconnect
```

Camera 1 uses `MAX_RECORDING_GAP_MS` for all uncovered media. Camera 2 requires at least two non-overlapping segments; its largest positive interior gap is the intentional disconnect and is measured in `reconnect-camera-$CAMERA_2_ID-coverage.txt` without an SLA. It must agree with the recorded disconnect/recovery timing or the gate fails. The threshold still applies to camera 2 leading/trailing gaps, every other interior gap, and their total.

## Soak Session
Snapshot with `snapshot_sessions "$PILOT_EVIDENCE/soak-sessions-before.txt"`, then run one fresh session for the full expected class duration with non-sensitive content. Both previews and recorders must remain stable. At start, periodically, and before Stop run:

```bash
date -u '+utc=%Y-%m-%dT%H:%M:%SZ' | tee -a "$PILOT_EVIDENCE/soak-space.txt"
df -h "$LEO_DATA_DIR" | tee -a "$PILOT_EVIDENCE/soak-space.txt"
```

Maintain the approved margin and Stop normally. Resolve and inspect the new directory against the full-class covered-media minimum; soak uses the same strict per-gap and total-gap threshold as nominal, so any accepted gap requires an equally longer elapsed session:

```bash
case "$EXPECTED_CLASS_DURATION_SECS" in ''|0|*[!0-9]*) exit 1 ;; esac
SESSION_DIR="$(resolve_new_session soak "$PILOT_EVIDENCE/soak-sessions-before.txt")"
inspect_session soak "$SESSION_DIR" 2 "$((EXPECTED_CLASS_DURATION_SECS * 1000))" strict
```

Close Leo normally after idle. The one-shot `launch_leo initial` returns successfully; compare process state, clean-restart with a fresh SSD identity check, navigate to Analyze without pressing Analyze or Resume, confirm all completed rows, and close normally:

```bash
ps -axo pid=,ppid=,comm= | awk '$3 ~ /(^|\/)(ffmpeg|ffprobe|mediamtx)$/ {print}' \
  | LC_ALL=C sort > "$PILOT_EVIDENCE/processes-after-app.txt"
comm -13 "$PILOT_EVIDENCE/processes-before-app.txt" "$PILOT_EVIDENCE/processes-after-app.txt" \
  > "$PILOT_EVIDENCE/processes-new-after-app.txt"
test ! -s "$PILOT_EVIDENCE/processes-new-after-app.txt"

cp "$LEO_DATA_DIR"/logs/leo.jsonl.* "$PILOT_EVIDENCE/"
ps -axo pid=,ppid=,comm= | awk '$3 ~ /(^|\/)(ffmpeg|ffprobe|mediamtx)$/ {print}' \
  | LC_ALL=C sort > "$PILOT_EVIDENCE/processes-before-restart.txt"
launch_leo clean-restart
ps -axo pid=,ppid=,comm= | awk '$3 ~ /(^|\/)(ffmpeg|ffprobe|mediamtx)$/ {print}' \
  | LC_ALL=C sort > "$PILOT_EVIDENCE/processes-after-restart.txt"
comm -13 "$PILOT_EVIDENCE/processes-before-restart.txt" "$PILOT_EVIDENCE/processes-after-restart.txt" \
  > "$PILOT_EVIDENCE/processes-new-after-restart.txt"
test ! -s "$PILOT_EVIDENCE/processes-new-after-restart.txt"
cp "$LEO_DATA_DIR"/logs/leo.jsonl.* "$PILOT_EVIDENCE/"
```

## Evidence And Sign-Off
Require daily JSON logs, both console logs, sanitized SSD identity and capacity, Axis model/firmware records, camera probe JSON, session before/after/added snapshots, each `events.jsonl`, every segment's FFprobe JSON and range, per-camera coverage summaries, marker/no-partial PASS files, process differences, reconnect timing/intentional-gap evidence, operator/date, and pass/fail per gate. Open one focused issue for every failure before sign-off; attach no secrets. Failed-start, readiness, and Stop timing are not acceptance criteria.

## Provider Gate
Recording sign-off does not authorize paid work. Before `just test-paid`, real-provider desktop E2E, or pressing Analyze/Resume with real credentials, obtain separate explicit approval naming provider, model, target session, and accepted cost. Paid tests use fixtures; judge actual pilot quality separately in the app.

## Known Limits
- No SSD discovery, mount/eject lifecycle, or automatic capacity monitoring.
- No active-session crash, force-quit, sleep, or power-loss recovery.
- No packaged deployment or physical-camera timeout calibration.
- No retention, export, deletion, playback, or arbitrary-session validation CLI.
- No concurrent recording sessions or analyses.
