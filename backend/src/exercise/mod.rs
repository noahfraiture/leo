//! Operator-owned checklists and manually labeled reference images.
//! Blocking store operations publish immutable revisions; model proposals never publish them.

mod error;
mod formatter;
mod image;
mod model;
mod persistence;
mod preparation;
mod review;
mod store;

pub use error::{Error, Result, UnavailableExercise};
pub use formatter::{CHECKLIST_FORMAT_VERSION, format_checklist};
pub use image::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, ReferencePreview, preview_reference_bytes};
pub use model::{
    Checklist, Criterion, ExerciseDraft, ExerciseRevision, ImageAsset, ImageMediaType,
    ReferenceAsset,
};
pub use preparation::{ChecklistOrder, Clarification, Preparation};
pub use review::{
    ChecklistProposal, ProposalComment, ProposalCommentKind, ProposedCriterion, SourceRange,
};
pub use store::{ExerciseCatalog, ExerciseStore};

#[cfg(test)]
mod tests;
