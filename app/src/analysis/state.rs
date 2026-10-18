use std::{collections::BTreeMap, path::PathBuf};

use backend::analysis::{HumanCorrection, Judgment, PlaybackClip, RunHistory, RunSnapshot};
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
    /// Changes when a successful save consumes the reason; absent before the first reset.
    pub reason_edit_id: Option<Uuid>,
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
            reason_edit_id: None,
            evidence_ids: Vec::new(),
            outside_information: false,
            saving: false,
            error: None,
        }
    }
}

impl CorrectionDraft {
    /// Reject input from a reason field replaced after its contents were saved.
    pub fn set_reason(&mut self, edit_id: Option<Uuid>, reason: String) {
        if self.reason_edit_id == edit_id {
            self.reason = reason;
            self.error = None;
        }
    }

    /// Clear a saved reason only when the operator has not changed the submitted draft.
    pub fn finish_save(&mut self, submitted: &Self) {
        if self.reason == submitted.reason
            && self.judgment == submitted.judgment
            && self.evidence_ids == submitted.evidence_ids
            && self.outside_information == submitted.outside_information
        {
            self.reason.clear();
            self.reason_edit_id = Some(Uuid::new_v4());
        }
        self.saving = false;
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
    /// False when a history refresh found this cached run missing or unreadable on disk.
    /// Its unsaved review drafts remain in memory in case the run is restored.
    pub available: bool,
    /// Progress generation captured before reading history on a worker.
    pub snapshot_generation: u64,
    pub selected_criterion: Option<Uuid>,
    pub playback: PlaybackState,
    pub reference_previews: BTreeMap<Uuid, String>,
    pub references_loading: bool,
    pub reference_error: Option<String>,
    pub corrections: BTreeMap<Uuid, CorrectionDraft>,
    pub assessment_draft: AssessmentDraft,
}
impl RunEntry {
    /// Human corrections and review decisions share one run-local storage writer.
    pub fn review_saving(&self) -> bool {
        self.assessment_draft.saving || self.corrections.values().any(|draft| draft.saving)
    }

    /// Restore a current saved decision when first opening a run; later updates retain its draft.
    pub fn new(snapshot: RunSnapshot) -> Self {
        let assessment_draft = snapshot
            .current_student_assessment()
            .map(|saved| AssessmentDraft {
                outcome: saved.outcome,
                note: saved.note.clone(),
                ..Default::default()
            })
            .unwrap_or_default();
        Self {
            selected_criterion: snapshot
                .inputs
                .exercise
                .checklist
                .criteria
                .first()
                .map(|item| item.id),
            snapshot,
            available: true,
            snapshot_generation: 0,
            playback: PlaybackState::default(),
            reference_previews: BTreeMap::new(),
            references_loading: false,
            reference_error: None,
            corrections: BTreeMap::new(),
            assessment_draft,
        }
    }

    /// An unchanged decision is already saved only while its reviewed results are current.
    pub fn assessment_draft_is_saved(&self) -> bool {
        self.snapshot
            .current_student_assessment()
            .is_some_and(|saved| {
                saved.outcome == self.assessment_draft.outcome
                    && saved.note == self.assessment_draft.note
            })
    }
}

/// Unsaved overall assessment belongs to one run and never carries into the next student.
#[derive(Clone)]
pub struct AssessmentDraft {
    pub outcome: backend::analysis::StudentOutcome,
    pub note: String,
    pub saving: bool,
    pub error: Option<String>,
}
impl Default for AssessmentDraft {
    fn default() -> Self {
        Self {
            outcome: backend::analysis::StudentOutcome::NeedsEvidence,
            note: String::new(),
            saving: false,
            error: None,
        }
    }
}

/// One recording's independent run history. Loading another recording does not change this selection.
#[derive(Default)]
pub struct SessionAnalyses {
    pub directory: PathBuf,
    pub runs: BTreeMap<Uuid, RunEntry>,
    pub archived: Vec<backend::analysis::ArchivedRun>,
    pub selected_run_id: Option<Uuid>,
    pub loaded: bool,
    pub loading: bool,
    /// Identifies the worker allowed to publish this directory's history.
    pub history_load_id: Option<Uuid>,
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
    pub model_profile_id: Option<String>,
    pub reference_images: bool,
    pub automatic_recheck: bool,
}
impl AnalysisPageState {
    /// Keep a recording bound to its current folder while model or review writes still own it.
    pub fn session_work_in_progress(&self, session_id: Uuid) -> bool {
        self.active
            .is_some_and(|active| active.session_id == session_id)
            || self
                .sessions
                .get(&session_id)
                .is_some_and(|session| session.runs.values().any(RunEntry::review_saving))
    }

