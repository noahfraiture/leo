use std::{fs, path::PathBuf};

use hypertext::prelude::*;

#[derive(Clone)]
pub struct FrontendAssets {
    stylesheet_path: String,
}

impl FrontendAssets {
    // Ensure the stylesheet has been built before the backend starts.
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let stylesheet_path = "main.css";
        fs::metadata(Self::assets_dir().join(stylesheet_path))?;

        Ok(Self {
            stylesheet_path: format!("/assets/{stylesheet_path}"),
        })
    }

    // Render the stylesheet used by Rust-rendered pages.
    pub fn render_tags(&self) -> impl Renderable {
        rsx! {
            <link rel="stylesheet" href=(self.stylesheet_path) />
        }
    }

    // Expose the built asset directory that Axum should serve under `/assets`.
    pub fn assets_dir() -> PathBuf {
        frontend_dist_dir().join("assets")
    }

    // Build a deterministic asset set for backend tests without reading the Vite manifest.
    #[cfg(test)]
    pub fn for_test(stylesheet_path: &str) -> Self {
        Self {
            stylesheet_path: stylesheet_path.to_owned(),
        }
    }
}

// Locate the frontend build output directory from the backend crate root.
fn frontend_dist_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("backend crate should have a repository root parent")
        .join("frontend")
        .join("dist")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a deterministic asset set for testing without depending on the Vite manifest.
    fn test_assets() -> FrontendAssets {
        FrontendAssets::for_test("/assets/main-test.css")
    }

    // Ensure the rendered tags include the compiled stylesheet.
    #[test]
    fn render_tags_include_frontend_assets() {
        let html = test_assets().render_tags().render();

        assert!(html.as_inner().contains(r#"href="/assets/main-test.css""#));
        assert!(!html.as_inner().contains("<script"));
    }

    // Ensure the backend serves only the built frontend asset directory under `/assets`.
    #[test]
    fn assets_dir_points_to_frontend_assets_directory() {
        let assets_dir = FrontendAssets::assets_dir();

        assert!(assets_dir.ends_with(PathBuf::from("frontend/dist/assets")));
    }
}
