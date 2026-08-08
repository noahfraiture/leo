# Axis PTZ Query Dispatch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the VAPIX-compatible `info=1` and `rpan=<degrees>` operations on the single `GET /axis-cgi/com/ptz.cgi` endpoint.

**Architecture:** Keep one Axum route and deserialize its query string into one typed `PtzParams` struct whose optional fields can grow with future combinable commands. The router owns `Camera`, wrapping it in `Arc<Mutex<Camera>>` only as Axum state; the handler never holds the lock across an `.await` and only dispatches to the existing no-op `Camera::pan` in this increment.

**Tech Stack:** Rust 2024, Axum 0.8, Tokio 1, Serde 1, Tower 0.5

## Global Constraints

- Keep exactly one `GET /axis-cgi/com/ptz.cgi` route; query keys select behavior and are not separate routes.
- Match VAPIX response conventions: successful commands return `204 No Content`; information and errors return `200 OK` with `Content-Type: text/plain`.
- Format errors as `Error:<message>`.
- Support only channel `1`; omitted `camera` also means channel `1`.
- Accept `rpan` only in the documented inclusive range `-360.0..=360.0`.
- `info=1` advertises only commands implemented by this server.
- Reject `info` combined with a movement command; movement fields remain optional so future movement commands can be applied together.
- Dispatch `rpan` to `Camera::pan` without modeling or persisting a pan position.
- Add no custom router, middleware, extractor, module, or dependency other than Serde.
- Do not create a git commit unless the user explicitly requests one.

---

## File Structure

- Modify `camera/src/lib.rs`: repair the existing test setup and add HTTP behavior tests.
- Modify `camera/src/server.rs`: own camera state, deserialize PTZ query parameters, dispatch operations, and format VAPIX responses.
- Modify `camera/Cargo.toml`: add Serde with derive support.
- Modify `Cargo.lock`: let Cargo record the direct Serde dependency when tests run.
- Leave `camera/src/camera.rs` unchanged: use the existing no-op `Camera::pan(&mut self)` because this increment covers dispatch, not position modeling.

### Task 1: Repair Router Ownership and Existing Tests

**Files:**
- Modify: `camera/src/lib.rs:5-49`
- Modify: `camera/src/server.rs:6-20`

**Interfaces:**
- Consumes: `Camera::new(address: SocketAddr) -> Camera`
- Produces: `server::app(camera: Camera) -> Router` and `server::start(camera: Camera) -> std::io::Result<()>`

- [ ] **Step 1: Update existing tests to construct and transfer an owned camera**

Replace the test module setup and the two existing calls to `app` with:

```rust
#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::{camera::Camera, server::app};

    fn camera() -> Camera {
        Camera::new("127.0.0.1:0".parse().unwrap())
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let response = app(camera())
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_path_returns_not_found() {
        let response = app(camera())
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
```

- [ ] **Step 2: Run the tests to verify the ownership API is not implemented yet**

Run: `cargo test -p camera`

Expected: compilation fails because `app` still expects `&mut Camera`.

- [ ] **Step 3: Make `start` and `app` own the camera**

Change only the signatures and call site in `camera/src/server.rs`; state wrapping comes with the PTZ handler in Task 2:

```rust
pub async fn start(mut camera: Camera) -> std::io::Result<()> {
    if camera.status != Status::Ready {
        return Ok(());
    }

    let listener = TcpListener::bind(camera.address).await?;
    camera.status = Status::Running;
    axum::serve(listener, app(camera)).await
}

pub fn app(_camera: Camera) -> Router {
    let axis = Router::new().route("/ptz.cgi", get(|| async {}));
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .nest("/axis-cgi/com", axis)
}
```

- [ ] **Step 4: Run the repaired baseline tests**

Run: `cargo test -p camera`

Expected: both `health_returns_ok` and `unknown_path_returns_not_found` pass.

### Task 2: Add Typed PTZ Query Dispatch

**Files:**
- Modify: `camera/Cargo.toml:6-9`
- Modify: `camera/src/lib.rs:5-49`
- Modify: `camera/src/server.rs:1-21`
- Modify: `Cargo.lock` through Cargo

**Interfaces:**
- Consumes: `server::app(camera: Camera) -> Router` from Task 1 and `Camera::pan(&mut self)` from `camera/src/camera.rs`
- Produces: private `PtzParams { camera: Option<u8>, info: Option<u8>, rpan: Option<f64> }` and the `GET /axis-cgi/com/ptz.cgi` behavior described in Global Constraints

- [ ] **Step 1: Add HTTP test helpers and failing PTZ behavior tests**

Extend the imports and test module in `camera/src/lib.rs` so the complete module is:

