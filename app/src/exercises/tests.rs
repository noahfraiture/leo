use backend::exercise::{
    Checklist, Criterion, ExerciseDraft, ExerciseRevision, ImageAsset, ImageMediaType,
    ReferenceAsset,
};
use uuid::Uuid;

use super::state::{ExerciseEditorState, ExercisePageState};

#[cfg(unix)]
#[test]
fn incomplete_reference_metadata_explains_why_configuration_cannot_be_saved() {
    use crate::test_support::{RenderHarness, assert_button_disabled, ready_preview};
    for missing_object in [true, false] {
        let mut draft = draft("Reference validation");
        let mut image = reference(draft.checklist.criteria[0].id);
        if missing_object {
            image.object_label.clear();
        } else {
            image.purpose.clear();
        }
        draft.references.push(image);
        let mut harness = RenderHarness::new();
        harness.exercises.selected_id = Some(draft.id);
        harness
            .exercises
            .configurations
            .insert(draft.id, ExerciseEditorState::new(draft, None));
        let html = harness.render_at(ready_preview(), "/analyze");
        assert_button_disabled(&html, "Confirm configuration", true);
        assert!(
            html.contains("each reference needs an object label, purpose, and criterion link",),
        );
        assert!(html.contains("aria-describedby=\"exercise-publication-error\""));
    }
}

#[cfg(unix)]
#[test]
fn unavailable_exercise_folders_remain_visible_alongside_healthy_editors() {
    use crate::test_support::{RenderHarness, assert_button_disabled, ready_preview};
    let draft = draft("Healthy configuration remains usable");
    let mut harness = RenderHarness::new();
    harness.exercises.selected_id = Some(draft.id);
    harness
        .exercises
        .configurations
        .insert(draft.id, ExerciseEditorState::new(draft, None));
    harness
        .exercises
        .unavailable
        .push(backend::exercise::UnavailableExercise {
            exercise_id: Uuid::new_v4(),
            directory: "/retained-exercise-folder".into(),
            source: backend::exercise::Error::Invalid("Damaged saved draft".into()),
        });

    let html = harness.render_at(ready_preview(), "/analyze");

    assert!(html.contains("Healthy configuration remains usable"));
    assert!(html.contains("Configurations needing repair"));
    assert!(html.contains("Damaged saved draft"));
    assert!(html.contains("/retained-exercise-folder"));
    assert_button_disabled(&html, "New configuration", false);
}

#[test]
fn ordinary_edits_do_not_clear_a_publication_storage_warning() {
    let draft = draft("Publication needs repair");
    let id = draft.id;
    let mut editor = ExerciseEditorState::new(draft, None);
    editor.storage_error = Some("Saved publication needs repair".into());
    let mut page = ExercisePageState {
        configurations: [(id, editor)].into(),
        ..Default::default()
    };

    page.edit(id, |draft| draft.name = "Edited draft".into());

    assert_eq!(
        page.configurations[&id].storage_error.as_deref(),
        Some("Saved publication needs repair")
    );
}

fn question_editor() -> ExerciseEditorState {
    let mut draft = draft("Question identities");
    let criterion = &draft.checklist.criteria[0];
    draft.preparation.questions = ["First answer", "Second answer"]
        .into_iter()
        .map(|answer| backend::exercise::Clarification {
            criterion_id: criterion.id,
            question: "Which case?".into(),
            source_text: criterion.text.clone(),
            answer: answer.into(),
        })
        .collect();
    ExerciseEditorState::new(draft, None)
}

#[test]
fn late_question_events_cannot_target_a_neighbor_after_removal() {
    let mut editor = question_editor();
    let removed = editor.question_ids[0];
    let remaining = editor.question_ids[1];
    assert_ne!(removed, remaining);

    editor.remove_question(removed);
    editor.remove_question(removed);
    editor.set_question_answer(removed, "Late input for removed question".into());
    assert_eq!(editor.draft.preparation.questions.len(), 1);
    assert_eq!(
        editor.draft.preparation.questions[0].answer,
        "Second answer"
    );
    editor.set_question_answer(remaining, "Updated remaining answer".into());
    assert_eq!(
        editor.draft.preparation.questions[0].answer,
        "Updated remaining answer"
    );
    assert_eq!(editor.question_ids, vec![remaining]);
}

