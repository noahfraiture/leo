use thiserror::Error;

use crate::{analysis::error::AnalysisError as AiAnalysisError, db};

pub struct AnalysisSubmission {
    pub provider: Option<String>,
    pub frame_sample_rate_fps: Option<f64>,
    pub video_keys: Vec<String>,
    pub prompt: String,
}

pub struct AnalysisSnapshot {
    pub analysis: db::analysis::Analysis,
    pub events: Vec<db::analysis::AnalysisEvent>,
}

#[derive(Debug, Error)]
pub enum AnalysisJobError {
    #[error("{0}")]
    BadRequest(&'static str),
    #[error(transparent)]
    Video(#[from] db::video::VideoError),
    #[error(transparent)]
    Analysis(#[from] db::analysis::AnalysisError),
    #[error(transparent)]
    AiAnalysis(#[from] AiAnalysisError),
}