```rust
#[cfg(test)]
mod tests {
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header::CONTENT_TYPE},
        response::Response,
    };
    use tower::ServiceExt;

    use super::{camera::Camera, server::app};

    fn camera() -> Camera {
        Camera::new("127.0.0.1:0".parse().unwrap())
    }

    async fn get(app: Router, uri: &str) -> Response {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn body(response: Response) -> String {
        String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let response = get(app(camera()), "/health").await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_path_returns_not_found() {
        let response = get(app(camera()), "/missing").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ptz_info_lists_implemented_commands() {
        let response = get(app(camera()), "/axis-cgi/com/ptz.cgi?info=1").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
        assert_eq!(
            body(response).await,
            "Available commands:{camera=[n]}rpan=[offset]"
        );
    }

    #[tokio::test]
    async fn ptz_relative_pan_returns_no_content() {
        for uri in [
            "/axis-cgi/com/ptz.cgi?rpan=10",
            "/axis-cgi/com/ptz.cgi?rpan=-10.5&camera=1",
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain");
            assert_eq!(body(response).await, "");
        }
    }

    #[tokio::test]
    async fn ptz_errors_use_vapix_response_format() {
        for uri in [
            "/axis-cgi/com/ptz.cgi",
            "/axis-cgi/com/ptz.cgi?camera=2&rpan=10",
            "/axis-cgi/com/ptz.cgi?info=2",
            "/axis-cgi/com/ptz.cgi?info=1&rpan=10",
            "/axis-cgi/com/ptz.cgi?rpan=361",
            "/axis-cgi/com/ptz.cgi?rpan=invalid",
            "/axis-cgi/com/ptz.cgi?zoom=100",
        ] {
            let response = get(app(camera()), uri).await;
            assert_eq!(response.status(), StatusCode::OK, "{uri}");
            assert_eq!(response.headers()[CONTENT_TYPE], "text/plain", "{uri}");
            assert!(body(response).await.starts_with("Error:"), "{uri}");
        }
    }
}
```

- [ ] **Step 2: Run the PTZ tests to verify they fail against the empty handler**

Run: `cargo test -p camera ptz_`

Expected: all three PTZ tests fail because the current handler returns an empty `200 OK` response.

- [ ] **Step 3: Add Serde derive support**

Add the direct dependency to `camera/Cargo.toml`:

```toml
[dependencies]
axum = "0.8.9"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1.53.0", features = ["macros", "rt-multi-thread", "net"] }
```

- [ ] **Step 4: Implement the single typed-query handler and owned shared state**

Replace `camera/src/server.rs` with:

```rust
use std::{fmt::Display, sync::{Arc, Mutex}};

use axum::{
    Router,
    extract::{Query, State, rejection::QueryRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::camera::{Camera, Status};

type SharedCamera = Arc<Mutex<Camera>>;

const AVAILABLE_COMMANDS: &str = "Available commands:{camera=[n]}rpan=[offset]";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PtzParams {
    camera: Option<u8>,
    info: Option<u8>,
    rpan: Option<f64>,
}

pub async fn start(mut camera: Camera) -> std::io::Result<()> {
    if camera.status != Status::Ready {
        return Ok(());
    }

    let listener = TcpListener::bind(camera.address).await?;
    camera.status = Status::Running;
    axum::serve(listener, app(camera)).await
}

pub fn app(camera: Camera) -> Router {
    let axis = Router::new().route("/ptz.cgi", get(ptz));
    Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .nest("/axis-cgi/com", axis)
        .with_state(Arc::new(Mutex::new(camera)))
}

async fn ptz(
    State(camera): State<SharedCamera>,
    query: Result<Query<PtzParams>, QueryRejection>,
) -> Response {
    let Query(params) = match query {
        Ok(params) => params,
        Err(error) => return vapix_error(error.body_text()),
    };

    if params.camera.is_some_and(|camera| camera != 1) {
        return vapix_error("Only camera 1 is supported");
    }

    if let Some(info) = params.info {
        if info != 1 {
            return vapix_error("info must be 1");
        }
        if params.rpan.is_some() {
            return vapix_error("info cannot be combined with PTZ commands");
        }
        return text_response(StatusCode::OK, AVAILABLE_COMMANDS);
    }

    let Some(rpan) = params.rpan else {
        return vapix_error("Unsupported PTZ command");
    };
    if !(-360.0..=360.0).contains(&rpan) {
        return vapix_error("rpan must be between -360 and 360");
    }

    let mut camera = match camera.lock() {
        Ok(camera) => camera,
        Err(_) => return vapix_error("Camera state unavailable"),
    };
    camera.pan();

    text_response(StatusCode::NO_CONTENT, "")
}

fn vapix_error(message: impl Display) -> Response {
    text_response(StatusCode::OK, format!("Error:{message}"))
}

fn text_response(status: StatusCode, body: impl Into<String>) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain")],
        body.into(),
    )
        .into_response()
}
```

- [ ] **Step 5: Format the Rust files**

Run: `cargo fmt --all -- --check`

Expected: failure only if formatting differs.

If it fails, run `cargo fmt --all`, then rerun `cargo fmt --all -- --check` and expect success.

- [ ] **Step 6: Run the focused PTZ tests**

Run: `cargo test -p camera ptz_`

Expected: `ptz_info_lists_implemented_commands`, `ptz_relative_pan_returns_no_content`, and `ptz_errors_use_vapix_response_format` pass. Cargo updates `Cargo.lock` with the direct Serde dependency metadata as needed.

- [ ] **Step 7: Run all camera tests and static checks**

Run: `cargo test -p camera`

Expected: all five tests pass.

Run: `cargo clippy -p camera --all-targets -- -D warnings`

Expected: success with no warnings.

Run: `git diff --check`

Expected: no whitespace errors.

## Completion Criteria

- Both `/axis-cgi/com/ptz.cgi?info=1` and `/axis-cgi/com/ptz.cgi?rpan=10` work through the same Axum route.
- `camera=1` is accepted and other channel numbers produce a VAPIX-formatted error.
- Unknown, malformed, mixed information/movement, missing, and out-of-range queries produce `200 text/plain` errors beginning with `Error:`.
- Successful relative pan dispatch calls `Camera::pan` and returns `204 text/plain` with an empty body.
- Existing health and not-found behavior still passes.
- Formatting, tests, Clippy, and whitespace checks all pass.
