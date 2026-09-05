# Manual validation and Luna tuning

## Outcome and execution context

Validate Leo on this macOS laptop, then recommend monitoring and analysis profiles using real OpenAI results. Allow one or two days if the evidence needs it. Finish with a clear account of recording reliability, the best settings found, their cost and quality compared with the baseline, and anything still untested.

Implementation baseline: `4dae58b5638385354c76b63eead0febf6ac56aec` on `codex/monitoring-analysis-profiles`. Read `AGENTS.md`, `app/AGENTS.md`, `docs/architecture.md`, and `docs/validation.md` before execution. Follow this plan from its committed revision; preserve unrelated working-tree changes.

The user requested this plan and its commit, overriding the repository's general instruction not to commit plans. The user also selected this laptop, the existing OpenAI key, and **`gpt-5.6-luna` for paid model tests**. Reviewing and committing this document prepares the campaign; execution starts in the subsequent session or on an explicit instruction to begin. No paid calls are part of preparing the plan.

## What is already established

The implementation passed 247 non-ignored tests, eight local media/mock/desktop checks, formatting, and strict workspace Clippy checks. These establish local behavior, not physical-camera endurance or Luna accuracy. No paid model tests or full-day physical rehearsal have been run for this implementation.

The starting configuration is a comparison baseline, not an optimized recommendation:

| Setting | Baseline for this campaign |
| --- | --- |
| Model | `gpt-5.6-luna` |
| Monitoring sample interval | 1,000 ms |
| Maximum images per request | 16, counting all cameras |
| Maximum prompt time span | 7,000 ms |
| Overlap | 2 complete frame sets |
| Local image sizing | Original |
| Provider image detail | Provider default |
| Output token limit | Unset |

A frame set contains the available camera images for one sampling timestamp. It must remain whole. Freeze the checklist, instruction text, and rolling-context behavior during parameter comparisons. The current pipeline passes the previous complete response into the next request. Results from earlier models or prompt variants are historical evidence only.

## 1. Prepare the laptop and a bounded campaign

1. Record the source revision, macOS version, dependency versions, camera/storage availability, and available disk space. Keep the laptop powered and awake during endurance work. Confirm its normal use will not interfere with the rehearsal.
2. Create a dedicated artifact directory outside the checkout and outside `target`, so build cleanup cannot erase the campaign. Store its absolute path in the handoff. Use isolated application settings and disposable test sessions; preserve the user's real camera configuration, provider settings, recordings, and analysis files.
3. Reuse the existing key without displaying or copying its plaintext into reports, scripts, command arguments, or Git. Check credential availability without printing its value. Verify that it can access Luna and that tests target OpenAI directly. If the key is unavailable, continue free work and report the missing prerequisite.
4. Inspect the existing account/project spending controls if accessible, without increasing them. Do not assume that a key itself enforces a hard limit. Proposed campaign ceiling: **US$10 total, or the lower confirmed available allowance**. The first paid pilot is at most US$0.25; subsequent experiment waves are at most US$1 each within that total. These are campaign limits for review, not claims about OpenAI's account settings.
5. Recheck Luna's current image support, output limits, and pricing before estimating runs. Record the dated price source and billing regime. Count input, cached input when available, and output tokens. Reserve conservative cost for pending requests and uncertain billed failures; do not schedule a request that would exceed the remaining allowance. Treat provider spend reporting as potentially delayed.
6. Run `nix develop --command just test-unit test-e2e`. Record results for the actual campaign checkout. Keep paid flags unset for this phase. Resolve failures before expensive experiments; preserve a reproduction and focused patch for any defect.

