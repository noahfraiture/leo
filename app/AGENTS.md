# Agent Notes

This is a Dioxus 0.7 app. Keep guidance and code aligned with the current 0.7 APIs.

## Project

- Rust edition: 2024
- Dioxus dependency: `dioxus = { version = "0.7.7", features = ["router", "fullstack"] }`
- Default feature: `desktop`
- Routes live in `src/main.rs` on the `Route` enum.
- Route views live under `src/views/`.
- Shared components live under `src/components/`.
- CSS and other static assets are referenced with `asset!("/assets/...")`.

## Commands

```sh
cargo check
dx serve
```

Use `dx serve` for local development. Do not add install steps unless the user asks.

## Dioxus 0.7 Rules

- Import Dioxus items with `use dioxus::prelude::*;`.
- Root app shape:

```rust
fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! { Router::<Route> {} }
}
```

- Components are `#[component] fn Name(...) -> Element`.
- Props should be owned values such as `String` and `Vec<T>`, not borrowed values.
- Props must be `Clone + PartialEq`; use `ReadOnlySignal<T>` when props need to stay reactive.
- Use `use_signal`, `use_memo`, `use_resource`, and `use_context`; do not use old APIs like `cx`, `Scope`, or `use_state`.
- Read signals with `signal()` or `signal.read()`, and update them with `signal.set(...)`, `*signal.write() = ...`, or `signal.with_mut(...)`.
- Keep hooks at the top level of components. Do not call hooks inside `if`, loops, async blocks, or closures.
- Prefer RSX control flow directly:

```rust
rsx! {
    for item in items {
        div { "{item}" }
    }
    if enabled {
        button { "Enabled" }
    }
}
```

## Routing

- Use `#[derive(Routable, Clone, PartialEq)]` on the `Route` enum.
- Use `#[layout(Navbar)]` for shared layout.
- Use `Link { to: Route::Home {}, "Home" }` for internal navigation.
- Layouts render child routes with `Outlet::<Route> {}`.

## Fullstack

- Server functions use `#[get(...)]` or `#[post(...)]`.
- Server functions should accept and return serializable owned values.
- Server-only imports or logic belong inside the server function or behind `#[cfg(feature = "server")]`.
- For SSR/hydration-sensitive async data, prefer `use_server_future` over `use_resource`.
