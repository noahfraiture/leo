use crate::http::router::AppState;
use hypertext::prelude::*;

/// Render a full HTML document around a UI route body.
pub fn document(state: &AppState, title: &str, body: impl Renderable) -> impl Renderable {
    rsx! {
        <html lang="en" data-theme="goodfox">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <title>(title)</title>
                <script src="https://cdn.jsdelivr.net/npm/htmx.org@2.0.8/dist/htmx.min.js"></script>
                <script src="https://cdn.jsdelivr.net/npm/alpinejs@3.15.11/dist/cdn.min.js"></script>
                (state.assets())
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
