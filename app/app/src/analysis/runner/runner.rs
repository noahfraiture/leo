use std::path::PathBuf;

use rig_core::completion::CompletionModel;

use crate::analysis::agent::{Agent, AnalysisBatch, AnalysisRequest, AnalysisResponse};

use super::{
    error::{Error, Result},
    progress::{AnalysisProgress, CompletedBatch},
};

/// Advances a planned analysis one materialized batch at a time and checkpoints each result.
pub(crate) struct AnalysisRunner<M: CompletionModel> {
    agent: Agent<M>,
    checklist: String,
    total_batches: usize,
    progress_path: PathBuf,
    progress: AnalysisProgress,
}

impl<M: CompletionModel> AnalysisRunner<M> {
    /// Resumes from a checkpoint after validating it against the rebuilt batch count.
    pub(crate) fn resume(
        agent: Agent<M>,
        checklist: String,
        total_batches: usize,
        progress_path: PathBuf,
    ) -> Result<Self> {
        let progress = AnalysisProgress::load(&progress_path)?;
        progress.validate(total_batches)?;

        Ok(Self {
            agent,
            checklist,
            total_batches,
            progress_path,
            progress,
        })
    }

    /// Index the caller should materialize next after rebuilding the batch plan.
    pub(crate) fn next_batch_index(&self) -> usize {
        self.progress.next_batch_index()
    }

    /// Analyzes the next batch and only advances in-memory progress after persistence succeeds.
    pub(crate) async fn analyze_next(
        &mut self,
        batch: &AnalysisBatch,
    ) -> Result<&AnalysisResponse> {
        let index = self.next_batch_index();
        if index >= self.total_batches {
            return Err(Error::AnalysisComplete {
                total: self.total_batches,
            });
        }

        let response = self
            .agent
            .analyze(AnalysisRequest {
                batch,
                checklist: &self.checklist,
                previous: self.progress.previous_response(),
            })
            .await?;
        self.progress
            .completed_batches
            .push(CompletedBatch { index, response });

        if let Err(error) = self.progress.save(&self.progress_path) {
            // Keep memory aligned with disk so callers can safely retry this batch.
            self.progress.completed_batches.pop();
            return Err(error);
        }

        Ok(&self
            .progress
            .completed_batches
            .last()
            .expect("the completed response was just appended")
            .response)
    }
}

#[cfg(test)]
mod tests {
    use rig_core::completion::Message;
    use rig_core::message::UserContent;
    use rig_core::test_utils::{MockCompletionModel, MockTurn};

    use crate::analysis::agent::{
        Agent, AnalysisBatch, AnalysisResponse, ChecklistProgress, Observation,
    };

    use super::AnalysisRunner;

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

    #[tokio::test]
    async fn successful_batch_advances_and_persists_progress() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let expected = response("The first batch is complete.");
        let model = MockCompletionModel::text(
            serde_json::to_string(&expected).expect("response should serialize"),
        );
        let mut runner = AnalysisRunner::resume(
            Agent::new(model),
            "Start the exercise".into(),
            2,
            checkpoint.clone(),
        )
        .expect("new analysis should start");
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };

        let actual = runner
            .analyze_next(&batch)
            .await
            .expect("batch should be analyzed");

        assert_eq!(actual, &expected);
        assert_eq!(runner.next_batch_index(), 1);
        assert!(checkpoint.exists());
    }

    #[tokio::test]
    async fn resumed_runner_sends_the_previous_response_to_the_next_batch() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let first = response("The first batch is complete.");
        let first_model = MockCompletionModel::text(
            serde_json::to_string(&first).expect("response should serialize"),
        );
        let mut first_runner = AnalysisRunner::resume(
            Agent::new(first_model),
            "Start the exercise".into(),
            2,
            checkpoint.clone(),
        )
        .expect("new analysis should start");
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };
        first_runner
            .analyze_next(&batch)
            .await
            .expect("first batch should complete");
        drop(first_runner);

        let second = response("Both batches are complete.");
        let second_model = MockCompletionModel::text(
            serde_json::to_string(&second).expect("response should serialize"),
        );
        let recorded_model = second_model.clone();
        let mut resumed = AnalysisRunner::resume(
            Agent::new(second_model),
            "Start the exercise".into(),
            2,
            checkpoint,
        )
        .expect("analysis should resume");

        assert_eq!(resumed.next_batch_index(), 1);
        resumed
            .analyze_next(&batch)
            .await
            .expect("second batch should complete");

        let requests = recorded_model.requests();
        let Message::User { content } = requests[0]
            .chat_history
            .iter()
            .last()
            .expect("request should contain a user message")
        else {
            panic!("last request message should be from the user");
        };

        assert!(matches!(
            content.first(),
            UserContent::Text(text) if text.text.contains("The first batch is complete.")
        ));
    }

    #[tokio::test]
    async fn failed_model_request_does_not_advance_or_write_progress() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("analysis.json");
        let model = MockCompletionModel::new([MockTurn::error("provider unavailable")]);
        let mut runner = AnalysisRunner::resume(
            Agent::new(model),
            "Start the exercise".into(),
            1,
            checkpoint.clone(),
        )
        .expect("new analysis should start");
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };

        let result = runner.analyze_next(&batch).await;

        assert!(result.is_err());
        assert_eq!(runner.next_batch_index(), 0);
        assert!(!checkpoint.exists());
    }

    #[tokio::test]
    async fn failed_checkpoint_write_does_not_advance_progress() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let checkpoint = directory.path().join("missing").join("analysis.json");
        let expected = response("The first batch is complete.");
        let model = MockCompletionModel::text(
            serde_json::to_string(&expected).expect("response should serialize"),
        );
        let mut runner = AnalysisRunner::resume(
            Agent::new(model),
            "Start the exercise".into(),
            1,
            checkpoint.clone(),
        )
        .expect("missing checkpoint should start a new analysis");
        let batch = AnalysisBatch {
            frame_sets: Vec::new(),
        };

        let result = runner.analyze_next(&batch).await;

        assert!(result.is_err());
        assert_eq!(runner.next_batch_index(), 0);
        assert!(!checkpoint.exists());
    }
}
