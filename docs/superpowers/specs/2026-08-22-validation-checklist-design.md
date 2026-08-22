# Leo Validation Checklist Design

## Purpose

Replace the current operational pilot test harness with a short validation checklist. The checklist answers: given the resources available today, what small sequence demonstrates that the corresponding part of Leo works?

The document is for a developer manually checking the system. It is not an operator procedure, release framework, evidence collector, or exhaustive reliability test.

## Structure

Rename `docs/operational-pilot.md` to `docs/validation.md` and title it **Leo Validation Checks**.

The document contains three independent levels in increasing order of external requirements:

1. Local mock validation.
2. Local real-provider validation.
3. Full physical-hardware validation.

Commands run from the workspace root inside `nix develop`, which supplies Just and the media tools.

Each level contains only:

- Prerequisites.
- One main command.
- A pass condition and a pass, fail, or not-run result.

The physical level also has a short manual observation checklist.

## Level 1: Local Mock

### Prerequisites

- The development environment works on the laptop.
- No physical cameras, external SSD, or provider credentials are required.

### Command

```bash
just test-unit test-e2e
```

### Pass Condition

Both recipes exit successfully. Together they cover unit behavior, local media, recorder reconnect, the mock provider, and the desktop end-to-end flow.

## Level 2: Local Real Provider

### Prerequisites

- Level 1 passes.
- `OPENAI_API_KEY` and `ANALYSIS_MODEL` are already exported without placing their values in the document.
- The specific paid run has explicit cost approval.

### Command

```bash
LEO_RUN_PAID_OPENAI_TEST=1 LEO_E2E_REAL_OPENAI=1 just test-paid
```

### Pass Condition

The existing paid application and desktop end-to-end checks both exit successfully against the selected real provider and model using local fixture media.

## Level 3: Full Physical Flow

### Prerequisites

- Level 1 passes.
- Level 2 passes for the selected provider and model.
- Two configured Axis H.264 RTSP cameras and the intended external SSD are available.
- `LEO_CAMERA_CONFIG`, `LEO_DATA_DIR`, `OPENAI_API_KEY`, and `ANALYSIS_MODEL` are already exported. `LEO_DATA_DIR` points to the SSD.
- The real-provider analysis in this run has explicit cost approval.

### Command

```bash
just app
```

### Observation Checklist

1. Both physical previews move.
2. Start a session and confirm both cameras record.
3. During a 2-5 minute recording, disconnect one camera once. Confirm the other keeps recording, then reconnect the camera and confirm it returns to recording.
4. Stop normally and confirm the session completes.
5. Confirm the new session directory is under `LEO_DATA_DIR` on the SSD and both camera directories contain MKV media.
6. Analyze the completed session with the real provider and confirm results appear.
7. Quit and relaunch with the same command. Confirm the completed session and persisted analysis remain available.

### Pass Condition

Every observation succeeds. A failure affects this level only and becomes a focused GitHub issue with a short reproduction.

## Results

Record one short row per run with:

- Date.
- Validation level.
- Relevant environment, such as macOS version, provider/model, or camera/SSD identifiers.
- `Pass`, `Fail`, or `Not run`.
- A GitHub issue link when failed.

Do not retain logs or copy artifacts by default. Keep only the output needed to diagnose a failure. Never record API keys, authorization values, RTSP URLs, or camera credentials.

Unavailable hardware is `Not run`; it does not invalidate the local levels.

## Non-Goals

- Full-class-duration soak testing.
- Exact media-gap arithmetic or FFprobe evidence manifests.
- Process snapshots or automated evidence directories.
- Failed-start, power-loss, force-quit, sleep, SSD-removal, or exhaustive fault drills.
- New pilot scripts or wrapper recipes that duplicate existing `just` recipes.
- Packaging or routine operator instructions.

## Repository Changes

- Replace `docs/operational-pilot.md` with `docs/validation.md`.
- Update repository links that reference the old path.
- Update issue #27 to link directly to the physical-flow section.
- Do not add scripts, dependencies, or new `just` recipes.
