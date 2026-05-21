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

### Production Debugging

13. The production deployment is managed from `~/nix`, not this repository. The Leo app manifests live under `~/nix/clusters/fusion/apps/leo`, and the Helm chart lives under `~/nix/deploy/helm/leo`.
14. Do not paste secrets from `~/nix/clusters/fusion/apps/leo/helmrelease.yaml` into chat or logs.
15. Use Helios for Kubernetes access:

```sh
ssh helios 'kubectl -n leo get pods -o wide'
ssh helios 'kubectl -n leo rollout status deploy/leo --timeout=180s'
ssh helios 'kubectl -n leo get deploy leo -o jsonpath="{.spec.template.spec.containers[0].image}{\"\n\"}{.spec.template.metadata.annotations}{\"\n\"}"'
```

16. Force Flux to pick up pushed `~/nix` changes when needed:

```sh
ssh helios 'flux reconcile source git flux-system -n flux-system && flux reconcile kustomization flux-system -n flux-system --with-source'
```

17. Logs are structured JSON. For analysis failures, start with the analysis id from `/analysis/<id>` and filter production logs by that id:

```sh
ssh helios 'kubectl -n leo logs deploy/leo --since=1h | grep "\"analysis_id\":\"<analysis-id>\""'
ssh helios 'kubectl -n leo logs -f deploy/leo'
```

Useful log fields include `analysis_id`, `provider`, `component`, `event`, `stage`, `attempt`, `attempts`, `payload_bytes`, `offset`, `bytes`, `video_name`, and `error`.

18. Provider-specific events to look for:

- OpenAI: `frames_extracted`, `frames_chunked`, `chunk_request`, `request_send`, `request_retry`, `summary_response`.
- Gemini: `upload_started`, `upload_chunk_send`, `upload_chunk_retry`, `upload_offset_queried`, `upload_final_response_lost`, `generate_content_response`.
- Job lifecycle: `analysis_job` `started` / `failed`, plus persisted analysis events `queued`, `running`, `videos_loaded`, `complete`.
- Uploads: `session_started`, `chunk_accepted`, `chunk_failed`, `session_completed`; chunk logs include browser retry headers as `client_attempt` and `total_chunks`.

19. The analysis result fragment exposes persisted diagnostics and event history. Query it directly with HTMX headers:

```sh
curl -fsSL -H 'HX-Request: true' http://petite.at/analysis/<analysis-id>
```

20. Metrics are exposed at `/metrics` and scraped through pod annotations. Useful counters include:

```sh
curl -fsSL http://petite.at/metrics
```

- `leo_analysis_submissions_total{provider="..."}`
- `leo_analysis_jobs_total{provider="...",result="completed|failed"}`
- `leo_upload_sessions_total{result="started|completed|failed"}`
- `leo_upload_chunks_total{result="accepted|failed"}`
- `leo_canary_runs_total{result="queued|prune_failed|setup_failed|queue_failed"}`

21. Synthetic canaries are controlled by `ANALYSIS_CANARY_*` env vars in the HelmRelease. Production currently queues OpenAI and Gemini canaries on startup and then daily. Canary analyses and the canary video upload are hidden from normal user-facing history, and each canary cycle prunes the previous synthetic analysis for each provider before queuing the next one. The canary video is named `leo-analysis-canary.mp4` and is replaced on each run. Use canary log lines to get the latest generated analysis ids, then inspect those `/analysis/<id>` fragments while that canary is still the latest one.

22. To deploy website code, push this repository first and wait for the `Publish website image` workflow. It publishes tags shaped like `prod-<run>-<short-sha>`. Then update the Leo HelmRelease image tag in `~/nix`, commit, push, reconcile Flux, and verify the deployment image on Helios.

### Backend UI

23. Keep a single mounted page `Route` / `RouteView` pair per file.
24. Keep backend UI feature files page-oriented. Extract a shared file only when markup or logic is meaningfully reused.
25. Do not extract a render helper that is called only once from the same file unless it materially improves readability.
26. Meaningful composable fragments should be embedded `Route` / `RouteView` types in their own file, not plain helper functions.
27. Full pages render through `document()`. `fragment()` is the HTMX and embedding render surface.
28. Prefer DaisyUI classes and semantic state tokens over raw Tailwind color utilities when DaisyUI has an equivalent.
29. Prefer native axum extractors such as `Path`, `Query`, `Form`, `Multipart`, and tuples of extractors for route input. Match the existing `Route::Input = (Path<T>, NoInput)` style for path-only UI routes instead of hand-parsing request URIs.
30. Only introduce custom input/extractor types when they add validation or behavior that axum extractors cannot express cleanly, such as parsing repeated HTML form fields into `Vec<T>`.

### Tests

31. Keep tests focused on non-trivial behavior and shared contracts.
32. Do not add tests for straightforward UI composition or refactor-only changes unless the user asked for them or the change introduces logic.
