# Video Analysis Agent Guidelines

## Purpose

Video Analysis is a Rust web app for receiving uploaded videos and analyzing them with AI providers. It uses server-rendered pages, HTMX fragments, TailwindCSS/DaisyUI, and embedded SurrealDB.

## General Rules

1. Keep `AGENTS.md` up to date when repository conventions or agent instructions change.
2. Prefer useful in-code comments near the relevant pattern over markdown documentation.
3. Keep `README.md` limited to the shortest runnable quickstart.
4. Use conventional commits.

## Stack

### Backend

- Rust
- axum for HTTP
- hypertext for server-rendered HTML
- embedded SurrealDB with SurrealKV for local persistence
- SurrealDB file buckets for uploaded video storage

### Frontend

- HTMX for server interactions
- TailwindCSS and DaisyUI for styling
- Tailwind CLI for frontend CSS builds

## Repository Conventions

### Operational

5. Tooling and workflow instructions for agents belong in `AGENTS.md`; implementation guidance belongs in code comments.
6. Run project workflows through the flake dev shell, usually `nix develop -c task ...`.
7. Use Taskfile entrypoints instead of ad hoc command sequences when a task exists.
8. Keep the tracked `.local` file as the local configuration source; do not introduce `*.example` config files.
9. When reorganizing functions within a file, prefer public functions first, then private helpers in top-down call order.
10. SurrealDB runs embedded in the backend process. Do not add a required local SurrealDB server task unless the architecture changes.

### Backend UI

11. Keep a single mounted page `Route` / `RouteView` pair per file.
12. Keep backend UI feature files page-oriented. Extract a shared file only when markup or logic is meaningfully reused.
13. Do not extract a render helper that is called only once from the same file unless it materially improves readability.
14. Meaningful composable fragments should be embedded `Route` / `RouteView` types in their own file, not plain helper functions.
15. Full pages render through `document()`. `fragment()` is the HTMX and embedding render surface.
16. Prefer DaisyUI classes and semantic state tokens over raw Tailwind color utilities when DaisyUI has an equivalent.

### Tests

17. Keep tests focused on non-trivial behavior and shared contracts.
18. Do not add tests for straightforward UI composition or refactor-only changes unless the user asked for them or the change introduces logic.
