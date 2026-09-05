# Leo Validation Checks

Use the smallest check supported by the resources currently available. Run commands from the workspace root inside `nix develop`. Each level is independent: record the date, environment, `Pass`, `Fail`, or `Not run`, and a focused issue link for any failure. Keep credentials and RTSP URLs out of notes.

## 1. Local mock

**Requires:** the development laptop only. No cameras, SSD, or provider credentials.

```bash
just test-unit test-e2e
```

This is a free, local-only check. **Pass:** both recipes exit successfully. They cover unit behavior, local media, recorder reconnect, the mock provider, and the desktop happy, analysis-recovery, and preview-degraded workflows. The latter changes profiles and participation, injects a metadata append failure, verifies both cameras keep recording across it, decodes the retained video, and records a subsequent session. Local evidence checks also verify resizing, image detail, and output-token limits.

## 2. Local real provider

**Requires:** the local check passes and this paid run has explicit cost approval. `LEO_RUN_PAID_OPENAI_TEST`, `LEO_E2E_REAL_OPENAI`, `OPENAI_API_KEY`, and `ANALYSIS_MODEL` are inputs to the gated test processes only; they are not Leo application configuration. `OPENAI_BASE_URL` must be unset because desktop paid validation targets OpenAI directly.

```bash
LEO_RUN_PAID_OPENAI_TEST=1 LEO_E2E_REAL_OPENAI=1 just test-paid
```

**Pass:** all three paid checks exit successfully against OpenAI using the selected model and local fixture media. Record the model, but no credentials.

## 3. Full physical flow

**Requires:** the local and provider checks pass; the actual H.264 RTSP camera or cameras and intended external storage are available; and the provider request has explicit cost approval.

1. Launch `just app`. In Settings, configure the actual camera or cameras, select the external data root, configure monitoring and analysis profiles, enter the provider key and optional base URL, choose the log level, and confirm the recorder timeout.
2. Save, restart Leo, and confirm Settings shows the intended data path.
3. Confirm every configured physical preview moves.
4. Start a session and confirm every configured camera records.
5. For the two-camera disconnect/reconnect acceptance setup, disconnect one camera during a 2-5 minute recording. Confirm the other keeps recording, then reconnect the camera and confirm it returns to recording. This setup tests independent reconnect behavior; it is not a product camera-count restriction.
6. Stop normally and confirm the session completes under the data path shown in Settings, with MKV media in each expected camera directory.
7. Analyze the session with the real provider and confirm results appear.
8. Quit and relaunch with `just app`. Confirm the completed session and persisted analysis remain available.

**Pass:** every observation succeeds. Open one focused issue with a short reproduction for each failure.

### Optional external-storage-loss drill

This is an exploratory recovery check, not part of the required smoke gate. Use only a disposable external volume whose contents can be lost; an operating system may refuse a normal eject while FFmpeg has files open, and a forced removal can corrupt the volume.

1. Configure the disposable volume as the data root, restart Leo, start a session, and wait until every camera reports `Recording`.
2. Disconnect or unmount the volume while recording. Do not do this with the real working volume.
3. Confirm Leo reports a session fault and does not return to a normal completed-session state. Record the visible message and timing without credentials or complete RTSP URLs.
4. Quit Leo, reconnect the same volume, and inspect the original session directory. It must not contain `recording-complete`; retain whatever event and partial-media files the operating system successfully persisted for diagnosis.
5. With the configured data root mounted again, restart Leo and complete a new short session. Leo does not hot-recover or make the interrupted session analyzable.

The important safety property is that uncertain storage is never presented as a completed recording. If the operating system refuses the removal, record the drill as `Not run` rather than forcing a non-disposable device.

Hardware being unavailable is `Not run`; it does not invalidate either local check. Keep credentials and complete RTSP URLs out of reports and logs. Power-loss drills, process-crash continuity, and exhaustive failure injection remain outside this smoke checklist.


## 4. Before customer use: full-day rehearsal

Use the intended cameras and storage for a full working day. Repeatedly start and stop sessions, navigate Settings/Monitor/Analyze, change individual and bulk monitoring profiles and participation, and disconnect/reconnect one camera. On disposable sessions, perform controlled metadata-write failure checks without removing the recording volume. Confirm continued capture where possible, honest last-saved warnings, playable finalized media, discoverable incomplete folders, and the ability to record the next session. Record elapsed duration and any interruptions. Automated local checks do not establish day-long reliability. Provider analysis still needs separate cost approval.
