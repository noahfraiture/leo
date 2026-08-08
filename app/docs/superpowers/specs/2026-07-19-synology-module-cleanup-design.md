# Synology Module Cleanup Design

## Goal

Organize the Synology mock like the camera crate: a small server module, an API directory organized by responsibility, typed Axum query extraction, and errors owned by the module that emits them.

## Structure

```text
synology/src/
├── lib.rs
├── server.rs
├── camera.rs
└── api/
    ├── mod.rs
    ├── error.rs
    ├── info.rs
    ├── entry.rs
    ├── camera.rs
    └── external_recording.rs
```

`server.rs` only binds, serves, builds shared camera state, and nests `/webapi`. `api/mod.rs` owns the Axum router, shared state alias, and success response envelope. Each Synology API module owns its constants, validation, response DTOs, and operation.

## Request Flow

`info::handle` extracts a typed `InfoRequest` for `query.cgi`. `entry::handle` extracts one typed `EntryRequest` containing the common envelope and all currently supported optional entry parameters, then dispatches by `api` to the camera or external-recording module.

`EntryRequest` accumulates future entry API parameters rather than storing a flattened string map. API-specific value types such as `CameraId` and `RecordingAction` preserve type safety while retaining invalid variants needed for Synology's documented error codes.

Handlers accept `Result<Query<T>, QueryRejection>` and return `Result<Response, ApiError>`. `ApiError` converts query rejection to error 101 and implements `IntoResponse`; handlers use `?` instead of manually building failure responses.

## Errors

`api/error.rs` owns `ApiError`, numeric API codes, failure response DTOs, `From<QueryRejection>`, and `IntoResponse`. The crate-level `error.rs`, `Error`, and `Result` aliases are deleted; `server::start` returns `std::io::Result<()>` directly.

Add this repository rule to `AGENTS.md`: when a module becomes a directory, non-trivial errors and response conversions belong in that module's `error.rs`; do not create a crate-level error module solely to wrap one standard-library error.

## State and I/O

The API state remains `Arc<Mutex<Vec<Camera>>>`. Camera data is cloned while locked, network reachability is checked after releasing the lock, and recording state is updated only after reacquiring it. No lock is held across an await.

`Camera::reachable` owns the 250 ms TCP probe shared by camera listing and external recording.

## Compatibility

Preserve the existing CGI routes, JSON shapes, numeric errors, validation order, reachability behavior, recording mutation, and public `server::app`/`server::start` entry points. Add no dependency.

## Tests

Keep black-box route coverage but colocate API-specific tests with their modules. State-specific recording tests construct API state directly instead of requiring a production `app_with_state` function. Keep bind-failure coverage in `server.rs`.
