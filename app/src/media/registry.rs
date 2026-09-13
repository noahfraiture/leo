use dioxus::{desktop::use_asset_handler, prelude::*};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Opaque asset IDs resolve only to media explicitly registered by the application.
#[derive(Clone, Default)]
pub struct MediaRegistry(pub Arc<Mutex<HashMap<String, PathBuf>>>);
impl MediaRegistry {
    /// Registers an existing bounded playback derivative, never a model-provided path.
    pub fn register(&self, path: &Path) -> Result<String, String> {
        let path = path
            .canonicalize()
            .map_err(|_| "Playback file is unavailable")?;
        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("mp4") {
            return Err("Unsupported playback file".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.0
            .lock()
            .map_err(|_| "Playback registry unavailable")?
            .insert(id.clone(), path);
        Ok(format!("/leo-media/{id}"))
    }
}

/// Mount at the application root so media requests remain valid across route changes.
pub fn use_media_registry() {
    let registry = use_hook(MediaRegistry::default);
    let context = registry.clone();
    use_context_provider(|| context);
    use_asset_handler("leo-media", move |request, responder| {
        let registry = registry.clone();
        tokio::task::spawn_blocking(move || {
            responder.respond(super::response::respond(&registry, request))
        });
    });
}
