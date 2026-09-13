use std::{collections::BTreeMap, path::PathBuf};

use backend::analysis::{HumanCorrection, Judgment, PlaybackClip, RunSnapshot};
use dioxus::prelude::*;
use uuid::Uuid;

/// Navigation state is kept above the router, together with active work and unsaved corrections.
#[derive(Clone, Copy)]
pub struct AnalysisContext {
    pub state: Signal<AnalysisPageState>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AnalysisTab {
    #[default]
    Configure,
    Evaluate,
}

/// Editable correction, separate from the model's original assessment.
#[derive(Clone)]
pub struct CorrectionDraft {
    pub judgment: Judgment,
    pub reason: String,
    pub evidence_ids: Vec<String>,
    pub outside_information: bool,
    pub saving: bool,
    pub error: Option<String>,
}
impl Default for CorrectionDraft {
    fn default() -> Self {
        Self {
            judgment: Judgment::CannotVerify,
            reason: String::new(),
            evidence_ids: Vec::new(),
            outside_information: false,
            saving: false,
            error: None,
        }
    }
}

/// Current playable citation and its request token, preventing a slow old clip from replacing a new one.
#[derive(Clone, Default)]
pub struct PlaybackState {
    pub frame_id: Option<String>,
    pub request_id: Option<Uuid>,
    pub clip: Option<PlaybackClip>,
    pub url: Option<String>,
    pub before_ms: u64,
    pub after_ms: u64,
    pub loading: bool,
    pub error: Option<String>,
}

/// The saved run plus navigation and correction state that survive progress callbacks.
#[derive(Clone)]
pub struct RunEntry {
    pub snapshot: RunSnapshot,
    /// Progress generation captured before reading history on a worker.
    pub snapshot_generation: u64,
    pub selected_criterion: Option<Uuid>,
    pub playback: PlaybackState,
    pub reference_previews: BTreeMap<Uuid, String>,
    pub references_loading: bool,
    pub reference_error: Option<String>,
    pub corrections: BTreeMap<Uuid, CorrectionDraft>,
}
impl RunEntry {
    pub fn new(snapshot: RunSnapshot) -> Self {
        Self {
            selected_criterion: snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .first()
                .map(|item| item.id),
            snapshot,
            snapshot_generation: 0,
            playback: PlaybackState::default(),
            reference_previews: BTreeMap::new(),
            references_loading: false,
            reference_error: None,
            corrections: BTreeMap::new(),
        }
    }
}

/// One recording's independent run history. Loading another recording does not change this selection.
#[derive(Default)]
pub struct SessionAnalyses {
    pub directory: PathBuf,
    pub runs: BTreeMap<Uuid, RunEntry>,
    pub selected_run_id: Option<Uuid>,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ActiveRun {
    /// Distinguishes preparations for the same recording before a saved run ID exists.
    pub token: Uuid,
    pub session_id: Uuid,
    pub run_id: Option<Uuid>,
}

/// Analysis owns only evaluation work; recording/session discovery remain in OperatorState.
#[derive(Default)]
pub struct AnalysisPageState {
    pub tab: AnalysisTab,
    pub sessions: BTreeMap<Uuid, SessionAnalyses>,
    pub active: Option<ActiveRun>,
    pub model_set_id: Option<String>,
}
impl AnalysisPageState {
    /// Use a history read only when this run has not changed since the read was requested.
    pub fn apply_history_snapshot(
        &mut self,
        snapshot: RunSnapshot,
        expected_generation: Option<u64>,
    ) {
        let session_id = snapshot.inputs.initial_evidence.session_id;
        let run_id = snapshot.inputs.id;
        let current_generation = self
            .sessions
            .get(&session_id)
            .and_then(|session| session.runs.get(&run_id))
            .map(|entry| entry.snapshot_generation);
        if current_generation == expected_generation {
            self.apply_snapshot(snapshot);
        } else {
            self.apply_corrections(session_id, run_id, snapshot.corrections);
        }
    }

    /// A correction save refreshes only human history, so it cannot roll back active run progress.
    pub fn apply_corrections(
        &mut self,
        session_id: Uuid,
        run_id: Uuid,
        corrections: Vec<HumanCorrection>,
    ) {
        if let Some(entry) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.runs.get_mut(&run_id))
        {
            for correction in corrections {
                if !entry
                    .snapshot
                    .corrections
                    .iter()
                    .any(|item| item.id == correction.id)
                {
                    entry.snapshot.corrections.push(correction);
                }
            }
        }
    }

    /// Edit one criterion's human review draft without changing its saved machine assessment.
    pub fn edit_correction(
        &mut self,
        session_id: Uuid,
        run_id: Uuid,
        criterion_id: Uuid,
        update: impl FnOnce(&mut CorrectionDraft),
    ) {
        if let Some(entry) = self
            .sessions
            .get_mut(&session_id)
            .and_then(|session| session.runs.get_mut(&run_id))
        {
            let draft = entry.corrections.entry(criterion_id).or_default();
            update(draft);
            draft.error = None;
        }
    }
    /// Merge a durable callback by its own identity, preserving review selections and manual edits.
    pub fn apply_snapshot(&mut self, snapshot: RunSnapshot) {
        let session_id = snapshot.inputs.initial_evidence.session_id;
        let session = self.sessions.entry(session_id).or_default();
        let id = snapshot.inputs.id;
        if let Some(entry) = session.runs.get_mut(&id) {
            let mut updated = snapshot;
            // A concurrent saved correction can be newer than an in-flight execution snapshot.
            for correction in &entry.snapshot.corrections {
                if !updated
                    .corrections
                    .iter()
                    .any(|item| item.id == correction.id)
                {
                    updated.corrections.push(correction.clone());
                }
            }
            entry.snapshot = updated;
            entry.snapshot_generation = entry.snapshot_generation.wrapping_add(1);
        } else {
            session.runs.insert(id, RunEntry::new(snapshot));
        }
    }
}

/// Install analysis state once in the application root so routing cannot cancel a run.
pub fn use_analysis_context() {
    let state = use_hook(|| Signal::new_in_scope(AnalysisPageState::default(), ScopeId::ROOT));
    use_context_provider(|| AnalysisContext { state });
}
