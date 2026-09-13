# Analysis workflow

Leo prepares an exercise once, then evaluates completed recordings against the operator's saved checklist and reference images. Models supply observations and a proposed assessment; the operator can inspect footage and record a separate correction.

## Operator and automatic steps

| Operator | Automatic work | Saved result |
| --- | --- | --- |
| Create an exercise; paste and edit checklist steps | Assign stable criterion IDs; validate structure and reference links | Editable draft |
| Request Format and review | Ask the chosen checklist assistant for a source-linked proposal and questions | Temporary proposal, never an analysis instruction |
| Upload JPEG/PNG; enter object, state and purpose; link steps | Decode and validate the file, preserve original bytes, prepare a preview | Manually labeled reference assets; no model request |
| Apply, edit or dismiss the proposal; Save configuration | Atomically save the exact draft and publish an immutable revision | One operator-owned checklist |
| Choose recording and model set; Start new analysis | Freeze inputs, verify recording sources and capacity, execute independent model jobs | Separate run directory |
| Select a result and a citation | Resolve camera and recording offset; prepare and serve a bounded clip | Playable candidate footage with seek and context controls |
| Save my assessment | Append a reasoned correction and optional candidate citations | Human history separate from machine results |
| Revise configuration and rerun | Publish another revision and another run | Previous inputs, outputs and corrections remain intact |

Draft edits and formatting comments never become grading requirements without the operator saving them into the checklist. The assistant cannot silently publish a revision. An outstanding proposal is marked stale if its source draft changes. Formatter comments use the tested source-linked question format; they are not a second checklist. Save explains missing required reference metadata before publication.

## Runtime flow

```text
published exercise + recorded session + selected model set
  |
  v
freeze and validate inputs
  |
  v
independent readers, same criterion and candidate windows
  |
  v
visual reviewer checks every window, even when readers agree
  |
  +--> unresolved evidence: one bounded additional sampling round
  |      followed by independent reading and visual review
  v
text-only selection accounts for every visual review
  |
  +--> required facts unresolved or decisive pack too large
  |      -> Cannot verify, with the actual limitation
  v
final reviewer sees decisive, contextual and contrary images
  |
  v
Met / Not met / Cannot verify + evidence + preserved raw findings
```

Every criterion receives the recording's eligible initial coverage. A reader cannot inspect another reader's output. The visual reviewer sees individual findings and actual footage. The selector organizes existing reviews; it does not claim to see images or produce the final candidate judgment.

Selection must account for every review exactly once as supporting, contradictory, irrelevant or unresolved, with reasons and source IDs. New runs give the selector short, separate review and candidate-frame aliases. The exact alias dictionaries and original typed reply are saved; translation back to source IDs still validates complete accounting and review-window membership. Existing frozen v1 runs retain their original request contract. Explicit negative observations from either readers or reviewers are automatically included in the required final comparison. An oversized comparison is not truncated to obtain a result. The final reviewer receives actual selected images and must retain uncertainty when order, identity or attempts cannot be established together. There is no majority vote.

These are evidence and provenance checks, not a guarantee that a model correctly understands the footage. The operator must still inspect important judgments. Pauses or checklist reading alone do not establish failure; exercise policy comes from the operator's checklist. Missing samples do not prove an omitted step. Hidden state stays unverified.

## Component map

| Component | Owns | Does not own |
| --- | --- | --- |
| `backend::exercise` | Draft/revision persistence, manual imports, source-linked formatting contract | Candidate judgments or automatic publication |
| `backend::model` | Explicit transports, structured completions, model identity and memory scheduling | Exercise policy or provider fallback |
| `analysis::evidence` | Source hashes, candidate IDs, recorded participation/cadence, extraction and playback | Reference labels or model conclusions |
| `analysis::run::engine` | Stage execution, admission waves, accepted response publication and progress | Rendering |
| `analysis::run::workflow` | Initial visual reviews and one additional evidence round | Infinite self-review loops |
| `analysis::run::selection` | Review accounting, complete final comparison and assessment projection | Treating textual agreement as visual proof |
| `analysis::run::store` | Frozen inputs, responses, checkpoint recovery and correction history | Mutable provider secrets |
| `analysis::reconcile` | Typed findings and validation of citations/selection/corrections | Professional truth about an exercise |
| `app::exercises` | Root-owned editable configuration and proposal state | Publishing without a user action |
| `app::analysis` | Root-owned run, playback and correction tasks addressed by session/run ID | Recording supervision |
| `app::operator` | Recording lifecycle and session selection | The replaced single-model analysis checkpoint |
| `app::inference` | One application-wide allowance shared by formatting and analysis | Unloading external model servers |
| `app::media` | Opaque registered playback assets with byte-range responses | Arbitrary model-provided paths |
| `app::views::analyze` | Exercise setup and recording evaluation screens | Business persistence rules |

