//! Offline video-analysis orchestration from materialized frame batches to resumable model results.

mod agent;
mod video;

pub(crate) use agent::{
    Agent, AnalysisBatch, AnalysisRequest, AnalysisResponse, ChecklistProgress,
    Error as AgentError, Observation, OpenAiAgent, PromptFrame, PromptFrameSet,
};
