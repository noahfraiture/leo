# Video Analysis

Rust web app for video upload and AI analysis with axum, hypertext SSR, HTMX fragments, TailwindCSS/DaisyUI, and embedded SurrealDB.

## Quickstart

```bash
nix develop -c task setup
nix develop -c task run
```

Open `http://localhost:8080`.

Local CLI analysis:

```bash
nix develop -c task backend:analyze -- --provider gemini --model gemini-3-flash-preview --prompt "Summarize these videos" ./one.mp4 ./two.mp4
```

The tracked `.local` file contains local defaults. SurrealDB runs embedded with data under `.data/`.

Implementation guidance lives in comments near the code it explains.
