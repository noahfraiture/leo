//! Typed visual judgments and citation validation; software checks identity, not visual truth.
mod model;
mod validation;

pub use model::{
    CriterionAssessment, EvidenceRequest, EvidenceSelection, FinalFinding, Judgment, ReaderFinding,
    ReviewDisposition, ReviewDispositionKind, ReviewFinding, VisualFact,
};
pub use validation::{validate_final, validate_reader, validate_review, validate_selection};
