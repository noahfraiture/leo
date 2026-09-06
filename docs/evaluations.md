# Reproducible local evaluations

The `backend` evaluation example reuses the production sampling, batch planner, frame extraction, prompt, provider parser, and durable checkpoints. It is available only with `test-support`. Store manifests and results outside the checkout and `target`, using isolated session metadata and immutable source media.

Build with `nix develop --command cargo build -p backend --example evaluate --features test-support`. A manifest contains:

```json
{
  "directory": "/absolute/evaluation/session",
  "checklist": "Describe only visible evidence for the supplied steps.",
  "profile": {
    "id": 1,
    "name": "Comparison baseline",
    "model": "gpt-5.6-luna",
    "maxImagesPerPrompt": 16,
    "maxPromptSpanMs": 7000,
    "overlapFrameSets": 2,
    "imageSize": "original",
    "imageDetail": "providerDefault",
    "maxOutputTokens": null
  },
  "checkpoint_path": "/absolute/evaluation/run/analysis.json",
  "base_url": null
}
```

The session must have valid completed metadata and finalized media. The output must be a separate absolute path with an existing parent directory, outside the source session even after resolving directory aliases. Symbolic-link output files are rejected. The example rejects changes to an existing checkpoint's production fingerprint rather than restarting it silently.

Run `nix develop --command ./target/debug/examples/evaluate /absolute/manifest.json --dry-run` first. It reports unique sampled images and the image count and timestamp bounds of every actual batch. Dry runs can create the initial empty checkpoint but send no provider requests. An image limit counts all cameras; overlap counts whole frame sets. Increasing the sampling interval can increase requests when the time span and overlap leave little new evidence per batch.

## Paid Luna measurements

Paid execution requires explicit user approval, `OPENAI_API_KEY`, `LEO_RUN_PAID_OPENAI_TEST=1`, `LEO_EVALUATION_JOURNAL` containing an absolute path, and `LEO_EVALUATION_RUN` naming the run. Pass credentials through the process environment. Leave `OPENAI_BASE_URL` unset. Remove `--dry-run` to execute, or use `--one-batch` to finish one incomplete batch and exit at a durable checkpoint. A successful `--one-batch` exit does not mean the whole session is complete.

Use one journal for the entire approved campaign, including pilots, retries, and desktop tests. The metering adapter accepts only `gpt-5.6-luna` for paid requests and pins the default service tier. `LEO_EVALUATION_LIMIT_USD` can lower the journal's ceiling from USD 100; it cannot raise it. This does not inspect or modify account spending controls.

Every paid request reserves USD 1 before dispatch, covering the documented maximum standard request cost at the prices verified on 2026-09-05. On completion, the journal records reported usage, elapsed time, response ID, service tier, completion state, and parse success. Known usage settles the reservation using the dated prices in `agent/metering.rs`. Missing usage or interruption retains the reservation. These are estimates from provider token counts, not invoice totals. Recheck [OpenAI pricing](https://developers.openai.com/api/docs/pricing) before reusing the adapter with changed prices or model limits.

The latest record for each request ID is authoritative. Independent writers wait briefly for the journal lock, then fail closed if it remains unavailable. Do not reset the journal to resume a failed run. A resumed analysis uses its saved prefix; an independent repeat needs a fresh output path. Preserve run manifests, media hashes, code revision and patch, attempt outcomes, reference annotations, and the pricing snapshot alongside the journal.

## Local LM Studio measurements

Use a separate manifest with the actual served model identifier and an explicit loopback `base_url`, such as `http://127.0.0.1:1234/v1`. The example never substitutes OpenAI for a local endpoint. The same journal can record local usage and latency with zero OpenAI charge.

For a server that enforces schemas through Chat Completions, add `"chat_completions": true` and, if supported, `"reasoning_effort": "none"`. This selects the evaluation-only local transport while retaining the shared production prompt, response schema, parser, and checkpoint engine. It does not add a Chat Completions provider setting to the desktop application, which still uses Responses. Verify the real response schema and visible evidence before long local runs. A parseable response can still contain wrong actions, unsupported checklist statuses, or unsuitable timestamp formatting.

## Desktop validation additions

`just test-e2e` also exercises a generated, very low-bitrate static RTSP stream. The recorder must start within its configured timeout and produce decodable media even when FFmpeg's normal output buffer would take much longer to fill. Packet flushing keeps the on-disk readiness check aligned with incoming frame progress.

The ignored `rehearsal::desktop_profiles_save_and_reload` test creates profiles through the native settings form, saves, restarts the app, and checks the restored default and image-detail selection. It needs a new absolute `LEO_E2E_OUTPUT_DIR` and runs through the `camera` crate's `desktop_e2e` target with the `desktop-e2e` feature.

The separately ignored `rehearsal::desktop_capture_rehearsal` requires `LEO_DESKTOP_E2E_REHEARSAL=1` and a new output directory. It records for 15 minutes, changes monitoring and participation, completes two further sessions, and fully decodes retained media. It sends no model requests. By default it uses two fixture RTSP streams. An optional owner-only JSON file supplied through `LEO_E2E_PHYSICAL_SOURCE_FILE` can provide `rtsp_url` for the first camera. `LEO_E2E_EXPECT_RECONNECT=1` additionally requires that camera to produce a new segment after a separately controlled interruption. The test checks bounded missing duration and writes actual coverage measurements; it does not change the physical camera or interrupt the network itself.

`rehearsal::desktop_validate_retained_rehearsal` rechecks an existing rehearsal with a successful desktop driver result. It clips coverage to the recorded session boundaries and decodes every frame using the source time base, avoiding false timestamp collisions from the null output's default frame-rate rounding. It preserves the recordings and original capture logs.

Run these tests individually by exact name with `--ignored --exact --nocapture --test-threads=1`. Keep capture and local model inference separate, and distinguish automated desktop assertions, physical capture evidence, visual review, and full-day reliability in the report.
