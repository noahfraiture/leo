This is a project to make video analysis of student exercise. Check the architecture in ./docs/architecture.md

- Prefer public data fields over trivial getters or setters when access needs no validation or additional logic.
- Declare dependencies shared by multiple workspace crates in the root `Cargo.toml` and reference them with `workspace = true`.
- Use thiserror for error handling.
- When a module is large enough to become a directory, keep its non-trivial error types and response conversions in that module's `error.rs`.
- Do not commit plans
- Keep mod.rs minimal, it should not contain any logic or exports beyond what is necessary for the module's functionality
- Do not commit unless explicitly requested
- Use conventional commits with the crate as the scope
- Use tailwindcss and daisyUI for the frontend
- After implementing a plan, clean up the commit history as needed. Organize incidental fixes, regression tests, and unplanned refactors into focused commits, each with a single responsibility and a clear purpose, so the history is easy to review.
- After implementing a plan, make a report of the changes and the difficulty encountered
- When implementating a plan, stop if you encounter major difficulty that would force you to drift from the plan and rework the plan with the user
- Use the narrowest visibility that satisfies the requirement. Keep items private by default, and prefer pub(super) or pub(crate) over pub when broader exposure is unnecessary
- Document public or main components: primary structs, ambiguous fields, entry-point methods, and modules when helpful. Keep documentation concise—usually one or two sentences explaining a component’s responsibility to someone returning to the code after a week. Also add a brief comment for non-trivial code when its behavior or purpose is not obvious.
