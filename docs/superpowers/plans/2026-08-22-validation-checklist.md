# Validation Checklist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the oversized operational pilot runbook with three short validation sequences using existing `just` recipes.

**Architecture:** Keep validation instructions in one Markdown file, ordered by resource requirements: local mock, local real provider, then physical hardware. Do not add scripts, dependencies, or wrapper recipes; update the README and GitHub issue/milestone links to the new document.

**Tech Stack:** Markdown, Just, GitHub CLI.

## Global Constraints

- Do not add scripts, dependencies, or new `just` recipes.
- Never run `just test-paid`, set `LEO_RUN_PAID_OPENAI_TEST=1`, or make a provider request during implementation.
- Preserve unrelated `materiel.md` changes.
- Do not commit without explicit user authorization.
- Keep credentials and RTSP URLs out of documentation and GitHub metadata.

---

### Task 1: Replace The Pilot With A Validation Checklist

**Files:**
- Create: `docs/validation.md`
- Delete: `docs/operational-pilot.md`
- Modify: `README.md:16`
- Reference: `docs/superpowers/specs/2026-08-22-validation-checklist-design.md`
- Update remotely: GitHub issue #27 and milestone #6

**Interfaces:**
- Consumes: Existing `just test-unit`, `just test-e2e`, `just test-paid`, and `just app` recipes.
- Produces: One validation document with stable section `#3-full-physical-flow` for issue #27.

- [ ] **Step 1: Replace the runbook with the approved concise document**

Create `docs/validation.md` with exactly this content and remove `docs/operational-pilot.md`:

````markdown
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
````

- [ ] **Step 2: Update the README link**

Replace `README.md:16` with:

```markdown
Use the [validation checklist](docs/validation.md) to check Leo locally, with a real provider, or with physical cameras and external storage.
```

- [ ] **Step 3: Verify the local documentation and existing recipes without executing tests**

Run:

```bash
test -f docs/validation.md
test ! -e docs/operational-pilot.md
! rg 'operational-pilot|Operational Pilot' README.md docs/validation.md
nix develop --command just --dry-run test-unit test-e2e
nix develop --command just --dry-run test-paid
git diff --check
```

Expected: both path checks pass, no old wording is found, Just prints the existing commands without running them, and `git diff --check` reports no errors.

- [ ] **Step 4: Update issue #27**

Keep its title, state, milestone assignment, labels, and assignees. Replace only its body with:

```markdown
## Goal

Validate Leo's complete physical-camera workflow on the intended Mac and external SSD by following [Full physical flow](https://github.com/noahfraiture/leo/blob/main/app/docs/validation.md#3-full-physical-flow).

## Prerequisites

- The local mock and local real-provider checks in `app/docs/validation.md` pass.
- Two configured Axis H.264 RTSP cameras and the intended external SSD are available.
- The provider request has explicit cost approval.

## Check

- [ ] Both physical previews work.
- [ ] Both cameras record directly to the SSD.
- [ ] One physical camera disconnects and reconnects without stopping the other recorder.
- [ ] Stop finalizes a completed session with media from both cameras.
- [ ] Real-provider analysis completes for that session.
- [ ] Relaunch restores the completed session and persisted analysis.

Record `Pass`, `Fail`, or `Not run`. Open one focused issue with a short reproduction for each failure. Hardware being unavailable is `Not run`, not a failure.
```

Write the exact body to mode-`600` ignored scratch file `/Users/noah/Projects/leo/.superpowers/sdd/issue-27-validation-body.md`, then run:

```bash
gh issue edit 27 --repo noahfraiture/leo --body-file /Users/noah/Projects/leo/.superpowers/sdd/issue-27-validation-body.md
```

Do not change any other issue field.

- [ ] **Step 5: Update milestone #6**

Keep its title `Operational pilot`, state, and due date. Replace only its description with:

```text
Run the full physical-flow check in app/docs/validation.md with two real Axis cameras, the intended external SSD, and the approved real provider. Local mock and provider checks can be completed before hardware is available.
```

Create mode-`600` ignored payload `/Users/noah/Projects/leo/.superpowers/sdd/milestone-6-validation.json` containing:

```json
{"description":"Run the full physical-flow check in app/docs/validation.md with two real Axis cameras, the intended external SSD, and the approved real provider. Local mock and provider checks can be completed before hardware is available."}
```

Then run:

```bash
gh api --method PATCH repos/noahfraiture/leo/milestones/6 \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2022-11-28' \
  --input /Users/noah/Projects/leo/.superpowers/sdd/milestone-6-validation.json
```

- [ ] **Step 6: Verify GitHub metadata and the worktree**

Run:

```bash
gh api repos/noahfraiture/leo/issues/27 \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2022-11-28' \
  | jq -e '.state=="open" and .title=="Validate the operational pilot on real Axis cameras and external SSD" and .milestone.number==6 and (.body|contains("app/docs/validation.md#3-full-physical-flow"))'

gh api repos/noahfraiture/leo/milestones/6 \
  -H 'Accept: application/vnd.github+json' \
  -H 'X-GitHub-Api-Version: 2022-11-28' \
  | jq -e '.state=="open" and .title=="Operational pilot" and .description=="Run the full physical-flow check in app/docs/validation.md with two real Axis cameras, the intended external SSD, and the approved real provider. Local mock and provider checks can be completed before hardware is available."'

git status --short
```

Expected: both `jq` checks print `true`; status contains the pre-existing `materiel.md` change plus the intended documentation changes. Do not commit.
