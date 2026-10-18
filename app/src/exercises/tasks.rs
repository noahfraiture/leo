use std::path::PathBuf;

use backend::exercise::{ExerciseDraft, ExerciseStore, ReferenceAsset};
use base64::{Engine, engine::general_purpose::STANDARD};
use dioxus::prelude::*;

use super::state::{ExerciseContext, ExerciseEditorState};

/// Restore stored configurations and their latest operator publications at application startup.
pub fn load(mut context: ExerciseContext) {
    context.state.write().loading = true;
    let store = context.store;
    dioxus::dioxus_core::spawn_forever(async move {
        let loaded = tokio::task::spawn_blocking(move || {
            let catalog = store.list()?;
            let editors = catalog
                .drafts
                .into_iter()
                .map(|draft| load_editor(&store, draft))
                .collect::<Vec<_>>();
            Ok::<_, backend::exercise::Error>((editors, catalog.unavailable))
        })
        .await;
        let mut page = context.state.write();
        page.loading = false;
        match completed(loaded) {
            Ok((editors, unavailable)) => {
                page.unavailable = unavailable;
                for editor in editors {
                    page.configurations.insert(editor.draft.id, editor);
                }
                page.selected_id = page.configurations.keys().next().copied();
            }
            Err(error) => page.error = Some(error),
        }
    });
}

fn load_editor(store: &ExerciseStore, draft: ExerciseDraft) -> ExerciseEditorState {
    let mut errors = Vec::new();
    let published = match store.revisions(draft.id) {
        Ok(mut revisions) => revisions.pop(),
        Err(error) => {
            errors.push(format!("Saved publications could not load: {error}. The draft remains editable. Existing publications need repair."));
            None
        }
    };
    let mut editor = ExerciseEditorState::new(draft, published);
    for reference in &editor.draft.references {
        match store.preview_reference(editor.draft.id, &reference.image, 320) {
            Ok(preview) => {
                editor
                    .previews
                    .insert(reference.image.id, png_data_uri(&preview.png));
            }
            Err(error) => errors.push(error.to_string()),
        }
    }
    if !errors.is_empty() {
        editor.storage_error = Some(errors.join("\n"));
    }
    editor
}

pub fn create(mut context: ExerciseContext) {
    if context.state.read().creating {
        return;
    }
    context.state.write().creating = true;
    let store = context.store;
    dioxus::dioxus_core::spawn_forever(async move {
        let result = completed(
            tokio::task::spawn_blocking(move || store.create_draft("New exercise")).await,
        );
        let mut page = context.state.write();
        page.creating = false;
        match result {
            Ok(draft) => {
                page.selected_id = Some(draft.id);
                page.configurations
                    .insert(draft.id, ExerciseEditorState::new(draft, None));
                page.error = None;
            }
            Err(error) => page.error = Some(error),
        }
    });
}

/// Save exactly the clicked snapshot. Publication is the operator's explicit final-save action.
pub fn save(mut context: ExerciseContext, exercise_id: uuid::Uuid, publish: bool) {
    let draft = {
        let mut page = context.state.write();
        let Some(editor) = page.configurations.get_mut(&exercise_id) else {
            return;
        };
        let unchanged = editor.draft == editor.saved;
        let already_published = editor
            .published
            .as_ref()
            .is_some_and(|revision| revision.edit_version == editor.saved.edit_version);
        if editor.saving || editor.importing || (unchanged && (!publish || already_published)) {
            return;
        }
        editor.saving = true;
        editor.error = None;
        editor.draft.clone()
    };
    let submitted = draft.clone();
    let store = context.store;
    dioxus::dioxus_core::spawn_forever(async move {
        let result = completed(
            tokio::task::spawn_blocking(move || {
                if publish {
                    draft.validate_for_publication()?;
                }
                let saved = store.save_draft(&draft)?;
                let publication = if publish {
                    store.publish_revision(&saved).map(Some)
                } else {
                    Ok(None)
                };
                Ok::<_, backend::exercise::Error>((saved, publication))
            })
            .await,
        );
        let mut page = context.state.write();
        let Some(editor) = page.configurations.get_mut(&exercise_id) else {
            return;
        };
        editor.saving = false;
        match result {
            Ok((saved, publication)) => {
                let (revision, error) = match publication {
                    Ok(revision) => (revision, None),
                    Err(error) => (None, Some(error.to_string())),
                };
                editor.finish_save(&submitted, saved, revision);
                editor.error = error;
            }
            Err(error) => editor.error = Some(error),
        }
    });
}

/// Validate, copy, and thumbnail a file off the UI thread; labels remain empty for manual entry.
pub fn import(mut context: ExerciseContext, exercise_id: uuid::Uuid, source: PathBuf) {
    {
        let mut page = context.state.write();
        let Some(editor) = page.configurations.get_mut(&exercise_id) else {
            return;
        };
        if editor.importing || editor.saving {
            return;
        }
        editor.importing = true;
        editor.error = None;
    }
    let store = context.store;
    dioxus::dioxus_core::spawn_forever(async move {
        let result = completed(
            tokio::task::spawn_blocking(move || {
                let image = store.import_reference(exercise_id, &source)?;
                let preview = store.preview_reference(exercise_id, &image, 320)?;
                let reference = ReferenceAsset {
                    image,
                    object_label: String::new(),
                    state_label: String::new(),
                    purpose: String::new(),
                    criterion_ids: Vec::new(),
                };
                Ok::<_, backend::exercise::Error>((reference, png_data_uri(&preview.png)))
            })
            .await,
        );
        let mut page = context.state.write();
        match result {
            Ok((reference, preview)) => page.finish_import(exercise_id, reference, preview),
            Err(error) => {
                if let Some(editor) = page.configurations.get_mut(&exercise_id) {
                    editor.importing = false;
                    editor.error = Some(error);
                }
            }
        }
    });
}

fn completed<T>(
    result: Result<backend::exercise::Result<T>, tokio::task::JoinError>,
) -> Result<T, String> {
    result
        .map_err(|_| "The exercise storage task could not finish.".to_owned())?
        .map_err(|error| error.to_string())
}

fn png_data_uri(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", STANDARD.encode(bytes))
}

#[cfg(test)]
#[path = "tests/loading.rs"]
mod tests;
