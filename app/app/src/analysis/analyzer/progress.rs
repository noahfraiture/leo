use std::{
    fs::File,
    io::{ErrorKind, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::analysis::agent::AnalysisResponse;

use super::error::{Error, Result};

const SCHEMA_VERSION: u8 = 1;

/// Durable analysis metadata and completed responses for one rebuilt batch plan.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AnalysisCheckpoint {
    /// Version of the persisted checkpoint schema.
    pub(super) schema_version: u8,
    /// Session UUID whose analysis is being checkpointed.
    pub(super) session_id: Uuid,
    /// Batch count from the freshly rebuilt canonical plan.
    pub(super) total_batches: usize,
    /// Contiguous completed prefix in ascending batch order.
    pub(super) completed_batches: Vec<CompletedBatch>,
}

/// One successfully analyzed batch and the response used by the next request.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompletedBatch {
    /// Zero-based batch position in the rebuilt plan.
    pub(super) index: usize,
    /// Complete model response carried into the next batch.
    pub(super) response: AnalysisResponse,
}

impl AnalysisCheckpoint {
    /// Loads and validates a checkpoint, or initializes metadata when it is missing.
    pub(super) fn load(path: &Path, session_id: Uuid, total_batches: usize) -> Result<Self> {
        match File::open(path) {
            Ok(file) => {
                let checkpoint: Self = serde_json::from_reader(file)?;
                checkpoint.validate(session_id, total_batches)?;
                Ok(checkpoint)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self {
                schema_version: SCHEMA_VERSION,
                session_id,
                total_batches,
                completed_batches: Vec::new(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    /// Atomically replaces the checkpoint after its complete JSON has reached disk.
    pub(super) fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), self)?;
        temporary.as_file_mut().write_all(b"\n")?;
        temporary.as_file_mut().flush()?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;

        Ok(())
    }

    /// Verifies that stored results still form a prefix of the rebuilt batch plan.
    pub(super) fn validate(&self, session_id: Uuid, total_batches: usize) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::CheckpointSchema {
                expected: SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        if self.session_id != session_id {
            return Err(Error::CheckpointSession {
                expected: session_id,
                actual: self.session_id,
            });
        }
        if self.total_batches != total_batches {
            return Err(Error::CheckpointBatchCount {
                expected: total_batches,
                actual: self.total_batches,
            });
        }
        if self.completed_batches.len() > total_batches {
            return Err(Error::ProgressExceedsPlan {
                completed: self.completed_batches.len(),
                total: total_batches,
            });
        }

        for (expected, batch) in self.completed_batches.iter().enumerate() {
            if batch.index != expected {
                return Err(Error::NonContiguousBatch {
                    expected,
                    actual: batch.index,
                });
            }
        }

        Ok(())
    }

    /// Returns the first batch not present in the completed prefix.
    pub(super) fn next_batch_index(&self) -> usize {
        self.completed_batches.len()
    }

    /// Returns the latest complete response for the next prompt's context.
    pub(super) fn previous_response(&self) -> Option<&AnalysisResponse> {
        self.completed_batches.last().map(|batch| &batch.response)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use crate::analysis::agent::{AnalysisResponse, ChecklistProgress, Observation};

    use super::{AnalysisCheckpoint, CompletedBatch, Error};

    const TOTAL_BATCHES: usize = 2;

    fn session_id() -> Uuid {
        Uuid::from_u128(1)
    }

    fn response(summary: &str) -> AnalysisResponse {
        AnalysisResponse {
            observations: vec![Observation {
                timestamp: "00:00:01".into(),
                description: "The student starts the exercise.".into(),
            }],
            sequence_summary: summary.into(),
            checklist_progress: vec![ChecklistProgress {
                item: "Start the exercise".into(),
                status: "respected".into(),
                note: String::new(),
            }],
        }
    }

    fn load_error(
        schema_version: u8,
        stored_session_id: Uuid,
        stored_total_batches: usize,
        completed_indices: &[usize],
    ) -> Error {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("analysis.json");
        let completed_batches = completed_indices
            .iter()
            .map(|index| {
                json!({
                    "index": index,
                    "response": response("Stored response.")
                })
            })
            .collect::<Vec<_>>();
        let contents = serde_json::to_vec_pretty(&json!({
            "schema_version": schema_version,
            "session_id": stored_session_id,
            "total_batches": stored_total_batches,
            "completed_batches": completed_batches
        }))
        .expect("checkpoint should serialize");
        std::fs::write(&path, contents).expect("checkpoint should be written");

        AnalysisCheckpoint::load(&path, session_id(), TOTAL_BATCHES)
            .expect_err("invalid checkpoint should be rejected while loading")
    }

    #[test]
    fn missing_checkpoint_initializes_exact_plan_metadata() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");

        let progress = AnalysisCheckpoint::load(&checkpoint, session_id(), TOTAL_BATCHES)
            .expect("missing checkpoint should create empty progress");

        assert_eq!(progress.schema_version, 1);
        assert_eq!(progress.session_id, session_id());
        assert_eq!(progress.total_batches, TOTAL_BATCHES);
        assert_eq!(progress.next_batch_index(), 0);
        assert!(progress.previous_response().is_none());
    }

    #[test]
    fn checkpoint_round_trips_completed_responses() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let progress = AnalysisCheckpoint {
            schema_version: 1,
            session_id: session_id(),
            total_batches: TOTAL_BATCHES,
            completed_batches: vec![CompletedBatch {
                index: 0,
                response: response("Batch zero is complete."),
            }],
        };

        progress.save(&checkpoint).expect("checkpoint should save");
        let loaded = AnalysisCheckpoint::load(&checkpoint, session_id(), TOTAL_BATCHES)
            .expect("checkpoint should load");

        assert_eq!(loaded.next_batch_index(), 1);
        assert_eq!(
            loaded
                .previous_response()
                .expect("response should be restored")
                .sequence_summary,
            "Batch zero is complete."
        );
        assert!(
            std::fs::read(&checkpoint)
                .expect("checkpoint should remain readable")
                .ends_with(b"\n")
        );

        let updated = AnalysisCheckpoint {
            schema_version: 1,
            session_id: session_id(),
            total_batches: TOTAL_BATCHES,
            completed_batches: vec![
                CompletedBatch {
                    index: 0,
                    response: response("Batch zero is complete."),
                },
                CompletedBatch {
                    index: 1,
                    response: response("Batch one is complete."),
                },
            ],
        };
        updated
            .save(&checkpoint)
            .expect("existing checkpoint should be replaced");
        let reloaded = AnalysisCheckpoint::load(&checkpoint, session_id(), TOTAL_BATCHES)
            .expect("checkpoint should reload");

        assert_eq!(reloaded.next_batch_index(), 2);
        assert_eq!(
            reloaded
                .previous_response()
                .expect("latest response should be restored")
                .sequence_summary,
            "Batch one is complete."
        );
    }

    #[test]
    fn load_rejects_wrong_schema_version() {
        assert!(matches!(
            load_error(2, session_id(), TOTAL_BATCHES, &[]),
            Error::CheckpointSchema {
                expected: 1,
                actual: 2
            }
        ));
    }

    #[test]
    fn load_rejects_wrong_session_id() {
        let actual = Uuid::from_u128(2);

        assert!(matches!(
            load_error(1, actual, TOTAL_BATCHES, &[]),
            Error::CheckpointSession { expected, actual: found }
                if expected == session_id() && found == actual
        ));
    }

    #[test]
    fn load_rejects_changed_total_batch_count() {
        assert!(matches!(
            load_error(1, session_id(), TOTAL_BATCHES + 1, &[]),
            Error::CheckpointBatchCount {
                expected: TOTAL_BATCHES,
                actual: 3
            }
        ));
    }

    #[test]
    fn load_rejects_non_contiguous_batch_indices() {
        assert!(matches!(
            load_error(1, session_id(), TOTAL_BATCHES, &[1]),
            Error::NonContiguousBatch {
                expected: 0,
                actual: 1
            }
        ));
    }

    #[test]
    fn malformed_checkpoint_is_reported_without_replacement() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        std::fs::write(&checkpoint, b"not json").expect("malformed checkpoint should be written");

        let error = AnalysisCheckpoint::load(&checkpoint, session_id(), TOTAL_BATCHES)
            .expect_err("malformed checkpoint should not be accepted");

        assert!(error.to_string().contains("not valid JSON"));
        assert_eq!(
            std::fs::read(&checkpoint).expect("checkpoint should remain readable"),
            b"not json"
        );
    }
}
