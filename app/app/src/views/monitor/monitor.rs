use crate::{components::CameraFeed, preview::PreviewState};
use dioxus::prelude::*;

const PREVIEW_ERROR_GUIDANCE: &str = "Check the MediaMTX version and PATH, free the preview ports if occupied, or fix the reported configuration or filesystem issue, then restart the app.";

#[component]
pub fn Monitor() -> Element {
    match use_context::<PreviewState>() {
        PreviewState::Unavailable { message } => rsx! {
            div {
                class: "alert alert-warning m-2",
                role: "status",
                span {
                    "Live preview is unavailable: {message}. {PREVIEW_ERROR_GUIDANCE}"
                }
            }
        },
        PreviewState::Ready { feeds, reader } => rsx! {
            div {
                class: "grid grid-cols-1 gap-4 p-2 lg:grid-cols-2",
                for feed in feeds {
                    CameraFeed { feed, reader: reader.clone() }
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::PREVIEW_ERROR_GUIDANCE;

    #[test]
    fn unavailable_guidance_covers_startup_failures() {
        for cause in ["version", "PATH", "ports", "configuration", "filesystem"] {
            assert!(PREVIEW_ERROR_GUIDANCE.contains(cause));
        }
        assert!(!PREVIEW_ERROR_GUIDANCE.contains("Install"));
    }
}
