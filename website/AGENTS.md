# Video Analysis Agent Guidelines

## Purpose

Video Analysis is a Rust web app for receiving uploaded videos and analyzing them with AI providers. It uses server-rendered pages, HTMX fragments, TailwindCSS/DaisyUI compiled from backend UI CSS, and embedded SurrealDB.

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

The videos files will live in surrealdb with the new file support. They will also be served at `/video/<video>` so that the html can have a video player with the file path directly.

### UI

- HTMX for server interactions
- TailwindCSS and DaisyUI for styling
- CSS source lives in `backend/src/http/ui/styles.css`
- Compiled CSS lives in `backend/src/http/ui/styles.generated.css` and is embedded by `document()`

## Repository Conventions

### Operational

5. Tooling and workflow instructions for agents belong in `AGENTS.md`; implementation guidance belongs in code comments.
6. When the user asks a question or asks to discuss a design, answer and discuss first. Do not edit files unless the user explicitly asks for an implementation or confirms the direction.
7. Run project workflows through the flake dev shell, usually `nix develop -c task ...`.
8. Use Taskfile entrypoints instead of ad hoc command sequences when a task exists.
9. Keep the tracked `.local` file as the local configuration source; do not introduce `*.example` config files.
10. When reorganizing functions within a file, prefer public functions first, then private helpers in top-down call order.
11. SurrealDB runs embedded in the backend process. Do not add a required local SurrealDB server task unless the architecture changes.
12. Prefer public struct fields for simple data models over trivial accessor methods.

### Backend UI

13. Keep a single mounted page `Route` / `RouteView` pair per file.
14. Keep backend UI feature files page-oriented. Extract a shared file only when markup or logic is meaningfully reused.
15. Do not extract a render helper that is called only once from the same file unless it materially improves readability.
16. Meaningful composable fragments should be embedded `Route` / `RouteView` types in their own file, not plain helper functions.
17. Full pages render through `document()`. `fragment()` is the HTMX and embedding render surface.
18. Prefer DaisyUI classes and semantic state tokens over raw Tailwind color utilities when DaisyUI has an equivalent.
19. Prefer native axum extractors such as `Path`, `Query`, `Form`, `Multipart`, and tuples of extractors for route input. Match the existing `Route::Input = (Path<T>, NoInput)` style for path-only UI routes instead of hand-parsing request URIs.
20. Only introduce custom input/extractor types when they add validation or behavior that axum extractors cannot express cleanly, such as parsing repeated HTML form fields into `Vec<T>`.

### Tests

21. Keep tests focused on non-trivial behavior and shared contracts.
22. Do not add tests for straightforward UI composition or refactor-only changes unless the user asked for them or the change introduces logic.
