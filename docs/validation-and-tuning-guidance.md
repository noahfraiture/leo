# Approved execution brief

Start here when executing the validation campaign. Read [the approved plan](validation-and-tuning-plan.md), then apply the guidance below. This records the user's six notes attached to the Plannotator approval on 2026-09-05. These directions take precedence wherever they differ from the plan. The approved plan is retained unchanged.

## Scope and budget

Run on this macOS laptop. Evaluate both **OpenAI `gpt-5.6-luna`** and the **Qwen model downloaded in LM Studio**. Produce a final report with interpreted results and recommended parameters for each model, including where either model is unsuitable. Fix bugs encountered during the work, verify the fixes, and keep the changes focused and reviewable.

The authorized OpenAI ceiling is **US$100 total**, including pilots, retries, existing paid checks, and tuning runs. Respect any lower remaining account/project allowance without raising its settings. Spend less where possible, but prioritize thorough, useful testing over minimizing cost. The original US$10 campaign ceiling is superseded. Its pilot and wave sizes are starting suggestions, not additional approval gates. Continue tracking observed usage and conservatively reserving cost before dispatching requests.

There is **no fixed duration requirement or one-to-two-day deadline**. The user expects efficient work and does not want an artificial 8-12 hour wait. Begin with focused checks and representative 15-30 minute recordings, including repeated start/stop and recovery. Extend runtime when a specific failure, drift, or reliability question requires it. Report actual tested duration and do not claim full-day reliability from a shorter run. Similarly, 24 exploratory configurations is an initial search size, not a ceiling when further experiments are justified.

Account for the cost of the Codex work itself. Prefer automated collection and concise checkpoint summaries to continuous polling, duplicate analysis, or repeated broad test runs. Preserve progress before usage/context limits can interrupt the session. Run appropriate regression checks after relevant code changes; do not rerun the entire suite after every parameter-only experiment.

## Available camera and video sources

- A physical camera is running on the local network. Its view is not useful for exercise assessment, but it can validate preview, real capture, finalization, reconnect, and playback. The user supplied credentials privately with the approval. They are deliberately omitted from these committed documents. Reuse the supplied credentials through existing local configuration or private review context; never print them or commit them. Resolve the camera address from the current application configuration or the known local device setup.
- Helios is accessible on the network. Look for the user's `film` and `Movies` directories. Local SSH configuration resolves `Helios` to hostname `helios`, user `noah`, port 22; remote reachability and exact media paths remain to be verified. Treat these collections as read-only sources and copy only selected material needed for experiments.
- The user identified a short, single-camera gun disassembly/reassembly recording in Downloads as a useful real exercise example. Confirm the correct file through direct inspection before labeling it. Do not infer mechanical safety or hidden actions from appearances.
- Movies from Helios are permitted as supplementary material, but the user explicitly cautioned that they are not representative: real exercise recordings may have poor visibility and little motion. Separate movie/control results from real exercise results in the report.
- Find better public examples online if useful. Prefer suitably licensed, stationary-camera instructional or activity videos with limited motion, occlusion, and imperfect visibility. Record provenance and usage terms. Inspect/download them without uploading private recordings to search or third-party services.

A read-only inventory found these likely exercise-video candidates, but their contents have not yet been verified:

```text
/Users/noah/Downloads/wetransfer_video-test-neo-check-list_2026-05-04_1044/Test video check liste avec erreur.MOV
/Users/noah/Downloads/wetransfer_video-test-neo-check-list_2026-05-04_1044/Test vido check list ok.MOV
```

Inspect these first, then other local Downloads videos if needed. Aim for realistic 15-30 minute cases where available, alongside the short real example. Keep the plan's independent reference annotations, source-level split, negative controls, and visible-evidence scoring. If representative data is scarce, report that limit rather than treating movies, duplicated footage, or synthetic degradation as independent real-world validation.

## Add a separate Qwen evaluation track

The local model inventory contains:

```text
/Users/noah/.lmstudio/models/lmstudio-community/Qwen3.8-27B-GGUF/Qwen3.8-27B-Q4_K_M.gguf
/Users/noah/.lmstudio/models/lmstudio-community/Qwen3.8-27B-GGUF/mmproj-Qwen3.8-27B-BF16.gguf
/Users/noah/.lmstudio/models/lmstudio-community/Qwen3.8-27B-MLX-4bit/config.json
```

These paths establish downloaded model files, not a currently loaded or functioning server. At preflight, inspect LM Studio's actual loaded model, served identifier, endpoint, runtime/quantization, context limit, and supported image/structured-output behavior. Prefer its configured working variant. Do not download or compare additional large models without a concrete need.

Leo currently uses the OpenAI Responses client. First test the local endpoint with a tiny local image and the real response schema. Verify image understanding, request compatibility, output-token behavior, structured parsing, and resume behavior before spending time on long local runs. If necessary, make the smallest provider compatibility fix that preserves the application's production analysis path and existing OpenAI behavior. A broad provider redesign still requires a revised plan in Plannotator.

Use explicit local provider configuration and a distinct output namespace. Never route a supposed local experiment to OpenAI as a silent fallback. The repository's `just test-paid` recipe intentionally requires OpenAI directly; it is for Luna, not the LM Studio runs. Add/use a separate local evaluation entry point through the shared production pipeline.

Tune Qwen independently because its useful image size, batching, context capacity, and overlap may differ. Start with a fitting baseline and explain any compatibility-driven differences from Luna. Use the same source material, reference labels, and held-out split for the final comparison. Freeze each model's configuration before evaluating its held-out cases.

Measure latency, complete-session throughput, memory pressure, and stability as well as evidence quality. Record local inference as having no OpenAI charge, while recognizing laptop load, electricity, and agent time. Avoid resource contention with the capture rehearsal; run local inference separately unless a deliberate contention test is needed. Do not let it monopolize the user's laptop without a useful experiment in progress.

## Final handoff

Lead the final report with an interpretation: which settings to use for Luna and Qwen, how trustworthy the results are, and when to choose each. Include selected actual outputs and timestamped examples, a compact baseline/finalist comparison, failure cases, real-camera validation evidence, total API spend, elapsed work, and unresolved limitations. Distinguish model failures from sampling, visibility, or application defects.

Preserve durable experiment artifacts and focused bug fixes, and present the report and proposed changes in Plannotator. This planning task commits the approved plan plus this execution brief; it does not itself start the campaign.