Official references checked when preparing this plan: [image inputs and detail](https://developers.openai.com/api/docs/guides/images-vision) and [API pricing](https://developers.openai.com/api/docs/pricing). The application's "Original" sizing means no local resize; it is separate from the provider's image-detail setting. Request count alone is not a cost measure.

## 2. Validate the operator workflow and recording endurance

Use the actual macOS application. Record each case as Pass, Fail, or Not run, with timestamps and concise evidence. Reuse the existing desktop test driver for repeatable actions and inspect the visible result. Do not describe automated assertions as independent human review.

| Area | Evidence to collect |
| --- | --- |
| Settings | Create, edit, select, save, and reload both kinds of profiles. Reject invalid values clearly. A broken optional analysis configuration must not prevent recording. |
| Monitor | Change individual and bulk monitoring profiles and participation during capture. Confirm badges and the event log agree, including same-time changes and different profiles with the same cadence. |
| Capture independence | Preview failure, analysis configuration problems, and controlled metadata-write failure leave capture running where the recording backend is healthy. UI shows the last saved metadata honestly, and Stop remains usable. |
| Stop and recovery | Finalized media plays. Metadata-only stop failure leaves its folder discoverable and allows a new session. A true capture/storage fault is not represented as a normal completed recording. |
| Analyze | Choose a profile, see its resolved parameters in results, interrupt and resume from a durable checkpoint, and start a separate analysis. Resume still works after the selected saved profile is edited or removed. Test on copies. |
| Reconnect and restart | One camera reconnects while the other continues. Segments remain playable with gaps represented honestly. Relaunch preserves completed sessions and analysis. |

Then perform an **8-12 hour rehearsal** with the intended cameras and storage, if available. Include at least one two-hour recording and roughly 8-12 start/stop cycles, periodic navigation, profile changes, participation changes, and a controlled camera reconnect. Check free space, process memory/CPU, recording continuity, segment durations, finalization, and the ability to record the next session. Verify representative media throughout and decode/probe every completed recording for errors.

Run metadata failures only against disposable sessions using the existing test injection mechanism. Do not remove or damage the real recording volume. Physical power-loss and forced storage-removal drills are outside this campaign. If cameras or storage are unavailable, run the same duration with local fixture streams and report physical validation as Not run.

The desktop tests and rehearsal share ports and application resources, so sequence them. Do not start analysis through the UI while it is recording. Run the tuning campaign after capture, or with a separately isolated offline worker only after demonstrating that it does not disturb recording.

## 3. Build a small, independently scored corpus

Inventory available local recordings and `camera/fixtures`. Prefer 12-20 genuinely distinct recordings or exercise episodes, including several longer sessions. This is a target, not a prerequisite to fabricate data or a claim that enough material already exists. Record actual diversity and limitations.

Include clear actions, rapid actions, static scenes, unrelated activity, occlusion, difficult lighting, multiple cameras, missing-camera intervals, participation boundaries, and monitoring-profile changes. Include negative controls with no relevant exercise action. Generated visual fixtures test mechanics and simple recognition; they do not establish real exercise accuracy.

Before looking at model results, create a reference sheet from direct video inspection: visible actions, time intervals, checklist coverage/order, and what cannot be observed. Do not infer hidden safety-critical actions or mark them complete from plausibility. Empty observations can be correct on negative controls. Audit existing paid-test assertions for this distinction before relying on them.

Split approximately 60% for tuning and 40% for held-out evaluation, grouped by original recording and, where possible, person and setup. Adjacent clips from one session must not appear on both sides. Keep at least one unrelated control in each split. Seal the held-out labels and clip list before the search; do not use held-out results to adjust candidates.

The agent may annotate visible evidence, but label that review accurately. Human/domain review should resolve ambiguous exercise judgments. If unavailable, report those cases as unresolved and limit conclusions accordingly. A small or homogeneous corpus can support a provisional profile, not a general accuracy claim.

## 4. Add only the measurement support that is missing

The application already accepts an explicit analysis profile and separate checkpoint path through `AnalyzeSession`. It does **not** yet provide a parameter-search runner, monitoring override, public dry-run plan, or persisted provider usage measurements.

Add a small evaluation entry point that reuses the production sampling, batch planning, extraction, prompts, provider client, and checkpoint flow. It should support:

- A dry run reporting actual sampled frame sets, images, batches, overlap, and invalid configurations before any paid call.
- An explicit corpus manifest, monitoring/analysis settings, run identifier, model, and isolated output directory.
- Per-request token usage, duration, completion status, provider request ID when available, retry history, and estimated cost. Capture usage even when parsing fails. Never log credentials or raw image request bodies.
- A durable campaign ledger and per-run checkpoints. Each independent repeat starts fresh; resuming an interrupted run is not counted as a new repeat. Missing usage is unknown, not zero.

For cadence comparisons, use disposable evaluation session copies with independent, valid metadata and immutable copies of the source media. Preserve camera timing, gaps, participation, and profile-change boundaries while substituting the intended sampling intervals. Record source checksums and the transformation. Never rewrite source events or media. Share the production validators and planner; do not implement an alternate sampling algorithm. Account for disk space before copying.

Record exact source revision, source hashes, checklist/prompt hash, fully resolved profiles, pricing snapshot, and planned batch ranges for every run. A change to any of them requires a separate run. Persist enough diagnostics to reproduce a failed case without embedding secrets.

Use focused mock tests for the new measurement, budget, and resume behavior, plus the existing regression checks. If this requires a broad redesign or changes the analysis semantics, stop that work and revise the plan in Plannotator before continuing. Routine instrumentation and focused defect fixes are within this plan.

## 5. Run a staged Luna search

First execute one short paid pilot and inspect its actual structured output, image handling, usage, and cost. Then run the repository's three explicitly named paid checks using `ANALYSIS_MODEL=gpt-5.6-luna`, both paid flags enabled only for those processes, and `OPENAI_BASE_URL` unset. `just test-paid` contains the exact filters. Never run blanket ignored tests. Apply the campaign ledger to these checks too.

Use the baseline on all tuning cases. Search on a representative short subset first, then expand only promising candidates. Change one parameter family at a time, followed by a small check of interactions around the best candidates. The following are candidate ranges, not a Cartesian product:

| Order | Parameters to investigate | Candidate values |
| --- | --- | --- |
| 1 | Monitoring interval | 250, 500, 1,000, 2,000, 5,000 ms |
| 2 | Maximum images and time span | 8/16/32/64 images; 2/7/15/30 seconds |
| 3 | Overlap | 0/1/2/4 complete frame sets |
| 4 | Local maximum long edge and detail | 512/768/1,024 pixels or Original; provider default/low/high |
| 5 | Output limit | Unset baseline; measured candidates such as 2,048/4,096/8,192 after inspecting actual output use |

Reject impossible combinations through the production planner, including a whole frame set that cannot fit or overlap that prevents progress. Record why they were skipped. Do not silently change settings to make them run. Use the same media and checklist for each comparison; randomize candidate order to reduce time-of-day/provider effects.

Limit the exploratory search to about 24 configurations across the parameter families. Pick the next candidate from observed errors and cost, not a requirement to exhaust every listed value. For example, test denser sampling when short actions are missed, and image size/detail when small visual evidence is lost. Keep the model and prompt fixed. Prompt changes or other models require a separately reviewed experiment.

Repeat the baseline and up to three finalists at least three times as independent complete analyses. Use more repeats only to resolve an uncertain choice within the budget. Evaluate those frozen finalists on the held-out recordings once the search is complete. If held-out failures motivate further tuning, report that evaluation as used and obtain a new untouched holdout before claiming confirmation.

## 6. Judge quality before promoting cheaper settings

Predeclare the scoring rubric before paid comparisons. Report results by recording and important failure type, not just one aggregate score:

- Visible-action precision and recall, with important exercise steps identified in advance. Count unsupported claims separately, especially safety-critical ones.
- Checklist coverage and final status correctness, sequence ordering, duplicated observations, and context drift in longer sessions.
- Timestamp error against the annotated interval, including median and worst-case examples. Distinguish sampling limitations, missing video, and incorrect model timestamps.
- Valid structured outputs, truncation, recovery behavior, complete-run success rate, and provider failures.
- Requests, sampled images, repeated overlap images, tokens, total cost, cost per recorded minute with camera count stated, and latency per request and complete session.

Proposed promotion rule: no new unsupported safety-critical claims, no loss of an important step found by the baseline, no unexplained checklist or completion regression, and no more than a two-percentage-point decline in aggregate visible-action precision or recall. On small datasets, show the underlying counts and individual failures because percentages are unstable. A candidate must also improve cost or latency meaningfully; target at least 20%, without treating that target as more important than evidence quality.

If the baseline itself is inadequate, say so rather than promoting a merely similar failure. If tradeoffs remain, present up to three profiles, such as dense motion, balanced, and slow/static, with evidence for when each is appropriate. Do not implement automatic profile selection in this phase. Changing production defaults requires review of the recommendation.

## 7. Bound the work, preserve progress, and hand back evidence

Checkpoint after every experiment wave and recording cycle. Maintain a short `status.md` in the artifact directory containing the source revision, completed cases, spend ledger location, current best candidates, unresolved issues, and the exact next action. This must support a resumed Codex session without conversation history.

Retry transient provider failures with backoff and a small explicit attempt limit. Stop on exhausted quota, unavailable model access, the campaign ceiling, unexpected billing, unsafe disk headroom, or a recording-integrity failure. Keep usable artifacts and continue independent free analysis where possible. Never raise the account limit, change the paid model, or retry indefinitely to force completion.

End the search after the required checks and held-out comparisons, or after two waves bring no meaningful improvement. The one-to-two-day allowance is a maximum working window, not a reason to spend continuously. If time, budget, hardware, or corpus limits prevent coverage, report Partial with the remaining work rather than claiming completion.

Deliver:

1. A concise report leading with the recommended settings and a baseline/finalist table of request count, real cost, visible-action quality, and latency. Include trial counts, corpus limitations, important failures, and the physical rehearsal verdict.
2. Reproducible run manifests, checkpoints, measurements, the reference rubric, and selected timestamped evidence in the private artifact directory. Keep raw recordings and credentials out of Git.
3. Focused code changes for necessary instrumentation or defects, with relevant tests and a short explanation of difficulties. Separate them by responsibility for review.
4. The report and any proposed default/profile changes presented through Plannotator. Do not silently replace the user's saved profiles. Prepare the final changes for review; do not push or deploy them.

The current task commits the implementation and this reviewed plan. The campaign's measurements and conclusions will be produced during execution, not filled in ahead of time.
