use std::{
    fs::File,
    io::{ErrorKind, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};

use crate::analysis::agent::AnalysisResponse;

use super::error::{Error, Result};

/// Durable responses from completed batches, ordered by batch index.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct AnalysisProgress {
    pub completed_batches: Vec<CompletedBatch>,
}

/// One successfully analyzed batch and the response used by the next request.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CompletedBatch {
    pub index: usize,
    pub response: AnalysisResponse,
}

impl AnalysisProgress {
    /// Loads an existing checkpoint or starts empty when the file does not exist.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        match File::open(path) {
            Ok(file) => Ok(serde_json::from_reader(file)?),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    /// Atomically replaces the checkpoint after its complete JSON has reached disk.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), self)?;
        temporary.as_file_mut().write_all(b"\n")?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;

        Ok(())
    }

    /// Verifies that stored results still form a prefix of the rebuilt batch plan.
    pub(crate) fn validate(&self, total_batches: usize) -> Result<()> {
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

    pub(crate) fn next_batch_index(&self) -> usize {
        self.completed_batches.len()
    }

    pub(crate) fn previous_response(&self) -> Option<&AnalysisResponse> {
        self.completed_batches.last().map(|batch| &batch.response)
    }
}

#[cfg(test)]
mod tests {
    use crate::analysis::agent::{AnalysisResponse, ChecklistProgress, Observation};

    use super::{AnalysisProgress, CompletedBatch};

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

    #[test]
    fn missing_checkpoint_starts_with_no_completed_batches() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");

        let progress = AnalysisProgress::load(&checkpoint)
            .expect("missing checkpoint should create empty progress");

        assert_eq!(progress.next_batch_index(), 0);
        assert!(progress.previous_response().is_none());
    }

    #[test]
    fn checkpoint_round_trips_completed_responses() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let progress = AnalysisProgress {
            completed_batches: vec![CompletedBatch {
                index: 0,
                response: response("Batch zero is complete."),
            }],
        };

        progress.save(&checkpoint).expect("checkpoint should save");
        let loaded = AnalysisProgress::load(&checkpoint).expect("checkpoint should load");

        assert_eq!(loaded.next_batch_index(), 1);
        assert_eq!(
            loaded
                .previous_response()
                .expect("response should be restored")
                .sequence_summary,
            "Batch zero is complete."
        );

        let updated = AnalysisProgress {
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
        let reloaded = AnalysisProgress::load(&checkpoint).expect("checkpoint should reload");

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
    fn validation_rejects_non_contiguous_batch_indices() {
        let progress = AnalysisProgress {
            completed_batches: vec![CompletedBatch {
                index: 1,
                response: response("Wrong index."),
            }],
        };

        let error = progress
            .validate(2)
            .expect_err("batch indices must start at zero");

        assert!(error.to_string().contains("not contiguous"));
    }

    #[test]
    fn malformed_checkpoint_is_reported_without_replacement() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        std::fs::write(&checkpoint, b"not json").expect("malformed checkpoint should be written");

        let error = AnalysisProgress::load(&checkpoint)
            .expect_err("malformed checkpoint should not be accepted");

        assert!(error.to_string().contains("not valid JSON"));
        assert_eq!(
            std::fs::read(&checkpoint).expect("checkpoint should remain readable"),
            b"not json"
        );
    }
}