#[test]
fn source_confirmation_targets_only_the_requested_question_row() {
    let mut editor = question_editor();
    let first = editor.question_ids[0];
    let second = editor.question_ids[1];
    let old_source = editor.draft.preparation.questions[0].source_text.clone();
    editor.draft.checklist.criteria[0].text = "New original wording".into();

    editor.confirm_question_source(second);

    assert_eq!(
        editor.draft.preparation.questions[0].source_text,
        old_source
    );
    assert_eq!(
        editor.draft.preparation.questions[1].source_text,
        "New original wording"
    );
    assert_eq!(editor.question_ids, vec![first, second]);
}

#[test]
fn saving_an_earlier_draft_does_not_restore_removed_question_identity() {
    let mut editor = question_editor();
    let submitted = editor.draft.clone();
    let removed = editor.question_ids[0];
    let remaining = editor.question_ids[1];
    editor.remove_question(removed);
    let mut saved = submitted.clone();
    saved.edit_version += 1;

    editor.finish_save(&submitted, saved, None);
    editor.set_question_answer(remaining, "Typing after save".into());

    assert_eq!(editor.question_ids, vec![remaining]);
    assert_eq!(editor.draft.preparation.questions.len(), 1);
    assert_eq!(editor.saved.preparation.questions.len(), 2);
    assert_eq!(
        editor.draft.preparation.questions[0].answer,
        "Typing after save"
    );
}

#[test]
fn assistance_additions_keep_existing_question_identity_and_allocate_each_new_row_once() {
    use backend::exercise::{
        ChecklistProposal, ProposalComment, ProposalCommentKind, ProposedCriterion, SourceRange,
    };
    let mut editor = question_editor();
    let original_ids = editor.question_ids.clone();
    for _ in 0..2 {
        let range = SourceRange {
            start: 0,
            end: editor.draft.source_checklist().source_text().len(),
        };
        let proposal = ChecklistProposal::for_draft(
            &editor.draft,
            vec![ProposedCriterion {
                text: editor.draft.checklist.criteria[0].text.clone(),
                source_ranges: vec![range.clone()],
            }],
            vec![ProposalComment {
                kind: ProposalCommentKind::Ambiguity,
                text: "Must the case be closed?".into(),
                source_ranges: vec![range],
                proposed_item: Some(0),
            }],
        )
        .unwrap();
        editor.receive_proposal(proposal).unwrap();
    }

    assert_eq!(&editor.question_ids[..2], original_ids.as_slice());
    assert_eq!(editor.question_ids.len(), 3);
    assert_eq!(editor.draft.preparation.questions.len(), 3);
    assert!(!original_ids.contains(&editor.question_ids[2]));
}

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
        preparation: Default::default(),
    }
}

#[cfg(unix)]
fn render_checklist_review(
    draft: ExerciseDraft,
    selected_model: Option<String>,
    workflow: crate::settings::WorkflowSettings,
) -> String {
    use dioxus::{dioxus_core::NoOpMutations, prelude::*};

    fn root(
        (draft, selected_model, workflow, store): (
            ExerciseDraft,
            Option<String>,
            crate::settings::WorkflowSettings,
            crate::settings::SettingsStore,
        ),
    ) -> Element {
        let exercise_id = draft.id;
        let state = use_signal(move || ExercisePageState {
            selected_id: Some(exercise_id),
            formatter_model_id: selected_model,
            configurations: [(exercise_id, ExerciseEditorState::new(draft, None))].into(),
            ..Default::default()
        });
        let settings = store
            .resolve(crate::settings::Settings {
                workflow,
                ..Default::default()
            })
            .unwrap();
        use_context_provider(move || settings);
        use_context_provider(move || super::state::ExerciseContext {
            state,
            store: backend::exercise::ExerciseStore::new(store.default_data_root.join("exercises")),
        });
        crate::inference::use_inference_gate();
        rsx! { super::formatting::ChecklistAssistance { exercise_id } }
    }

    let temporary = tempfile::tempdir().unwrap();
    let store = crate::settings::SettingsStore::new(
        temporary.path().join("settings.json"),
        temporary.path().join("data"),
    );
    let mut dom = VirtualDom::new_with_props(root, (draft, selected_model, workflow, store));
    dom.rebuild(&mut NoOpMutations);
    dioxus_ssr::render(&dom)
}

