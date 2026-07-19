This is a project to make video analysis of student exercise. Check the architecture in ./docs/architecture.md

Prefer public data fields over trivial getters or setters when access needs no validation or additional logic.
Declare dependencies shared by multiple workspace crates in the root `Cargo.toml` and reference them with `workspace = true`.

When a module is large enough to become a directory, keep its non-trivial error types and response conversions in that module's `error.rs`. Do not create a crate-level `error.rs` solely to wrap a standard-library error.
