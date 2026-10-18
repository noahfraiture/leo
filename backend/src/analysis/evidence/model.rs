use super::{super::AnalysisWarning, EvidenceError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// An immutable recording file identity. Paths are relative to the selected session directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingSource {
    pub relative_path: PathBuf,
    pub camera_id: u32,
    pub start_utc_ms: i64,
    pub end_utc_ms: i64,
    pub size_bytes: u64,
    pub sha256: String,
}

/// One image sampled from the candidate's recording, not an example supplied by the instructor.
/// Its ID resolves to a source segment and exact timeline offset; models cite that ID only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateFrame {
    pub id: String,
    pub camera_id: u32,
    /// Index into EvidenceManifest.sources, never a path supplied by a model.
    pub source_index: usize,
    /// Milliseconds since session start, shared across cameras.
    pub session_offset_ms: u64,
    /// Milliseconds since this recording segment starts; used for extraction and playback.
    pub recording_offset_ms: u64,
}

/// Frozen sampling plan and recording identity used by every reader in one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceManifest {
    pub session_id: Uuid,
    pub session_start_utc_ms: i64,
    pub duration_ms: u64,
    pub sources: Vec<RecordingSource>,
    pub frames: Vec<CandidateFrame>,
    /// Complete camera sets; a model request must not split a set.
    pub frame_sets: Vec<Vec<String>>,
    pub warnings: Vec<AnalysisWarning>,
    pub eligible_periods: Vec<EligiblePeriod>,
}

/// Recorded camera participation and cadence, retained to constrain any additional sampling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EligiblePeriod {
    pub camera_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub sample_every_ms: u64,
}

/// A reviewer may request a bounded interval on a known camera, never a file path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceInterval {
    pub camera_id: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// A time-bounded request window whose candidate images plus references fit the model limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceWindow {
    pub frame_ids: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Identity of the actual JPEG bytes sent for a candidate sample after requested resizing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SentImage {
    pub candidate_id: String,
    pub sha256: String,
    pub size_bytes: u64,
}

impl EvidenceManifest {
    /// Resolves a citation only if it names a candidate frame from this frozen plan.
    pub fn frame(&self, id: &str) -> Option<&CandidateFrame> {
        self.frames.iter().find(|frame| frame.id == id)
    }

    /// Resolve a registered frame only when its camera and both timeline offsets match its source.
    pub fn source_for(&self, frame: &CandidateFrame) -> Result<&RecordingSource, EvidenceError> {
        if self.frame(&frame.id) != Some(frame) {
            return Err(EvidenceError::Invalid("Unknown candidate frame".into()));
        }
        let source = self
            .sources
            .get(frame.source_index)
            .ok_or_else(|| EvidenceError::Invalid("Unknown recording segment".into()))?;
        let segment_length = source
            .end_utc_ms
            .checked_sub(source.start_utc_ms)
            .and_then(|length| u64::try_from(length).ok())
            .filter(|length| *length > 0)
            .ok_or_else(|| EvidenceError::Invalid("Invalid segment time".into()))?;
        let expected_offset = i64::try_from(frame.recording_offset_ms)
            .ok()
            .and_then(|offset| source.start_utc_ms.checked_add(offset))
            .and_then(|utc| utc.checked_sub(self.session_start_utc_ms))
            .and_then(|offset| u64::try_from(offset).ok());
        if source.camera_id != frame.camera_id
            || frame.recording_offset_ms >= segment_length
            || frame.session_offset_ms >= self.duration_ms
            || expected_offset != Some(frame.session_offset_ms)
        {
            return Err(EvidenceError::Invalid(
                "Candidate frame is outside its segment or session timeline".into(),
            ));
        }
        Ok(source)
    }
}
