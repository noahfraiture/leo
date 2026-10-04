# Operator workflow implementation

Implemented on `main`, 4 October 2026. The approved preparation and review flow now uses a simpler analysis engine. The complete desktop workflow and failure recovery pass with controlled local responses. This is an implementation result, not a new accuracy benchmark.

## What changed

- Four preparation screens: original checklist/context/order, manually labeled references, wording assistance/questions/final edits, and explicit confirmation.
- Original text, editable proposals, questions, operator answers and the final checklist persist separately. Applying assistance preserves operator-added/deleted steps and reference links. Changed source wording cannot silently reuse an old answer.
- Preparation and analysis select model profiles directly. New runs use one model, one chronological scan per overlapping image window for the whole checklist, then one consolidation of all cited observations.
- An optional independent closer look uses original images without the earlier verdict. Automatic and manual requests share one allowance per criterion. The bounded packet preserves initial citations; it refuses to discard distant evidence merely to fit a request.
- Results expose frozen configuration, all checklist items, attention/resolved filters, original footage, initial and recheck findings, configuration issues and separate human corrections.
- The operator marks current results reviewed and saves an overall student assessment. Corrections or new findings invalidate the affected review; a pending recheck makes the prior overall assessment historical. New runs start with fresh human review state.
- Older runs remain read-only summaries with their full original files preserved. The old selector, aliases, repeated per-window reviews and their implementation-specific tests are removed.

Reference pixels and automatic rechecks default off and remain explicit options. Labels/context are always supplied. Existing recording and reference-import foundations are reused. There are no exercise-specific rules, provider fallbacks, silent retries or additional model agents.

## Review the commits

| Commit | Purpose |
| --- | --- |
| `72558f9c` | Preparation persistence and source-bound policy history |
| `864165d6` | Operator preparation screens and transition tests |
| `626ccb6b` | Analysis replacement, results/human assessment, regression tests and desktop acceptance |
| `012d9c14` | Removal of retired strategy modules, separated for easier review |
| `40525dd5` | Restrict generated styles to app sources instead of scanning research documents |
| Final documentation commit | Current architecture, workflow contracts and this report |

The runtime commit is larger than the usual limit because the saved run contract, engine, desktop consumers and acceptance fixtures must switch together. Keeping temporary compatibility executors solely to split it would preserve the complexity being removed. The deletion commit is also large but contains only retired files. Other implementation commits stay below 1,000 changed lines.

For the main runtime review, start with `run/model.rs`, then `engine.rs`, `jobs.rs`, `recheck.rs`, `results.rs`, and `review.rs`. The previous implementation is preserved on `codex/archive-main-before-simple-workflow-2026-10-04`. No push was performed. Unrelated pre-existing Settings changes, plans, media and AGENTS edits remain outside these commits.

## Problems caught and resolved

Independent simplicity/readability reviews caught four important boundaries: a recheck needs the full ordered checklist, two criteria must not compete for previously sampled frames, pending evidence must invalidate the current overall assessment, and a policy problem discovered during recheck must remain visible. Each now has a focused regression assertion.

Preparation review also caught the risk of losing operator additions or retaining an answer after its source changed. Tests exercise those transitions and saved proposal edits. Native desktop acceptance exposed the need for explicit selected options in webview controls. No major change to the agreed workflow was required.

The style build was scanning non-application files and producing irrelevant components. Its inputs now explicitly cover `app/src`; generated styles became smaller. Across the implementation and cleanup commits, the source tree is 1,452 lines smaller, including generated CSS and tests.

## Verification

- `cargo test --workspace --all-targets --all-features --locked`: **351 passed, 16 ignored**, no failures. Paid tests remain ignored.
- `desktop_operator_flow_records_two_cameras_and_analyzes`: passed with real WKWebView, two fixture cameras, recording, checklist preparation, extraction, decoded playback/seek, independent recheck, correction, review marks, saved student assessment and a separate rerun.
- `desktop_analysis_resumes_after_transient_provider_failure`: passed; completed requests are reused and the failed request resumes explicitly.
- Reference tests use actual encoded image bytes to verify opt-in delivery, criterion links, separation from candidate citations and rejection of modified originals.
- Formatting/diff checks pass. Clippy completes with non-fatal style warnings; it is not warning-free.
- Independent backend and app reviews found no remaining blocking issue after the fixes. Acceptance scenarios check cleanup of their owned processes.

No paid requests, downloads of models or live model accuracy experiments were performed. Native desktop controls were driven programmatically by the acceptance harness. The native reference file chooser and physical cameras were not manually exercised in this turn; decoding, storage and reference delivery are tested separately.

## Remaining limits

The model can still misread an image, miss a brief action, or lack enough evidence for ordering or a hidden state. A large text consolidation can exceed provider context limits and will pause instead of silently truncating evidence. These are visible limits, not solved accuracy claims.

The next quality evaluation should freeze a confirmed exercise and compare the simple baseline with reference pixels and the optional recheck separately. Software acceptance passing does not demonstrate a higher /33 score or readiness for autonomous grading. The operator remains responsible for the final student assessment.
