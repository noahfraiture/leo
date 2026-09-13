//! Offline session-video planning, materialization, model analysis, and resumable results.

mod agent;
mod analyzer;
mod error;
mod evidence;
mod reconcile;
mod run;
mod session;
mod video;
pub use evidence::gather_evidence_with_budget;
pub use evidence::{PlaybackClip, prepare_playback};

pub use agent::{AnalysisResponse, ChecklistProgress, Observation, OpenAiConfig};
pub use analyzer::{AnalysisCheckpoint, AnalysisWarning};
pub use error::Error;
pub use evidence::{
    CandidateFrame, EligiblePeriod, EvidenceError, EvidenceInterval, EvidenceManifest,
    EvidenceWindow, RecordingSource, SentImage, build_evidence, gather_evidence, materialize_frame,
    plan_windows, verify_sources,
};
pub use reconcile::{
    CriterionAssessment, EvidenceRequest, EvidenceSelection, FinalFinding, Judgment, ReaderFinding,
    ReviewDisposition, ReviewDispositionKind, ReviewFinding, VisualFact,
};
pub use run::{
    CreateRun, CriterionWindows, EvidencePolicy, ExecuteRun, FinalResult, HumanCorrection, JobKind,
    JobRecord, JobState, ReaderResult, ReviewResult, RunCheckpoint, RunError, RunInputs, RunResult,
    RunSnapshot, RunStage, RunStore, SelectionResult, create_run, execute_run,
};
pub use session::{AnalyzeSession, analyze_session};

#[cfg(feature = "test-support")]
pub use agent::{Agent, Error as ProviderError, LocalChatAgent, OpenAiAgent};
#[cfg(feature = "test-support")]
pub use analyzer::{Analyzer, BatchSummary, PlanSummary};
#[cfg(feature = "test-support")]
pub use run::{SelectorDiagnostic, diagnose_selector};
