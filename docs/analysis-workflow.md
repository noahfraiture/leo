# Analysis workflow

Leo prepares an exercise once and evaluates completed recordings against one operator-confirmed checklist. Models supply cited observations and proposed results. The operator owns policy, corrections and the final student assessment.

## Operator flow

| Operator step | Automatic work | Saved result |
| --- | --- | --- |
| 1. Name the exercise, enter context, choose ordering policy, paste original steps | Assign stable criterion IDs | Editable draft |
| 2. Upload JPEG/PNG references, label their object/state/purpose and link steps | Validate images; preserve original bytes and prepare previews, without AI | Reference assets |
| 3. Optionally request wording assistance; review suggestions and answer questions | Video-blind, source-linked review from the selected model | Source, editable proposal and questions remain distinct |
| Edit the final checklist freely; apply or dismiss the proposal | Preserve operator-added steps, deletions, order and reference links | One authoritative final checklist |
| 4. Review and confirm | Check source/answer freshness; publish an immutable revision | Confirmed exercise |
| Select a completed recording and one analysis model; start | Freeze inputs and execute the simple flow below | A new independent run |
| Review results, original clips and any closer look; correct findings if needed | Validate citations and save append-only correction history | Original model findings plus operator corrections |
| Mark each result reviewed; save the overall student assessment | Bind the assessment to those exact results | Pass / Needs practice / Needs more evidence, chosen by the operator |

Drafts can be saved at any step. Edited or dismissed proposals survive saving. A source edit does not silently change the final wording or reuse an answer to a different question: the operator must reconfirm or remove stale questions. Added final steps can explicitly be included in preparation without replacing earlier source text. Unanswered questions remain pending policy, never invented requirements.

## Automatic analysis

```text
confirmed checklist + context + operator answers + labeled references
                         + completed recording + one model
                                      |
                         freeze inputs and verify sources
                                      |
                overlapping images: scan the WHOLE checklist
                      once per chronological window
                                      |
                one text consolidation of all cited observations
                                      |
                Met / Not met / Cannot verify + policy issues
                                      |
                optional independent closer look for a criterion
                original images + full checklist + operator policy
                WITHOUT the earlier verdict or model explanation
                                      |
                preserve both findings and their original evidence
                                      |
                       operator review and assessment
```

Reference labels and context are always supplied. Actual reference pixels and automatic closer looks are separate options, both off by default while their quality benefit is assessed. A manual closer look is available even with automatic rechecks off. Both use the same one-request allowance per criterion; an explicit retry of a failed request is not another evidence search.

A closer look covers the original citations and an optional operator-selected timestamp, with two seconds of context on either side, within one episode of at most one minute. The model's image limit includes relevant reference images. The planner refuses to drop contrary or distant citations to force a result. If additional eligible images cannot fit, it explains the limitation for manual review. One criterion's sampling does not consume another criterion's access to the same episode.

All returned criteria and citations are validated. Consolidation must account for every checklist item and can cite only that item's observed frames. Reference images cannot count as student evidence. Contradictory decisive findings remain Cannot verify; both are retained. Images cannot resolve an unspecified operator policy. Missing observations are not proof that a step was omitted.

This pipeline has no multiple-reader aggregation, per-window second review, evidence selector, index retrieval agent, object tracker, demonstration calibration or native-video inference.

## Responsibilities

| Component | Responsibility |
| --- | --- |
| `backend::exercise` | Original/final checklist, source-linked preparation, manual reference imports and immutable publications |
| `backend::model` | Provider transport, structured output contracts and model capacity |
| `analysis::evidence` | Recorded sampling periods, source hashes, candidate identities, extraction and playback |
| `analysis::run::engine` | Sequential scan, consolidation, optional recheck, explicit resume and progress |
| `analysis::run::jobs` | Stateless prompts, operator policy and independent recheck requests |
| `analysis::run::recheck` | One bounded episode preserving required citations |
| `analysis::run::results` | Response validation and conservative combination of preserved findings |
| `analysis::run::store` / `legacy` | Atomic persistence, recovery and read-only older history |
| `analysis::run::review` | Append-only corrections, review marks and human student assessments |
| `app::exercises` / `app::analysis` | Root-owned state and tasks scoped by exercise, recording and run |
| `app::inference` / `app::media` | Shared inference admission and registered, bounded playback assets |

## Storage and recovery

```text
exercises/<exercise-id>/
  draft.json
  revisions/<revision-id>.json
  assets/<asset-id>.jpg or .png
sessions/<recording>/analyses/<run-id>/
  inputs.json             immutable schema-2 configuration and source identities
  checkpoint.json         current jobs, evidence and result projection
  responses/              accepted typed replies and exact sent-image hashes
  references/             copied original reference bytes
  review.json             append-only per-criterion human corrections
  operator-review.json    review marks and overall human assessments
  cache/                  reproducible images and playback clips
```

No provider secrets are stored in run inputs. Resume uses current credentials only when the saved connection identity matches. Changed source recordings or changed cached images are rejected. An interrupted checkpoint is reconstructed from durable responses; completed requests are not repeated. Provider failures pause without silent retry. A request interrupted before its response was saved may be sent again only through explicit Resume.

Run-format 1 history remains discoverable as a read-only summary. Its original inputs, evidence, replies and correction files stay in place. It cannot resume through the schema-2 engine. A new analysis creates a separate run instead of migrating or overwriting it.

The overall assessment is valid only for a ready run whose current findings and corrections match the reviewed version. New evidence or a correction invalidates affected review marks; a pending or failed recheck makes the previous overall assessment historical. Review state never transfers to another recording or rerun.

Settings schema 4 remains compatible. Preparation and evaluation select model profiles directly; old model-set settings no longer select readers for this flow. Settings changes still require restart. Recording remains available with missing optional analysis configuration.

## Validation and limits

Deterministic tests verify contracts, citations, source preservation, request counts, independent evidence, recovery and review invalidation. Desktop acceptance uses actual WKWebView, two fixture cameras, recording, FFmpeg extraction, playback and operator controls, with a scripted loopback model service. These establish software behavior, not accuracy or readiness for autonomous student grading.

Reference file decoding and provenance are covered by backend tests; the desktop acceptance does not automate the native file chooser. Long-recording accuracy, model runtime and the optional additions need separate controlled evaluations. Large consolidations can exceed a provider's context capacity: the run pauses rather than silently discarding observations. Source footage still may not reveal hidden states or small actions. Candidate judgments require operator review.