#[cfg(unix)]
#[test]
fn checklist_review_readiness_follows_its_original_source_wording() {
    use crate::test_support::opening_tag_with_marker;
    let mut disabled = Vec::new();
    for empty_source in [true, false] {
        let mut draft = draft("Review source readiness");
        draft.preparation.source = Some(draft.checklist.clone());
        if empty_source {
            draft.preparation.source.as_mut().unwrap().criteria[0]
                .text
                .clear();
        } else {
            draft.checklist.criteria[0].text.clear();
        }
        let workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
        let html = render_checklist_review(draft, None, workflow);
        disabled.push(
            opening_tag_with_marker(&html, "button", r#"id="exercise-format""#)
                .contains("disabled"),
        );
    }
    assert_eq!(disabled, [true, false]);
}

#[cfg(unix)]
#[test]
fn checklist_review_explains_an_unusable_selected_model_before_starting() {
    use crate::test_support::opening_tag_with_marker;

    for missing_model in [true, false] {
        let draft = draft("Checklist review admission");
        let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
        let (selected_model, expected) = if missing_model {
            (Some("missing".into()), "selected model profile is missing")
        } else {
            workflow.execution.memory_budget_mib = 1024;
            workflow.models[0].memory_cost_mib = 2048;
            (None, "exceeds the configured memory budget")
        };
        let html = render_checklist_review(draft, selected_model, workflow);

        assert!(
            opening_tag_with_marker(&html, "button", r#"id="exercise-format""#)
                .contains("disabled")
        );
        assert!(html.contains(expected));
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
        preparation: draft.preparation.clone(),
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
fn removing_a_reference_releases_only_its_runtime_preview() {
    let mut draft = draft("Reference removal");
    let removed = reference(draft.checklist.criteria[0].id);
    let remaining = reference(draft.checklist.criteria[0].id);
    let removed_id = removed.image.id;
    let remaining_id = remaining.image.id;
    draft.references = vec![removed, remaining.clone()];
    let exercise_id = draft.id;
    let mut editor = ExerciseEditorState::new(draft.clone(), None);
    editor.previews = [
        (removed_id, "removed thumbnail".into()),
        (remaining_id, "remaining thumbnail".into()),
    ]
    .into();
    let mut page = ExercisePageState {
        configurations: [(exercise_id, editor)].into(),
        ..Default::default()
    };

    page.remove_reference(exercise_id, removed_id);
    page.remove_reference(exercise_id, removed_id);

    let editor = &page.configurations[&exercise_id];
    assert_eq!(editor.draft.references, vec![remaining]);
    assert_eq!(
        editor.previews,
        [(remaining_id, "remaining thumbnail".into())].into()
    );
    assert_eq!(
        editor.saved, draft,
        "removal must not rewrite the saved snapshot"
    );
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
    editor.draft.preparation.proposal = Some(proposal);
    editor.apply_proposal().unwrap();
    assert_eq!(editor.draft.checklist.criteria[0].id, criterion_id);
    assert_eq!(editor.draft.references[0].criterion_ids, vec![criterion_id]);
    assert!(!editor.draft.checklist.source_text().contains("closed"));
    assert_eq!(
        editor.draft.preparation.last_review.unwrap().comments.len(),
        1
    );
}

fn propose(editor: &mut ExerciseEditorState) {
    use backend::exercise::{ChecklistProposal, ProposedCriterion, SourceRange};
    if editor.draft.preparation.source.is_none() {
        editor.draft.preparation.source = Some(editor.draft.checklist.clone());
    }
    let mut offset = 0;
    let criteria = editor
        .draft
        .source_checklist()
        .criteria
        .iter()
        .map(|item| {
            let start = offset;
            offset += item.text.len() + 2;
            ProposedCriterion {
                text: format!("Confirmed: {}", item.text),
                source_ranges: vec![SourceRange {
                    start,
                    end: offset - 2,
                }],
            }
        })
        .collect();
    editor
        .receive_proposal(
            ChecklistProposal::for_draft(&editor.draft, criteria, Vec::new()).unwrap(),
        )
        .unwrap();
}

#[test]
fn consumed_paste_rejects_queued_input_and_add_without_consuming_new_text() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    let previous_edit_id = editor.pasted_edit_id;
    editor.set_pasted_text(previous_edit_id, "First pasted step".into());
    editor.add_pasted_steps(previous_edit_id);
    assert!(editor.pasted_text.is_empty());
    assert_eq!(editor.draft.checklist.criteria.len(), 2);
    assert_eq!(editor.draft.checklist.criteria[1].text, "First pasted step");

    let current_edit_id = editor.pasted_edit_id;
    assert_ne!(current_edit_id, previous_edit_id);
    editor.set_pasted_text(current_edit_id, "Next operator text".into());
    editor.set_pasted_text(previous_edit_id, "Late old input".into());
    editor.add_pasted_steps(previous_edit_id);
    assert_eq!(editor.pasted_text, "Next operator text");
    assert_eq!(editor.draft.checklist.criteria.len(), 2);
}

#[test]
fn applied_wording_rejects_input_queued_from_the_previous_checklist_fields() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    let criterion_id = editor.draft.checklist.criteria[0].id;
    let previous_edit_id = editor.checklist_edit_id;
    editor.set_criterion_text(criterion_id, previous_edit_id, "Operator wording".into());
    propose(&mut editor);
    editor.apply_proposal().unwrap();

    let current_edit_id = editor.checklist_edit_id;
    assert_ne!(current_edit_id, previous_edit_id);
    editor.set_criterion_text(criterion_id, previous_edit_id, "Late old input".into());
    assert_eq!(
        editor.draft.checklist.criteria[0].text,
        "Confirmed: Operator wording"
    );
    editor.set_criterion_text(
        criterion_id,
        current_edit_id,
        "Further operator edits".into(),
    );
    assert_eq!(editor.draft.checklist.criteria[0].id, criterion_id);
    assert_eq!(
        editor.draft.checklist.criteria[0].text,
        "Further operator edits"
    );
}

#[test]
fn new_suggestions_reject_input_queued_from_the_previous_proposal_fields() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    propose(&mut editor);
    let previous_edit_id = editor.proposal_edit_id;
    editor.set_proposal_text(previous_edit_id, 0, "Edited first suggestion".into());
    assert_eq!(
        editor.draft.preparation.proposal.as_ref().unwrap().criteria[0].text,
        "Edited first suggestion"
    );

    propose(&mut editor);
    let current_edit_id = editor.proposal_edit_id;
    assert_ne!(current_edit_id, previous_edit_id);
    let expected = editor.draft.preparation.proposal.as_ref().unwrap().criteria[0]
        .text
        .clone();
    editor.set_proposal_text(previous_edit_id, 0, "Late old input".into());
    assert_eq!(
        editor.draft.preparation.proposal.as_ref().unwrap().criteria[0].text,
        expected
    );
    editor.set_proposal_text(current_edit_id, 0, "Edited new suggestion".into());
    assert_eq!(
        editor.draft.preparation.proposal.as_ref().unwrap().criteria[0].text,
        "Edited new suggestion"
    );
}

#[test]
fn applying_fresh_wording_preserves_operator_additions_deletions_and_order() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    let removed = Criterion {
        id: Uuid::new_v4(),
        text: "Original later step".into(),
    };
    editor.draft.checklist.criteria.push(removed.clone());
    propose(&mut editor);
    editor.apply_proposal().unwrap();
    let added = Criterion {
        id: Uuid::new_v4(),
        text: "Operator-added step".into(),
    };
    editor
        .draft
        .checklist
        .criteria
        .retain(|item| item.id != removed.id);
    editor.draft.checklist.criteria.insert(0, added.clone());
    editor.draft.references.push(reference(added.id));
    propose(&mut editor);
    editor.apply_proposal().unwrap();
    assert_eq!(editor.draft.checklist.criteria[0], added);
    assert_eq!(editor.draft.checklist.criteria.len(), 2);
    assert_eq!(editor.draft.references[0].criterion_ids, vec![added.id]);
    assert!(editor.draft.validate().is_ok());
}

