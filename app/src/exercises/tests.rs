use backend::exercise::{
    Checklist, Criterion, ExerciseDraft, ExerciseRevision, ImageAsset, ImageMediaType,
    ReferenceAsset,
};
use uuid::Uuid;

use super::state::{ExerciseEditorState, ExercisePageState};

fn draft(name: &str) -> ExerciseDraft {
    ExerciseDraft {
        schema_version: 1,
        id: Uuid::new_v4(),
        name: name.into(),
        edit_version: 1,
        checklist: Checklist {
            format_version: "operator-text-v1".into(),
            criteria: vec![Criterion {
                id: Uuid::new_v4(),
                text: "Place the item in the case.".into(),
            }],
        },
        references: Vec::new(),
    }
}

fn publication(draft: &ExerciseDraft) -> ExerciseRevision {
    ExerciseRevision {
        schema_version: 1,
        id: Uuid::new_v4(),
        exercise_id: draft.id,
        edit_version: draft.edit_version,
        name: draft.name.clone(),
        checklist: draft.checklist.clone(),
        references: draft.references.clone(),
    }
}

fn reference(criterion_id: Uuid) -> ReferenceAsset {
    ReferenceAsset {
        image: ImageAsset {
            id: Uuid::new_v4(),
            sha256: "fixture".into(),
            byte_length: 0,
            media_type: ImageMediaType::Png,
            width: 2,
            height: 3,
            orientation: 1,
        },
        object_label: "Case".into(),
        state_label: "Open".into(),
        purpose: "Identify case".into(),
        criterion_ids: vec![criterion_id],
    }
}

#[test]
fn save_completion_preserves_newer_editor_typing() {
    let submitted = draft("Original");
    let mut editor = ExerciseEditorState::new(submitted.clone(), None);
    editor.draft.name = "Typed while saving".into();
    let mut saved = submitted.clone();
    saved.edit_version += 1;
    editor.finish_save(&submitted, saved.clone(), Some(publication(&saved)));
    assert_eq!(editor.draft.name, "Typed while saving");
    assert_eq!(editor.draft.edit_version, saved.edit_version);
    assert_eq!(editor.saved.name, "Original");
}

#[test]
fn unsaved_changes_cannot_be_used_as_published_analysis_inputs() {
    let draft = draft("Example");
    let id = draft.id;
    let mut page = ExercisePageState {
        selected_id: Some(id),
        ..Default::default()
    };
    page.configurations.insert(
        id,
        ExerciseEditorState::new(draft.clone(), Some(publication(&draft))),
    );
    assert!(page.selected_revision().is_some());
    page.edit(id, |draft| {
        draft.checklist.criteria[0].text = "New instruction".into()
    });
    assert!(page.selected_revision().is_none());
}

#[test]
fn import_completion_targets_its_original_exercise_after_selection_changes() {
    let original = draft("Original");
    let selected = draft("Selected later");
    let original_id = original.id;
    let selected_id = selected.id;
    let reference = reference(original.checklist.criteria[0].id);
    let mut page = ExercisePageState {
        selected_id: Some(selected_id),
        ..Default::default()
    };
    page.configurations
        .insert(original_id, ExerciseEditorState::new(original, None));
    page.configurations
        .insert(selected_id, ExerciseEditorState::new(selected, None));
    page.finish_import(original_id, reference, "fixture-preview".into());
    assert_eq!(page.configurations[&original_id].draft.references.len(), 1);
    assert!(
        page.configurations[&selected_id]
            .draft
            .references
            .is_empty()
    );
    assert_eq!(page.selected_id, Some(selected_id));
}

#[test]
fn explicit_step_deletion_clears_only_its_reference_links() {
    let mut draft = draft("Example");
    let first = draft.checklist.criteria[0].id;
    let second = Uuid::new_v4();
    draft.checklist.criteria.push(Criterion {
        id: second,
        text: "Close case".into(),
    });
    let mut reference = reference(first);
    reference.criterion_ids.push(second);
    draft.references.push(reference);
    let id = draft.id;
    let mut page = ExercisePageState::default();
    page.configurations
        .insert(id, ExerciseEditorState::new(draft, None));
    page.delete_criterion(id, first);
    let updated = &page.configurations[&id].draft;
    assert_eq!(updated.references[0].criterion_ids, vec![second]);
    assert_eq!(updated.checklist.criteria[0].id, second);
}

#[test]
fn applying_edited_proposal_keeps_reference_links_and_comments_separate() {
    use backend::exercise::{
        ChecklistProposal, ProposalComment, ProposalCommentKind, ProposedCriterion, SourceRange,
    };
    let mut draft = draft("Example");
    let criterion_id = draft.checklist.criteria[0].id;
    draft.references.push(reference(criterion_id));
    let range = SourceRange {
        start: 0,
        end: draft.checklist.source_text().len(),
    };
    let proposal = ChecklistProposal::for_draft(
        &draft,
        vec![ProposedCriterion {
            text: "Put the item inside the storage case.".into(),
            source_ranges: vec![range.clone()],
        }],
        vec![ProposalComment {
            kind: ProposalCommentKind::Ambiguity,
            text: "Should the case be closed?".into(),
            source_ranges: vec![range],
            proposed_item: Some(0),
        }],
    )
    .unwrap();
    let mut editor = ExerciseEditorState::new(draft, None);
    editor.proposal = Some(proposal);
    editor.apply_proposal().unwrap();
    assert_eq!(editor.draft.checklist.criteria[0].id, criterion_id);
    assert_eq!(editor.draft.references[0].criterion_ids, vec![criterion_id]);
    assert!(!editor.draft.checklist.source_text().contains("closed"));
    assert_eq!(editor.review_notes.unwrap().comments.len(), 1);
}
