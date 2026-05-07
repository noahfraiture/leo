use std::{collections::HashMap, fs, path::PathBuf};

use hypertext::prelude::*;
use serde::Deserialize;

#[derive(Clone)]
pub struct FrontendAssets {
    script_path: String,
    stylesheet_paths: Vec<String>,
}

#[derive(Deserialize)]
struct ManifestEntry {
    file: String,
    #[serde(default)]
    css: Vec<String>,
}

impl FrontendAssets {
    // Read the Vite manifest and expose the backend-facing asset URLs it references.
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let manifest_path = frontend_dist_dir().join("manifest.json");
        let manifest = fs::read_to_string(&manifest_path)?;
        let entries: HashMap<String, ManifestEntry> = serde_json::from_str(&manifest)?;
        let entry = entries
            .get("src/main.ts")
            .ok_or_else(|| format!("missing Vite entry \"src/main.ts\" in {manifest_path:?}"))?;
        let asset_url =
            |path: &str| format!("/assets/{}", path.strip_prefix("assets/").unwrap_or(path));

        Ok(Self {
            script_path: asset_url(&entry.file),
            stylesheet_paths: entry.css.iter().map(|path| asset_url(path)).collect(),
        })
    }

    // Render the stylesheet and module script tags required by the browser runtime.
    pub fn render_tags(&self) -> impl Renderable {
        rsx! {
            @for stylesheet_path in &self.stylesheet_paths {
                <link rel="stylesheet" href=(stylesheet_path) />
            }
            <script type="module" src=(self.script_path)></script>
        }
    }

    // Expose the built asset directory that Axum should serve under `/assets`.
    pub fn assets_dir() -> PathBuf {
        frontend_dist_dir().join("assets")
    }

    // Build a deterministic asset set for backend tests without reading the Vite manifest.
    #[cfg(test)]
    pub fn for_test(script_path: &str, stylesheet_paths: &[&str]) -> Self {
        Self {
            script_path: script_path.to_owned(),
            stylesheet_paths: stylesheet_paths
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
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
        FrontendAssets::for_test("/assets/main-test.js", &["/assets/main-test.css"])
    }

    // Ensure the rendered tags include both stylesheet and module script references.
    #[test]
    fn render_tags_include_frontend_assets() {
        let html = test_assets().render_tags().render();

        assert!(html.as_inner().contains(r#"href="/assets/main-test.css""#));
        assert!(html.as_inner().contains(r#"src="/assets/main-test.js""#));
        assert!(html.as_inner().contains(r#"type="module""#));
    }

    // Ensure the backend serves only the built frontend asset directory under `/assets`.
    #[test]
    fn assets_dir_points_to_frontend_assets_directory() {
        let assets_dir = FrontendAssets::assets_dir();

        assert!(assets_dir.ends_with(PathBuf::from("frontend/dist/assets")));
    }
}
