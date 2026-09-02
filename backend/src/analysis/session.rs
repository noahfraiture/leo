//! Validates one completed session and drives its resumable analysis to completion.

use std::{fs, num::NonZeroUsize, path::PathBuf};

use rig_core::completion::CompletionModel;

use crate::{recording::list_segments, session::Session};

use super::{
    agent::{Agent, OpenAiAgent, OpenAiConfig},
    analyzer::{AnalysisCheckpoint, Analyzer},
    error::Error,
};

type Result<T> = std::result::Result<T, Error>;

/// A request to analyze one completed session directory against a checklist.
pub struct AnalyzeSession {
    /// Directory containing the event log, completion marker, and local recordings.
    pub directory: PathBuf,
    /// Correct exercise sequence supplied to every model request.
    pub checklist: String,
    /// Synchronized frame sets sent in each model request.
    pub frame_sets_per_prompt: NonZeroUsize,
    /// Frame sets repeated between adjacent model requests.
    pub overlap_frame_sets: usize,
    pub openai: OpenAiConfig,
}

/// Analyzes or resumes a completed local session and emits each durable checkpoint snapshot.
pub async fn analyze_session(
    request: AnalyzeSession,
    on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint> {
    analyze_session_with(
        request,
        |config| OpenAiAgent::from_config(config).map_err(Error::from),
        on_checkpoint,
    )
    .await
}

async fn analyze_session_with<M, F>(
    request: AnalyzeSession,
    make_agent: F,
    mut on_checkpoint: impl FnMut(AnalysisCheckpoint),
) -> Result<AnalysisCheckpoint>
where
    M: CompletionModel,
    F: FnOnce(OpenAiConfig) -> Result<Agent<M>>,
{
    let AnalyzeSession {
        directory,
        checklist,
        frame_sets_per_prompt,
        overlap_frame_sets,
        openai,
    } = request;
    if checklist.trim().is_empty() {
        return Err(Error::EmptyChecklist);
    }

    if !fs::symlink_metadata(&directory).is_ok_and(|metadata| metadata.file_type().is_dir()) {
        return Err(Error::InvalidSessionDirectory);
    }
    if !fs::symlink_metadata(directory.join("recording-complete"))
        .is_ok_and(|metadata| metadata.file_type().is_file() && metadata.len() == 0)
    {
        return Err(Error::InvalidCompletionMarker);
    }

    let session = Session::load(&directory.join("events.jsonl"))?;
    let camera_ids = session
        .cameras
        .iter()
        .map(|camera| camera.id)
        .collect::<Vec<_>>();
    let recordings_root = directory.join("recordings");
    let segments =
        tokio::task::spawn_blocking(move || list_segments(&recordings_root, &camera_ids))
            .await
            .map_err(Error::SegmentDiscoveryTask)??;
    let checkpoint_path = directory.join("analysis.json");
    let checklist = match fs::symlink_metadata(&checkpoint_path) {
        Ok(_) => checklist,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => checklist.trim().to_owned(),
        Err(error) => return Err(error.into()),
    };
    let mut analyzer = Analyzer::resume(
        segments,
        session,
        checklist,
        frame_sets_per_prompt,
        overlap_frame_sets,
        checkpoint_path,
    )
    .await?;

    let mut checkpoint = analyzer.checkpoint().clone();
    on_checkpoint(checkpoint.clone());
    if checkpoint.responses.len() == checkpoint.total_batches {
        return Ok(checkpoint);
    }

    let agent = make_agent(openai)?;
    while checkpoint.responses.len() < checkpoint.total_batches {
        analyzer.analyze_next(&agent).await?;
        checkpoint = analyzer.checkpoint().clone();
        on_checkpoint(checkpoint.clone());
    }
    Ok(checkpoint)
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
