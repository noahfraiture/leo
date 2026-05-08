use crate::http::router::AppState;
use hypertext::{Raw, prelude::*};

const STYLES: &str = include_str!("styles.generated.css");

/// Render a full HTML document around a UI route body.
pub fn document(_state: &AppState, title: &str, body: impl Renderable) -> impl Renderable {
    rsx! {
        <html lang="en" data-theme="goodfox">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>(title)</title>
                <style>(Raw::dangerously_create(STYLES))</style>
                <script src="https://cdn.jsdelivr.net/npm/htmx.org@2.0.8/dist/htmx.min.js"></script>
            </head>
            <body class="min-h-screen bg-base-100 text-base-content">
                (body)
            </body>
        </html>
    }
}

/// Default HTMX fallback body for routes that intentionally do not serve
/// fragments.
pub fn not_found_fragment() -> &'static str {
    "Not Found"
}
