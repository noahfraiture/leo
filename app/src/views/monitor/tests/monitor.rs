use super::PREVIEW_ERROR_GUIDANCE;

#[test]
fn unavailable_guidance_covers_startup_failures() {
    for cause in ["version", "PATH", "ports", "configuration", "filesystem"] {
        assert!(PREVIEW_ERROR_GUIDANCE.contains(cause));
    }
    assert!(!PREVIEW_ERROR_GUIDANCE.contains("Install"));
}
