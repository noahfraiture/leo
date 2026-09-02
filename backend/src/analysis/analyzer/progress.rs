use std::{fs::File, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::analysis::{agent::AnalysisResponse, error::Error, video::AnalysisWarning};

/// Current on-disk analysis checkpoint schema.
pub const ANALYSIS_SCHEMA_VERSION: u8 = 2;

/// Durable analysis identity, warnings, and completed response prefix for one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisCheckpoint {
    /// Version of the persisted checkpoint schema.
    pub schema_version: u8,
    /// Session UUID whose analysis is being checkpointed.
    pub session_id: Uuid,
    /// Correct-sequence checklist used for every model request.
    pub checklist: String,
    /// Path-independent SHA-256 identity of the canonical frame plan.
    pub plan_fingerprint: String,
    /// Batch count from the freshly rebuilt canonical plan.
    pub total_batches: usize,
    /// Physical recording gaps found while rebuilding the plan.
    pub warnings: Vec<AnalysisWarning>,
    /// Contiguous completed prefix; vector position is the batch index.
    pub responses: Vec<AnalysisResponse>,
}

impl AnalysisCheckpoint {
    /// Reads and validates a direct regular checkpoint file for the expected session.
    pub fn read(path: &Path, expected_session_id: Uuid) -> Result<Self, Error> {
        if !std::fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(Error::InvalidCheckpointFile);
        }

        let checkpoint: Self = serde_json::from_reader(File::open(path)?)?;
        checkpoint.validate(expected_session_id)?;
        Ok(checkpoint)
    }

    fn validate(&self, expected_session_id: Uuid) -> Result<(), Error> {
        if self.schema_version != ANALYSIS_SCHEMA_VERSION {
            return Err(Error::CheckpointSchema {
                expected: ANALYSIS_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        if self.session_id != expected_session_id {
            return Err(Error::CheckpointSession {
                expected: expected_session_id,
                actual: self.session_id,
            });
        }
        if self.checklist.is_empty() {
            return Err(Error::EmptyCheckpointChecklist);
        }
        if self.plan_fingerprint.is_empty() {
            return Err(Error::EmptyCheckpointPlanFingerprint);
        }
        if self.responses.len() > self.total_batches {
            return Err(Error::ProgressExceedsPlan {
                completed: self.responses.len(),
                total: self.total_batches,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/progress.rs"]
mod tests;
