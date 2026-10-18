use std::{
    fs,
    path::{Path, PathBuf},
};

use uuid::Uuid;

use super::{
    Checklist, Error, ExerciseDraft, ExerciseRevision, ImageAsset, ReferencePreview, Result,
    UnavailableExercise,
    image::{self, MAX_IMAGE_BYTES},
    model::SCHEMA_VERSION,
    persistence,
};

/// Readable configurations and per-folder failures, preserving access to the healthy library.
#[derive(Debug, Default)]
pub struct ExerciseCatalog {
    pub drafts: Vec<ExerciseDraft>,
    pub unavailable: Vec<UnavailableExercise>,
}

/// Blocking local persistence for exercise drafts, immutable publications, and image originals.
/// Call from a worker when used by the desktop UI. The root is the `data/exercises` directory.
#[derive(Clone, Debug)]
pub struct ExerciseStore {
    pub root: PathBuf,
}

impl ExerciseStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Create an editable configuration without an accepted checklist or model calls.
    pub fn create_draft(&self, name: impl Into<String>) -> Result<ExerciseDraft> {
        persistence::ensure_directory(&self.root)?;
        let draft = ExerciseDraft {
            schema_version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            edit_version: 0,
            checklist: Checklist {
                format_version: "operator-text-v1".into(),
                criteria: Vec::new(),
            },
            references: Vec::new(),
            preparation: Default::default(),
        };
        let directory = self.root.join(draft.id.to_string());
        fs::create_dir(&directory)?;
        persistence::write_json(&directory.join("draft.json"), &draft, false)?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(draft)
    }

    /// Enumerate saved drafts and report individual damaged folders without hiding healthy ones.
    pub fn list(&self) -> Result<ExerciseCatalog> {
        if !self.root.try_exists()? {
            return Ok(ExerciseCatalog::default());
        }
        persistence::ensure_directory(&self.root)?;
        let mut catalog = ExerciseCatalog::default();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            let directory = entry.path();
            let loaded = (|| {
                if !entry.file_type()?.is_dir() || entry.file_name() != id.to_string().as_str() {
                    return Err(Error::InvalidStorageEntry);
                }
                // An interrupted initial publication has no saved configuration to enumerate.
                if directory.join("draft.json").try_exists()? {
                    self.load_draft(id).map(Some)
                } else {
                    Ok(None)
                }
            })();
            match loaded {
                Ok(Some(draft)) => catalog.drafts.push(draft),
                Ok(None) => {}
                Err(source) => catalog.unavailable.push(UnavailableExercise {
                    exercise_id: id,
                    directory,
                    source,
                }),
            }
        }
        catalog
            .drafts
            .sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
        catalog
            .unavailable
            .sort_by(|left, right| left.directory.cmp(&right.directory));
        Ok(catalog)
    }

    pub fn load_draft(&self, exercise_id: Uuid) -> Result<ExerciseDraft> {
        let directory = self.directory(exercise_id)?;
        let draft: ExerciseDraft = persistence::read_json(&directory.join("draft.json"))?;
        draft.validate()?;
        if draft.id != exercise_id {
            return Err(Error::Invalid("exercise identity mismatch".into()));
        }
        Ok(draft)
    }

    /// Save edits only if edit_version still matches disk. Return the new version to the editor.
    pub fn save_draft(&self, draft: &ExerciseDraft) -> Result<ExerciseDraft> {
        draft.validate()?;
        let directory = self.directory(draft.id)?;
        let _lock = persistence::lock(&directory)?;
        let current = self.load_draft(draft.id)?;
        if current.edit_version != draft.edit_version {
            return Err(Error::StaleDraft);
        }
        self.verify_references(draft)?;
        let mut saved = draft.clone();
        saved.edit_version = current
            .edit_version
            .checked_add(1)
            .ok_or(Error::RevisionOverflow)?;
        persistence::write_json(&directory.join("draft.json"), &saved, true)?;
        Ok(saved)
    }

    /// Publish exactly the saved operator draft. An unsaved or stale snapshot cannot be published.
    pub fn publish_revision(&self, draft: &ExerciseDraft) -> Result<ExerciseRevision> {
        draft.validate_for_publication()?;
        let directory = self.directory(draft.id)?;
        let _lock = persistence::lock(&directory)?;
        if self.load_draft(draft.id)? != *draft {
            return Err(Error::StaleDraft);
        }
        self.verify_references(draft)?;
        let revision = ExerciseRevision {
            schema_version: SCHEMA_VERSION,
            id: Uuid::new_v4(),
            exercise_id: draft.id,
            edit_version: draft.edit_version,
            name: draft.name.clone(),
            checklist: draft.checklist.clone(),
            references: draft.references.clone(),
            preparation: draft.preparation.clone(),
        };
        persistence::write_json(
            &directory
                .join("revisions")
                .join(format!("{}.json", revision.id)),
            &revision,
            false,
        )?;
        Ok(revision)
    }

    /// Load the operator snapshot used by a run, independently of subsequent draft changes.
    pub fn load_revision(&self, exercise_id: Uuid, revision_id: Uuid) -> Result<ExerciseRevision> {
        let directory = self.directory(exercise_id)?.join("revisions");
        persistence::require_directory(&directory)?;
        let revision: ExerciseRevision =
            persistence::read_json(&directory.join(format!("{revision_id}.json")))?;
        if revision.schema_version != SCHEMA_VERSION
            || revision.exercise_id != exercise_id
            || revision.id != revision_id
        {
            return Err(Error::Invalid(
                "exercise revision identity or schema mismatch".into(),
            ));
        }
        ExerciseDraft {
            schema_version: revision.schema_version,
            id: exercise_id,
            name: revision.name.clone(),
            edit_version: revision.edit_version,
            checklist: revision.checklist.clone(),
            references: revision.references.clone(),
            preparation: revision.preparation.clone(),
        }
        .validate_for_publication()?;
        Ok(revision)
    }

    /// Enumerate immutable publications by edit version, leaving the current draft untouched.
    pub fn revisions(&self, exercise_id: Uuid) -> Result<Vec<ExerciseRevision>> {
        let directory = self.directory(exercise_id)?.join("revisions");
        if !directory.try_exists()? {
            return Ok(Vec::new());
        }
        if !fs::symlink_metadata(&directory)?.file_type().is_dir() {
            return Err(Error::InvalidStorageEntry);
        }
        let mut revisions = Vec::new();
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(|name| Uuid::parse_str(name).ok())
            else {
                continue;
            };
            revisions.push(self.load_revision(exercise_id, id)?);
        }
        revisions.sort_by_key(|revision| (revision.edit_version, revision.id));
        Ok(revisions)
    }

    /// Copy and validate an image original. Labels and criterion links are supplied manually later.
    /// Asset files are complete before this returns; failed imports cannot create a draft reference.
    pub fn import_reference(&self, exercise_id: Uuid, source: &Path) -> Result<ImageAsset> {
        self.load_draft(exercise_id)?;
        let bytes = persistence::read_bytes(source, MAX_IMAGE_BYTES)?;
        let asset = image::inspect(&bytes)?;
        let directory = self.directory(exercise_id)?.join("assets");
        persistence::ensure_directory(&directory)?;
        persistence::write_bytes(
            &directory.join(format!("{}.{}", asset.id, asset.media_type.extension())),
            &bytes,
            false,
        )?;
        persistence::write_json(&directory.join(format!("{}.json", asset.id)), &asset, false)?;
        Ok(asset)
    }

    /// Return verified original bytes for a run snapshot. Missing or altered originals are errors.
    pub fn read_reference(&self, exercise_id: Uuid, asset: &ImageAsset) -> Result<Vec<u8>> {
        let directory = self.directory(exercise_id)?.join("assets");
        persistence::require_directory(&directory)?;
        let stored: ImageAsset =
            persistence::read_json(&directory.join(format!("{}.json", asset.id)))?;
        if stored != *asset {
            return Err(Error::ChangedAsset(asset.id));
        }
        let bytes = persistence::read_bytes(
            &directory.join(format!("{}.{}", asset.id, asset.media_type.extension())),
            MAX_IMAGE_BYTES,
        )?;
        if bytes.len() as u64 != asset.byte_length || image::hash(&bytes) != asset.sha256 {
            return Err(Error::ChangedAsset(asset.id));
        }
        Ok(bytes)
    }

    /// Create a correctly oriented, bounded derivative while preserving the imported original.
    pub fn preview_reference(
        &self,
        exercise_id: Uuid,
        asset: &ImageAsset,
        maximum_long_edge: u32,
    ) -> Result<ReferencePreview> {
        image::preview_reference_bytes(&self.read_reference(exercise_id, asset)?, maximum_long_edge)
    }

    fn verify_references(&self, draft: &ExerciseDraft) -> Result<()> {
        for reference in &draft.references {
            self.read_reference(draft.id, &reference.image)?;
        }
        Ok(())
    }

    fn directory(&self, exercise_id: Uuid) -> Result<PathBuf> {
        if self.root.try_exists()? {
            persistence::require_directory(&self.root)?;
        }
        let directory = self.root.join(exercise_id.to_string());
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(directory),
            Ok(_) => Err(Error::InvalidStorageEntry),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(Error::NotFound(exercise_id))
            }
            Err(error) => Err(error.into()),
        }
    }
}
