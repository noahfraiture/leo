//! Offline session-video planning, materialization, model analysis, and resumable results.

mod agent;
mod analyzer;
mod error;
mod evidence;
mod process;
mod run;
mod session;
mod video;
pub use evidence::gather_evidence_with_budget;
pub use evidence::{PlaybackClip, prepare_playback};
pub use process::stop_media_processes;

pub use agent::{AnalysisResponse, ChecklistProgress, Observation, OpenAiConfig};
pub use analyzer::{AnalysisCheckpoint, AnalysisWarning};
pub use error::Error;
pub use evidence::{
    CandidateFrame, EligiblePeriod, EvidenceError, EvidenceInterval, EvidenceManifest,
    EvidenceWindow, RecordingSource, SentImage, build_evidence, gather_evidence, materialize_frame,
    plan_windows, verify_sources,
};
pub use run::{
    ArchivedRun, Consolidation, CreateRun, CriterionAssessment, EvidencePolicy, ExecuteRun,
    Finding, HumanCorrection, JobKind, JobOutput, JobRecord, JobState, Judgment, OperatorReview,
    ReviewMark, RunCheckpoint, RunError, RunHistory, RunInputs, RunResult, RunSnapshot, RunStage,
    RunStore, SavedResponse, ScanFinding, ScanObservation, StudentAssessment, StudentOutcome,
    create_run, execute_run, request_recheck,
};
pub use session::{AnalyzeSession, analyze_session};

#[cfg(feature = "test-support")]
pub use agent::{Agent, Error as ProviderError, LocalChatAgent, OpenAiAgent};
#[cfg(feature = "test-support")]
pub use analyzer::{Analyzer, BatchSummary, PlanSummary};
