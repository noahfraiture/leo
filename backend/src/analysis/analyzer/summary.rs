//! Read-only counts and timestamp bounds from the resolved production analysis plan.

use serde::Serialize;

/// Unique evidence counts and the complete provider work planned for one session.
#[derive(Debug, Serialize)]
pub struct PlanSummary {
    pub frame_sets: usize,
    pub unique_images: usize,
    pub batches: Vec<BatchSummary>,
}

/// One request's evidence, including overlap with the preceding request.
#[derive(Debug, Serialize)]
pub struct BatchSummary {
    pub frame_sets: usize,
    pub images: usize,
    pub first_offset_ms: u128,
    pub last_offset_ms: u128,
}