## Storage and restart

```text
data/exercises/<exercise-id>/
  draft.json
  revisions/<revision-id>.json
  assets/<asset-id>.jpg or .png

data/sessions/<session-directory>/analyses/<run-id>/
  inputs.json         frozen checklist, refs, models, policy, source manifest
  checkpoint.json     stage, jobs, coverage, extra evidence and assessments
  references/         copied original reference bytes for this run
  responses/          original typed replies and sent-image identities
  review.json         human corrections only
  cache/              reproducible frame and playback derivatives
```

Settings schema 4 stores named provider connections, model profiles, sets, execution limits and evidence limits. It does not migrate earlier schemas. Settings changes take effect after restart. Capture remains available when an optional model configuration is missing or malformed; a selected valid set is checked independently of unrelated incomplete sets.

Each run snapshots endpoint and transport choices without credentials. Resume requires matching connection identity and obtains credentials from current private settings. A changed source recording is rejected. Moving the whole recording directory preserves relative source identities.

Responses are durably saved before checkpoints. On restart, a saved response repairs an interrupted checkpoint projection. Successful jobs are reused; pending or failed jobs can be retried by Resume. The review screen exposes the frozen model identities, output/image limits, sampling settings and resource limits; changing current Settings never changes an existing run. A network request interrupted before its response was saved may be sent again. Processing failure is shown separately from candidate failure.

Human corrections include criterion, judgment, reason, citations, time and any superseded correction ID. Corrections using outside information are marked. They do not alter original model output or future prompts.

## Evidence and resource limits

- A candidate ID resolves to camera, session time, source segment and segment-relative offset. Saved request metadata includes hashes of the bytes actually sent.
- References are reserved in every applicable image budget and labeled ineligible as candidate evidence. No model call is needed to import or caption them.
- Initial windows preserve complete available camera frame sets and recorded sampling semantics. Gaps remain explicit.
- One additional round per criterion can inspect at most eight eligible intervals and 128 additional images. Denser samples are distributed across requested spans; a partial search is not proof of omission.
- A local model profile has a declared memory allowance in MiB. The total allowance is capped at 49152 MiB for the current laptop. Hosted models use zero. Concurrency and requests per model are separate limits.
- Aliases of the same endpoint/model share scheduling capacity. Sequential requests do not unload resident weights; the external model server must manage eviction. Settings estimates do not certify actual memory fit.
- Candidate playback creates at most one minute of MP4 context within one source segment, with a 64 MiB derivative ceiling. It preserves the original MKV and does not bridge recording gaps.

## Verification boundaries

The format contract was selected by a recorded comparison of faithful rewriting and matched interpretation. The concise Luna format preserved the source requirements in that bounded test. This does not hardcode Luna: the operator chooses the checklist assistant. Smaller tested Gemma configurations changed meaning or omitted useful questions and were not selected for that experiment.

Provider, store, evidence and coordinator tests cover deterministic contracts and failure recovery. Desktop scenarios use real WKWebView, recording, extraction and playback with a loopback model service. Native reference upload and model quality require the final operator walkthrough; synthetic file events or mock answers cannot establish those results. Keep its observed results in the implementation report, separately from automated assertions.

Tracking, demonstration/calibration features and native-video inference are outside this implementation. Operators test a known-correct recording through the ordinary Analyze workflow and manually refine their checklist and references.
