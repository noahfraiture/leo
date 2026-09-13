//! Offline session-video planning, materialization, model analysis, and resumable results.

mod agent;
mod analyzer;
mod error;
mod evidence;
mod session;
mod video;

pub use agent::{AnalysisResponse, ChecklistProgress, Observation, OpenAiConfig};
pub use analyzer::{AnalysisCheckpoint, AnalysisWarning};
pub use error::Error;
pub use session::{AnalyzeSession, analyze_session};

#[cfg(feature = "test-support")]
pub use agent::{Agent, Error as ProviderError, LocalChatAgent, OpenAiAgent};
#[cfg(feature = "test-support")]
pub use analyzer::{Analyzer, BatchSummary, PlanSummary};

pub use evidence::{CandidateFrame, EligiblePeriod, EvidenceInterval, EvidenceError, EvidenceManifest, EvidenceWindow, RecordingSource, SentImage, PlaybackClip, build_evidence, plan_windows, materialize_frame, verify_sources, gather_evidence, gather_evidence_with_budget, prepare_playback};
