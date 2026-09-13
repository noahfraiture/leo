//! Operator-owned checklists and manually labeled reference images.
//! Blocking store operations publish immutable revisions; model proposals never publish them.

mod error;
mod image;
mod model;
mod persistence;
mod review;
mod store;

pub use error::{Error, Result};
pub use image::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, ReferencePreview, preview_reference_bytes};
pub use model::{
    Checklist, Criterion, ExerciseDraft, ExerciseRevision, ImageAsset, ImageMediaType,
    ReferenceAsset,
};
pub use review::{
    ChecklistProposal, ProposalComment, ProposalCommentKind, ProposedCriterion, SourceRange,
};
pub use store::ExerciseStore;

#[cfg(test)]
mod tests;
