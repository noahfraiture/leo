use super::load_editor;
use backend::exercise::{Criterion, ExerciseStore};

#[test]
fn a_damaged_publication_keeps_its_draft_editable_without_claiming_it_is_published() {
    let temporary = tempfile::tempdir().unwrap();
    let store = ExerciseStore::new(temporary.path().join("exercises"));
    let mut draft = store.create_draft("Repairable configuration").unwrap();
    draft.checklist.criteria.push(Criterion {
        id: uuid::Uuid::new_v4(),
        text: "Place the item in the case".into(),
    });
    let draft = store.save_draft(&draft).unwrap();
    let revision = store.publish_revision(&draft).unwrap();
    let revision_path = store
        .root
        .join(draft.id.to_string())
        .join("revisions")
        .join(format!("{}.json", revision.id));
    std::fs::write(&revision_path, b"damaged revision").unwrap();

    let editor = load_editor(&store, draft.clone());

    assert_eq!(editor.draft, draft);
    assert_eq!(editor.saved, draft);
    assert!(editor.published.is_none());
    assert!(
        editor
            .storage_error
            .unwrap()
            .contains("Saved publications could not load")
    );
    assert_eq!(std::fs::read(revision_path).unwrap(), b"damaged revision");
}