#[test]
fn including_added_steps_preserves_original_source_and_operator_final_wording() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    editor.draft.preparation.source = Some(editor.draft.checklist.clone());
    let original = editor.draft.checklist.criteria[0].clone();
    editor.draft.checklist.criteria[0].text = "Operator revised this wording".into();
    let added = Criterion {
        id: Uuid::new_v4(),
        text: "A newly required step".into(),
    };
    editor.draft.checklist.criteria.insert(0, added.clone());
    let final_checklist = editor.draft.checklist.clone();
    editor.include_new_source_steps();
    editor.include_new_source_steps();
    assert_eq!(editor.draft.checklist, final_checklist);
    assert_eq!(
        editor.draft.source_checklist().criteria,
        vec![original, added]
    );
}

#[test]
fn edited_and_dismissed_suggestions_survive_a_draft_round_trip() {
    let mut editor = ExerciseEditorState::new(draft("Example"), None);
    propose(&mut editor);
    editor.draft.preparation.proposal.as_mut().unwrap().criteria[0].text =
        "Operator-edited proposal".into();
    let stored = serde_json::to_vec(&editor.draft).unwrap();
    let mut reloaded = ExerciseEditorState::new(serde_json::from_slice(&stored).unwrap(), None);
    assert_eq!(
        reloaded
            .draft
            .preparation
            .proposal
            .as_ref()
            .unwrap()
            .criteria[0]
            .text,
        "Operator-edited proposal"
    );
    reloaded.draft.preparation.proposal = None;
    let stored = serde_json::to_vec(&reloaded.draft).unwrap();
    let reloaded = ExerciseEditorState::new(serde_json::from_slice(&stored).unwrap(), None);
    assert!(reloaded.draft.preparation.proposal.is_none());
}

#[test]
fn source_edits_preserve_old_question_meaning_and_require_explicit_reconfirmation() {
    let mut draft = draft("Example");
    draft.preparation.source = Some(draft.checklist.clone());
    let item = &draft.checklist.criteria[0];
    draft
        .preparation
        .questions
        .push(backend::exercise::Clarification {
            criterion_id: item.id,
            source_text: item.text.clone(),
            question: "Where?".into(),
            answer: "In the case".into(),
        });
    draft.preparation.reconcile().unwrap();
    let old_source = draft.preparation.questions[0].source_text.clone();
    draft.preparation.source.as_mut().unwrap().criteria[0].text =
        "Place the item on the shelf.".into();
    draft.preparation.reconcile().unwrap();
    assert_eq!(draft.preparation.questions[0].source_text, old_source);
    assert!(
        draft.validate_for_publication().is_err(),
        "acknowledging final wording cannot silently reactivate a stale answer"
    );
    draft.preparation.questions.clear();
    draft.preparation.reconcile().unwrap();
    assert!(draft.validate_for_publication().is_ok());
}
