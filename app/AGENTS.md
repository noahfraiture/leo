This is a project to make video analysis of student exercise. Check the architecture in ./docs/architecture.md

Prefer public data fields over trivial getters or setters when access needs no validation or additional logic.
Declare dependencies shared by multiple workspace crates in the root `Cargo.toml` and reference them with `workspace = true`.
