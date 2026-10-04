use std::collections::BTreeMap;

use backend::exercise::{
    ChecklistProposal, Criterion, ExerciseDraft, ExerciseRevision, ExerciseStore, ReferenceAsset,
};
use dioxus::prelude::*;
use uuid::Uuid;

/// Application-root exercise state. Async tasks retain this context when the operator navigates.
#[derive(Clone)]
pub struct ExerciseContext {
    pub state: Signal<ExercisePageState>,
    pub store: ExerciseStore,
}

/// Current edits and the last durable versions of one configuration.
#[derive(Clone)]
pub struct ExerciseEditorState {
    pub draft: ExerciseDraft,
    pub saved: ExerciseDraft,
    pub published: Option<ExerciseRevision>,
    pub previews: BTreeMap<Uuid, String>,
    pub pasted_text: String,
    pub pending_delete: Option<Uuid>,
    pub saving: bool,
    pub importing: bool,
    pub formatting: bool,
    pub error: Option<String>,
}

impl ExerciseEditorState {
    pub fn new(draft: ExerciseDraft, published: Option<ExerciseRevision>) -> Self {
        Self {
            saved: draft.clone(),
            draft,
            published,
            previews: BTreeMap::new(),
            pasted_text: String::new(),
            pending_delete: None,
            saving: false,
            importing: false,
            formatting: false,
            error: None,
        }
    }

    /// Apply a completed save while preserving any edits made after the request began.
    pub fn finish_save(
        &mut self,
        submitted: &ExerciseDraft,
        saved: ExerciseDraft,
        published: Option<ExerciseRevision>,
    ) {
        if self.draft == *submitted {
            self.draft = saved.clone();
        } else {
            self.draft.edit_version = saved.edit_version;
        }
        self.saved = saved;
        if let Some(revision) = published {
            self.published = Some(revision);
        }
        self.saving = false;
    }

    /// Explicitly add operator-created final steps to preparation without replacing earlier source.
    pub fn include_new_source_steps(&mut self) {
        if let Some(source) = &mut self.draft.preparation.source {
            for item in &self.draft.checklist.criteria {
                if !source
                    .criteria
                    .iter()
                    .any(|original| original.id == item.id)
                {
                    source.criteria.push(item.clone());
                }
            }
        }
    }

    /// Retain source and questions without changing the authoritative final wording.
    pub fn receive_proposal(&mut self, proposal: ChecklistProposal) -> Result<(), String> {
        proposal
            .validate_against(&self.draft)
            .map_err(|error| error.to_string())?;
        let sources = self.draft.source_checklist().criteria.clone();
        let mut questions = self.draft.preparation.questions.clone();
        for comment in &proposal.comments {
            let Some(item) = comment.proposed_item.and_then(|index| sources.get(index)) else {
                continue;
            };
            if !questions.iter().any(|question| {
                question.criterion_id == item.id
                    && question.question == comment.text
                    && question.source_text == item.text
            }) {
                questions.push(backend::exercise::Clarification {
                    criterion_id: item.id,
                    source_text: item.text.clone(),
                    question: comment.text.clone(),
                    answer: String::new(),
                });
            }
        }
        self.draft.preparation.questions = questions;
        if self.draft.preparation.reviewed_basis.is_none() {
            self.draft
                .preparation
                .reconcile()
                .map_err(|error| error.to_string())?;
        }
        let review =
            ChecklistProposal::for_draft(&self.draft, proposal.criteria, proposal.comments)
                .map_err(|error| error.to_string())?;
        self.draft.preparation.proposal = Some(review);
        Ok(())
    }