    /// Claim a history read, invalidating old folder results while preserving unsaved drafts.
    pub fn begin_history_load(
        &mut self,
        session_id: Uuid,
        directory: PathBuf,
        refresh: bool,
    ) -> Option<(Uuid, BTreeMap<Uuid, u64>)> {
        let busy = self.session_work_in_progress(session_id);
        let session = self.sessions.entry(session_id).or_default();
        let changed_directory = session.directory != directory;
        if (changed_directory && busy)
            || (!changed_directory && (session.loading || (session.loaded && !refresh)))
        {
            return None;
        }
        if changed_directory {
            session.loaded = false;
            session.archived.clear();
            for entry in session.runs.values_mut() {
                entry.available = false;
                entry.playback = PlaybackState::default();
                entry.reference_previews.clear();
                entry.references_loading = false;
                entry.reference_error = None;
            }
        }
        let id = Uuid::new_v4();
        session.directory = directory;
        session.history_load_id = Some(id);
        session.loading = true;
        session.error = None;
        let generations = session
            .runs
            .iter()
            .map(|(id, entry)| (*id, entry.snapshot_generation))
            .collect();
        Some((id, generations))
    }

    /// Ignore an older directory's response after a replacement read has taken ownership.
    pub fn finish_history_load(
        &mut self,
        session_id: Uuid,
        id: Uuid,
        history: Result<RunHistory, String>,
        generations: &BTreeMap<Uuid, u64>,
    ) {
        let Some(session) = self.sessions.get_mut(&session_id) else {
            return;
        };
        if session.history_load_id != Some(id) {
            return;
        }
        session.history_load_id = None;
        self.apply_history(session_id, history, generations);
    }

    /// Apply a history read without hiding newer live progress or discarding review drafts.
    pub fn apply_history(
        &mut self,
        session_id: Uuid,
        history: Result<RunHistory, String>,
        expected_generations: &BTreeMap<Uuid, u64>,
    ) {
        let (mut history, error) = match history {
            Ok(history) => (history, None),
            Err(error) => (RunHistory::default(), Some(error)),
        };
        // A foreign history response must never update another recording's cached review.
        history
            .runs
            .retain(|run| run.inputs.initial_evidence.session_id == session_id);
        let discovered = history
            .runs
            .iter()
            .map(|run| run.inputs.id)
            .collect::<std::collections::HashSet<_>>();
        for run in history.runs {
            let generation = expected_generations.get(&run.inputs.id).copied();
            self.apply_history_snapshot(run, generation);
        }
        let active_run = self
            .active
            .filter(|active| active.session_id == session_id)
            .and_then(|active| active.run_id);
        let session = self.sessions.entry(session_id).or_default();
        for (id, entry) in &mut session.runs {
            if !discovered.contains(id)
                && active_run != Some(*id)
                && expected_generations.get(id) == Some(&entry.snapshot_generation)
            {
                entry.available = false;
            }
        }
        if session
            .selected_run_id
            .is_none_or(|id| session.runs.get(&id).is_none_or(|entry| !entry.available))
        {
            session.selected_run_id = session
                .runs
                .values()
                .filter(|entry| entry.available)
                .max_by_key(|entry| {
                    (
                        entry.snapshot.inputs.created_utc_ms,
                        entry.snapshot.inputs.id,
                    )
                })
                .map(|entry| entry.snapshot.inputs.id);
        }
        if error.is_none() {
            session.archived = history.archived;
            session.loaded = true;
        }
        session.error = error;
        session.loading = false;
    }

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
            let session = self.sessions.entry(session_id).or_default();
            if let Some(entry) = session.runs.get_mut(&run_id) {
                // Disk history is authoritative when no newer callback arrived during the read.
                // Keep editor drafts, but never import another folder's saved review history.
                entry.snapshot = snapshot;
                entry.available = true;
                entry.snapshot_generation = entry.snapshot_generation.wrapping_add(1);
            } else {
                session.runs.insert(run_id, RunEntry::new(snapshot));
            }
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
                    entry.snapshot_generation = entry.snapshot_generation.wrapping_add(1);
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
            for mark in &entry.snapshot.operator_review.marks {
                if !updated.operator_review.marks.contains(mark) {
                    updated.operator_review.marks.push(mark.clone());
                }
            }
            for assessment in &entry.snapshot.operator_review.assessments {
                if !updated.operator_review.assessments.contains(assessment) {
                    updated.operator_review.assessments.push(assessment.clone());
                }
            }
            entry.snapshot = updated;
            entry.available = true;
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
