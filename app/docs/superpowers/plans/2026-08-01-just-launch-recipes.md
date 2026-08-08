# Just Launch Recipes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `camera`, `vlc`, and `app` recipes for launching the local preview stack.

**Architecture:** Keep the fixed development addresses and fixture directly in three independent `just` recipes. Run every tool through the existing Nix development shell.

**Tech Stack:** just, Nix, Cargo, Dioxus CLI, VLC

## Global Constraints

- Modify only `justfile`.
- Do not add configurable variables or a combined launcher.
- Do not commit unless explicitly requested.

---

### Task 1: Add launch recipes

**Files:**
- Modify: `justfile`
- Test: `just --dry-run camera vlc app`

**Interfaces:**
- Consumes: workspace packages `camera` and `app`, `camera/fixtures/default.mp4`, Nix-provided `vlc`
- Produces: `just camera`, `just vlc`, and `just app`

- [ ] **Step 1: Verify the recipes are absent**

Run: `just --dry-run camera`

Expected: FAIL with `Justfile does not contain recipe camera`.

- [ ] **Step 2: Add the minimal recipes**

```just
camera:
    nix develop --command cargo run -p camera -- --address 127.0.0.1:8080 --rtsp-address 127.0.0.1:8554 --video camera/fixtures/default.mp4

vlc:
    nix develop --command vlc rtsp://127.0.0.1:8554/axis-media/media.amp

app:
    nix develop --command dx serve -p app --desktop
```

- [ ] **Step 3: Verify command rendering**

Run: `just --dry-run camera vlc app`

Expected: PASS and print the three commands without launching long-running processes.

- [ ] **Step 4: Verify recipe discovery**

Run: `just --list`

Expected: PASS and list `camera`, `vlc`, and `app` with the existing recipes.

- [ ] **Step 5: Inspect the final diff**

Run: `git diff -- justfile`

Expected: only the three approved recipes are added.
