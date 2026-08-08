# Architecture Document Reorganization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Separate system ownership from application behavior in `docs/architecture.md` without losing existing requirements.

**Architecture:** Keep section 2 focused on topology and responsibility boundaries. Move application requirements, API usage, UI behavior, metadata, and processing into a distinct application-design section.

**Tech Stack:** Markdown

## Global Constraints

- Modify only documentation.
- Preserve existing technical decisions unless resolving a direct contradiction.
- Do not add dependencies or implementation detail not present in the source document.

---

### Task 1: Reorganize the architecture document

**Files:**
- Modify: `docs/architecture.md:47-340`

**Interfaces:**
- Consumes: Existing architecture decisions and requirements in `docs/architecture.md`
- Produces: A sequentially numbered document with distinct system-architecture and application-design sections

- [x] **Step 1: Clarify system ownership**

Rename section 2 to `System architecture`, retain the topology diagram, and make the ownership boundary explicit: the custom application requests recording changes through Synology, while Synology owns reliable recording execution and storage rotation.

- [x] **Step 2: Reorganize application behavior**

Rename `Software capabilities` to `Application design`. Group its content under functional requirements, external integrations, operator interface, session metadata, offline processing, and open questions. Remove API responsibility lists already stated in section 2.

- [x] **Step 3: Repair related document structure**

Renumber top-level sections sequentially, move unresolved inline questions into `Open questions`, and correct grammar only in edited passages.

- [x] **Step 4: Verify the resulting document**

Run:

```bash
git diff --check -- docs/architecture.md
```

Expected: no whitespace errors; the diff contains only the planned documentation reorganization.
