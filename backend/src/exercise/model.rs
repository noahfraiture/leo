use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Error, Result};

pub const SCHEMA_VERSION: u8 = 1;

/// One operator-confirmed requirement. Its ID survives wording edits and reordering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Criterion {
    pub id: Uuid,
    pub text: String,
}

/// The only authoritative checklist in an exercise. Comments and proposals live elsewhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checklist {
    /// Identifies the operator text representation, not a hidden grading checklist.
    pub format_version: String,
    pub criteria: Vec<Criterion>,
}

impl Checklist {
    /// Render the exact criterion wording in its operator-selected order.
    pub fn source_text(&self) -> String {
        self.criteria
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// Actual decoded media format, independent of the uploaded filename.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    Jpeg,
    Png,
}

impl ImageMediaType {
    /// File extension used for Leo's immutable original image.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    /// Content type for serving the original image bytes.
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

/// Immutable identity of an imported original. Dimensions precede EXIF orientation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageAsset {
    pub id: Uuid,
    pub sha256: String,
    pub byte_length: u64,
    pub media_type: ImageMediaType,
    pub width: u32,
    pub height: u32,
    /// EXIF orientation 1 through 8. Derivatives apply it once; originals are untouched.
    pub orientation: u8,
}

/// Operator labels for one reference image; this is instruction material, never evidence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReferenceAsset {
    pub image: ImageAsset,
    pub object_label: String,
    pub state_label: String,
    pub purpose: String,
    /// Stable criterion IDs, deliberately independent of their current display positions.
    pub criterion_ids: Vec<Uuid>,
}

/// Editable configuration. Saving advances edit_version; analysis consumes only a revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExerciseDraft {
    pub schema_version: u8,
    pub id: Uuid,
    pub name: String,
    pub edit_version: u64,
    pub checklist: Checklist,
    pub references: Vec<ReferenceAsset>,
    #[serde(default)]
    pub preparation: super::Preparation,
}

impl ExerciseDraft {
    /// Original wording for preparation; analysis uses only the final checklist.
    pub fn source_checklist(&self) -> &Checklist {
        self.preparation.source.as_ref().unwrap_or(&self.checklist)
    }

    /// Validate structure and links while allowing unfinished operator wording in a draft.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION || self.id.is_nil() {
            return Err(Error::Invalid(
                "unsupported schema or empty exercise ID".into(),
            ));
        }
        let mut criteria = HashSet::new();
        for item in &self.checklist.criteria {
            if item.id.is_nil() || !criteria.insert(item.id) {
                return Err(Error::Invalid(
                    "criterion IDs must be nonempty and unique".into(),
                ));
            }
        }
        let mut images = HashSet::new();
        for reference in &self.references {
            if reference.image.id.is_nil() || !images.insert(reference.image.id) {
                return Err(Error::Invalid(
                    "reference image IDs must be nonempty and unique".into(),
                ));
            }
            let mut links = HashSet::new();
            if reference
                .criterion_ids
                .iter()
                .any(|id| !criteria.contains(id) || !links.insert(*id))
            {
                return Err(Error::Invalid(
                    "reference links must name unique existing criteria".into(),
                ));
            }
        }
        self.preparation.validate(&self.checklist)?;
        Ok(())
    }

    /// Require a usable final checklist without inventing exercise policy or visual facts.
    pub fn validate_for_publication(&self) -> Result<()> {
        self.validate()?;
        if self.preparation.questions.iter().any(|question| {
            self.source_checklist()
                .criteria
                .iter()
                .find(|item| item.id == question.criterion_id)
                .is_none_or(|item| item.text != question.source_text)
        }) {
            return Err(Error::Invalid(
                "Review or remove questions whose original wording changed".into(),
            ));
        }
        if self.preparation.needs_reconciliation()? {
            return Err(Error::Invalid(
                "Review the final checklist after changing its source or clarification answers"
                    .into(),
            ));
        }
        if self.name.trim().is_empty()
            || self.checklist.format_version.trim().is_empty()
            || self.checklist.criteria.is_empty()
            || self
                .checklist
                .criteria
                .iter()
                .any(|item| item.text.trim().is_empty())
        {
            return Err(Error::Invalid(
                "a named exercise and nonempty checklist are required".into(),
            ));
        }
        if self.references.iter().any(|item| {
            item.object_label.trim().is_empty()
                || item.purpose.trim().is_empty()
                || item.criterion_ids.is_empty()
        }) {
            return Err(Error::Invalid(
                "each reference needs an object label, purpose, and criterion link".into(),
            ));
        }
        Ok(())
    }
}

/// An immutable operator publication copied into new analysis runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExerciseRevision {
    pub schema_version: u8,
    pub id: Uuid,
    pub exercise_id: Uuid,
    pub edit_version: u64,
    pub name: String,
    pub checklist: Checklist,
    pub references: Vec<ReferenceAsset>,
    #[serde(default)]
    pub preparation: super::Preparation,
}