    /// Apply edited suggestions while retaining each exactly mapped source criterion's identity.
    /// Questions and the exact proposal are retained separately from checklist requirements.
    pub fn apply_proposal(&mut self) -> Result<(), String> {
        let proposal = self
            .draft
            .preparation
            .proposal
            .as_ref()
            .ok_or("There is no proposal to apply.")?;
        proposal
            .validate_against(&self.draft)
            .map_err(|error| error.to_string())?;
        let mut offset = 0;
        let mut sources = BTreeMap::new();
        for criterion in &self.draft.source_checklist().criteria {
            sources.insert((offset, offset + criterion.text.len()), criterion.id);
            offset += criterion.text.len() + 2;
        }
        let mut criteria = Vec::new();
        for item in &proposal.criteria {
            let [source] = item.source_ranges.as_slice() else {
                return Err("This proposal cannot preserve the existing step links.".into());
            };
            let id = sources
                .remove(&(source.start, source.end))
                .ok_or("This proposal does not match the original checklist steps.")?;
            criteria.push(Criterion {
                id,
                text: item.text.clone(),
            });
        }
        if !sources.is_empty() {
            return Err("This proposal omits an original checklist step.".into());
        }
        // Suggestions change wording only. Operator additions, removals and ordering survive.
        for item in &mut self.draft.checklist.criteria {
            if let Some(proposed) = criteria.iter().find(|proposed| proposed.id == item.id) {
                item.text = proposed.text.clone();
            }
        }
        self.draft.checklist.format_version = backend::exercise::CHECKLIST_FORMAT_VERSION.into();
        self.draft.preparation.last_review = self.draft.preparation.proposal.take();
        self.draft
            .preparation
            .reconcile()
            .map_err(|error| error.to_string())?;
        self.error = None;
        Ok(())
    }
}

/// Keeps edits for every open configuration; changing the selected item cannot discard a draft.
#[derive(Default)]
pub struct ExercisePageState {
    pub configurations: BTreeMap<Uuid, ExerciseEditorState>,
    pub selected_id: Option<Uuid>,
    pub loading: bool,
    pub creating: bool,
    pub error: Option<String>,
    pub formatter_model_id: Option<String>,
}

impl ExercisePageState {
    /// Return a publication only when it represents the selected editor's current saved content.
    pub fn selected_revision(&self) -> Option<ExerciseRevision> {
        let editor = self.configurations.get(&self.selected_id?)?;
        let published = editor.published.as_ref()?;
        (editor.draft == editor.saved && published.edit_version == editor.saved.edit_version)
            .then(|| published.clone())
    }

    /// Apply an operator edit to its explicit configuration, never a later selected item.
    pub fn edit(&mut self, id: Uuid, update: impl FnOnce(&mut ExerciseDraft)) {
        if let Some(editor) = self.configurations.get_mut(&id) {
            update(&mut editor.draft);
            editor.error = None;
        }
    }

    /// Explicit step deletion removes only links to that stable criterion ID.
    pub fn delete_criterion(&mut self, exercise_id: Uuid, criterion_id: Uuid) {
        self.edit(exercise_id, |draft| {
            draft
                .checklist
                .criteria
                .retain(|item| item.id != criterion_id);
            for reference in &mut draft.references {
                reference.criterion_ids.retain(|id| *id != criterion_id);
            }
        });
        if let Some(editor) = self.configurations.get_mut(&exercise_id) {
            editor.pending_delete = None;
        }
    }

    /// An import completes against its starting exercise while retaining concurrent text edits.
    pub fn finish_import(&mut self, exercise_id: Uuid, reference: ReferenceAsset, preview: String) {
        if let Some(editor) = self.configurations.get_mut(&exercise_id) {
            editor.previews.insert(reference.image.id, preview);
            editor.draft.references.push(reference);
            editor.importing = false;
        }
    }
}

/// Install the editor above routing, then load configurations without blocking the desktop UI.
pub fn use_exercise_context(store: ExerciseStore) -> ExerciseContext {
    let state = use_hook(|| Signal::new_in_scope(ExercisePageState::default(), ScopeId::ROOT));
    let context = ExerciseContext { state, store };
    use_context_provider(|| context.clone());
    let loader = context.clone();
    use_hook(move || super::tasks::load(loader));
    context
}
