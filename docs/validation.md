# Leo Validation Checks

Use the smallest check supported by the resources currently available. Run commands from the workspace root inside `nix develop`. Each level is independent: record the date, environment, `Pass`, `Fail`, or `Not run`, and a focused issue link for any failure. Keep credentials and RTSP URLs out of notes.

## 1. Local mock

**Requires:** the development laptop only. No cameras, SSD, or provider credentials.

```bash
just test-unit test-e2e
```

**Pass:** both recipes exit successfully. They cover unit behavior, local media, recorder reconnect, the mock provider, and the desktop end-to-end flow.

## 2. Local real provider

**Requires:** the local check passes, `OPENAI_API_KEY` and `ANALYSIS_MODEL` are exported, and this paid run has explicit cost approval. Export `OPENAI_BASE_URL` and a non-secret `ANALYSIS_ENDPOINT_ID` only when using a custom endpoint.

```bash
LEO_RUN_PAID_OPENAI_TEST=1 LEO_E2E_REAL_OPENAI=1 just test-paid
```

**Pass:** both paid checks exit successfully against the selected provider and model using local fixture media. Record the provider and model, but no credentials.

## 3. Full physical flow

**Requires:** the local and provider checks pass; two configured Axis H.264 RTSP cameras and the intended external SSD are available; `LEO_CAMERA_CONFIG`, `LEO_DATA_DIR`, `OPENAI_API_KEY`, and `ANALYSIS_MODEL` are exported; `LEO_DATA_DIR` points to the SSD; and the provider request has explicit cost approval.

```bash
just app
```

1. Confirm both physical previews move.
2. Start a session and confirm both cameras record.
3. During a 2-5 minute recording, disconnect one camera once. Confirm the other keeps recording, then reconnect the camera and confirm it returns to recording.
4. Stop normally and confirm the session completes.
5. Confirm the new session is under `LEO_DATA_DIR` on the SSD and both camera directories contain MKV media.
6. Analyze the session with the real provider and confirm results appear.
7. Quit and relaunch with `just app`. Confirm the completed session and persisted analysis remain available.

**Pass:** every observation succeeds. Open one focused issue with a short reproduction for each failure.

Hardware being unavailable is `Not run`; it does not invalidate either local check. Full-duration soak, exact gap arithmetic, process snapshots, power-loss drills, SSD removal, and exhaustive failure injection are outside this smoke checklist.
