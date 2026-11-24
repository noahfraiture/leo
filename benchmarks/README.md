# Compare vision models on Leo's recordings

Run the commands below from the Leo checkout with Python 3.10 or later. The runner uses the Python standard library. It sends ordered images and questions to a vision model served through Chat Completions.

```sh
python3 -m benchmarks list
python3 -m benchmarks validate
python3 -m benchmarks run --suite all --model my-first-model --model my-second-model
python3 -m benchmarks compare
```

Tencent WeMM has a [local provider and build guide](providers/README.md).

The default endpoint is LM Studio at `http://127.0.0.1:1234/v1`. Use the exact served model ID. With different servers or model files, point each invocation at the same result directory.

```sh
python3 -m benchmarks run --suite all --model model-a \
  --base-url http://127.0.0.1:1234/v1 --revision weights-a --out ~/leo-comparison
python3 -m benchmarks run --suite all --model model-b \
  --base-url http://127.0.0.1:8080/v1 --revision weights-b --out ~/leo-comparison
python3 -m benchmarks compare ~/leo-comparison
```

The three default suites cover 70 judgments. Select `--suite all` for all seven, or repeat `--suite NAME` to choose your own set. Every model receives the same original images in the same order, captions, task wording, and output contract. The runner sets temperature to zero. The completion budget defaults to 2,048 tokens, matching the three-model comparison. For models that spend tokens on reasoning, use the same larger `--max-tokens` for each model. Record checkpoint, quantization, runtime build, and reasoning mode with the results. The server controls its image preprocessing and inference implementation.

| Suite | Model requests | Judgments | What the score covers |
| --- | ---: | ---: | --- |
| `leo-household` | 5 | 18 | The five household excerpts from the historical 33-check campaign. Always answering `met` scores 16/18. |
| `public-evidence` | 6 | 12 | Three public recordings and their paired evidence-removal controls. The gold describes what the supplied frames establish. |
| `perception-confirmation` | 40 | 40 | Two temporal or perceptual questions on each of 20 public recordings from separately grouped scenes. |
| `perception-development` | 24 | 24 | Two questions on each of 12 development recordings. |
| `perception-donor` | 24 | 24 | Two questions on each of 12 recordings used during method development. |
| `perception-linked` | 8 | 8 | Four additional confirmation recordings. Two link to development scenes and two to donor scenes. |
| `leo-drone` | 2 | 8 | Two longer equipment-handling recordings with 42 and 56 initial samples. Check server image and context limits. |

The four Perception Test suites use the archived 16-frame packets, original choice order and packet-specific adjudicated labels. Every score is a count of exact answer labels. A single-question request expects one label. A multi-question request expects a complete JSON object keyed by criterion. Incomplete generations and malformed answers are unassessed and stay in the declared denominator. The historical evidence-support audits and full desktop workflow scores measure different tasks. Several suites reuse scenes, and the archived labels include AI review.

## Results and recovery

Run `compare` at any point. Add `--baseline MODEL` to count gains and losses on questions both models answered. Each table groups models that share the same suite, input fingerprint and output budget. It shows correct, assessed, invalid, errored and pending judgments. Time includes all recorded attempts. The local JSON checkpoints retain the answer keys, individual replies, model ID, usage when returned, completion status and elapsed time.

On the Mac with the recovered archive, the default comparison directory includes the November 2026 Qwen3.8, Inkling 2 Mini and WeMM-Video results. Run a new model at the default budget and use `python3 -m benchmarks compare --baseline leo-qwen38-benchmark` to compare it with Qwen. The study report and question-level predictions are in `~/Library/Application Support/Leo/benchmark-runs/2026-11-21-comparison/`.

A repeated command resumes a partial run. Completed and invalid answers remain recorded. Use `--retry-errors` to retry transport errors after fixing the endpoint. The checkpoint preserves the earlier attempts. Use a different output directory for an independent repeat. For a server that needs authentication, supply `LEO_BENCH_API_KEY` in the environment. A remote endpoint receives the selected images.

## Test a model through the desktop workflow

The local desktop benchmark runs Leo's frame extraction, scan, consolidation and citation checks on 46 recordings. Its three suites contain 12 visibility controls, 18 household recordings and eight matched pairs of correct and incorrect cooking steps. Each incoming model receives the same checklists and frame-sampling settings as the saved Qwen reference. The scorer reports each cohort separately.

With the model already running on a local Chat Completions endpoint, set its advertised model ID and port in this command. Use a fresh results directory for each configuration. Repeating the command resumes the same comparison.

```sh
study="$HOME/Library/Application Support/Leo/benchmark-runs/2026-11-21-fallback-study"
python3 "$study/harness/compare_model.py" --model my-local-vision-model --port 1234 \
  --out "$HOME/leo-desktop-results/my-local-vision-model"
```

The study's `harness/README.md` describes single-suite runs, required inputs and interruption handling. Its `REPORT.md` contains the November model and fallback comparisons.

## Take the benchmarks to another machine

Export creates a new directory with versioned suite fingerprints, separate answer keys, ordered inputs and one copy of each unique image. It verifies the pack before publishing it. Copy that directory and this checkout's `benchmarks/` package to the other machine. The copied pack and Python package contain everything needed to validate the data.

```sh
python3 -m benchmarks export ~/leo-vision-pack
python3 -m benchmarks --pack ~/leo-vision-pack validate
python3 -m benchmarks --pack ~/leo-vision-pack run --model my-model --out ~/leo-comparison
```

An explicitly selected pack defaults to every suite it contains. Select suites explicitly for a smaller run.

## Find the original experiments

```sh
python3 -m benchmarks archive
```

On this Mac, `list`, `validate` and `run` automatically use the installed pack in `~/Library/Application Support/Leo/benchmarks/vision-v1`. On other machines the runner checks the same location relative to the Leo data home. Set `--archive` or `LEO_EVALUATIONS` to read the historical inputs directly.

On macOS the archive defaults to `~/Library/Application Support/Leo/evaluations`. On Linux it defaults to `$XDG_DATA_HOME/leo/evaluations` or `~/.local/share/leo/evaluations`. Set `LEO_EVALUATIONS` or pass `--archive DIRECTORY` before a command to use a relocated archive. Results default to the sibling `benchmark-runs/comparisons` under the platform's Leo data home.

| Within the evaluation archive | What to open |
| --- | --- |
| `strategy-retrospective-2026-09-30/report.html` | Searchable review of 53 strategies and links to original sources. |
| `2026-09-05-profiles/report.md` | Original validation and tuning campaign, with metrics and related experiments in the same directory. |
| `weight-space-2026-11-02/benchmark/catalog.json` | Canonical definitions for 14 historical suite entries, including repeated prompt variants and indexed workloads. Read `FINAL-STATUS.txt` for the later exposure status. |
| `video-transfer-2026-11-04/report/FINAL-REPORT-V1.txt` | Source grouping, temporal questions and interpretation of the public video transfer experiment. |

The experimental runners for evidence investigation, simple flow and bounded comparisons also survive in linked Git worktrees. `git worktree list` shows their locations. Some of those files are untracked there. The reusable runner reads the evaluation archive directly.

Run the offline checks with `python3 -m unittest discover -s benchmarks/tests`.
