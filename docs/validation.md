# Leo Validation Checks

Use the smallest check supported by the resources currently available. Run commands from the workspace root inside `nix develop`. Each level is independent: record the date, environment, `Pass`, `Fail`, or `Not run`, and a focused issue link for any failure. Keep credentials and RTSP URLs out of notes.

## 1. Local mock

**Requires:** the development laptop only. No cameras, SSD, or provider credentials.

```bash
just test-unit test-e2e
```

This is a free, local-only check. **Pass:** both recipes exit successfully. They cover unit behavior, local media, recorder reconnect, the mock provider, and the desktop end-to-end flow.

## 2. Local real provider

**Requires:** the local check passes and this paid run has explicit cost approval. `LEO_RUN_PAID_OPENAI_TEST`, `LEO_E2E_REAL_OPENAI`, `OPENAI_API_KEY`, and `ANALYSIS_MODEL` are inputs to the gated test processes only; they are not Leo application configuration. `OPENAI_BASE_URL` must be unset because desktop paid validation targets OpenAI directly.

```bash
LEO_RUN_PAID_OPENAI_TEST=1 LEO_E2E_REAL_OPENAI=1 just test-paid
```

**Pass:** all three paid checks exit successfully against OpenAI using the selected model and local fixture media. Record the model, but no credentials.

## 3. Full physical flow

**Requires:** the local and provider checks pass; the actual H.264 RTSP camera or cameras and intended external storage are available; and the provider request has explicit cost approval.

1. Launch `just app`. In Settings, configure the actual camera or cameras, select the external data root, enter the provider key, model, and optional base URL, choose the log level, and confirm the recorder timeout.
2. Save, restart Leo, and confirm Settings shows the intended data path.
3. Confirm every configured physical preview moves.
4. Start a session and confirm every configured camera records.
5. For the two-camera disconnect/reconnect acceptance setup, disconnect one camera during a 2-5 minute recording. Confirm the other keeps recording, then reconnect the camera and confirm it returns to recording. This setup tests independent reconnect behavior; it is not a product camera-count restriction.
6. Stop normally and confirm the session completes under the data path shown in Settings, with MKV media in each expected camera directory.
7. Analyze the session with the real provider and confirm results appear.
8. Quit and relaunch with `just app`. Confirm the completed session and persisted analysis remain available.

**Pass:** every observation succeeds. Open one focused issue with a short reproduction for each failure.

Hardware being unavailable is `Not run`; it does not invalidate either local check. Keep credentials and complete RTSP URLs out of reports and logs. Full-duration soak, exact gap arithmetic, process snapshots, power-loss drills, SSD removal, and exhaustive failure injection are outside this smoke checklist.
