//! Native exercise editing preserves queued input, step bounds and the saved operator draft.

use super::*;
use backend::exercise::{Clarification, Criterion, ExerciseStore, ReferenceAsset};

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_preparation_navigation_keeps_queued_events_in_bounds() {
    let (saved, _) = run_scenario("preparation-navigation", |_| {}, 0);
    assert_eq!(saved.name, "Navigation remains editable");
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_operator_text_keeps_newer_input_during_a_delayed_render() {
    let (saved, _) = run_scenario("rapid-exercise-input", prepare_text_fields, 0);
    assert_eq!(saved.name, "abc");
    assert_eq!(saved.preparation.scene_context, "abc");
    assert_eq!(saved.preparation.source.unwrap().criteria[0].text, "abc");
    assert_eq!(saved.preparation.questions[0].answer, "abc");
    let reference = &saved.references[0];
    assert_eq!(reference.object_label, "abc");
    assert_eq!(reference.state_label, "abc");
    assert_eq!(reference.purpose, "abc");
    assert_eq!(
        saved.checklist.criteria[0].text,
        "Place the item in the case."
    );
    assert_eq!(saved.checklist.criteria.len(), 2);
    assert_eq!(saved.checklist.criteria[1].text, "abc");
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_checklist_typing_and_explicit_proposals_stay_in_sync() {
    let (saved, publications) = run_scenario("checklist-replacement", prepare_text_fields, 2);
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].edit_version, saved.edit_version);
    assert_eq!(
        saved.checklist.criteria[0].text,
        "Applied proposal wording."
    );
    assert_eq!(
        saved.preparation.source.unwrap().criteria[0].text,
        "Place the item in the case."
    );
    assert_eq!(
        saved.preparation.last_review.unwrap().criteria[0].text,
        "Applied proposal wording."
    );
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_publication_requires_the_current_confirmation_step() {
    let (saved, publications) = run_scenario("checklist-publish-events", prepare_text_fields, 0);
    assert_eq!(
        saved.checklist.criteria[0].text,
        "Changed after leaving confirmation."
    );
    assert_eq!(publications.len(), 1);
    assert_eq!(publications[0].edit_version, saved.edit_version);
    assert_eq!(publications[0].checklist, saved.checklist);
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_paste_controls_cannot_consume_a_replacement_buffer() {
    let (saved, _) = run_scenario("checklist-paste-events", prepare_text_fields, 0);
    let wording: Vec<_> = saved
        .checklist
        .criteria
        .iter()
        .map(|step| step.text.as_str())
        .collect();
    assert_eq!(
        wording,
        [
            "Place the item in the case.",
            "First added step",
            "Second added step"
        ]
    );
}

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_delete_confirmation_stays_with_the_intended_step() {
    let (saved, _) = run_scenario(
        "checklist-delete-events",
        |store| {
            prepare_text_fields(store);
            let mut draft = store.list().unwrap().drafts.pop().unwrap();
            draft.checklist.criteria.push(Criterion {
                id: "00000000-0000-0000-0000-000000000002".parse().unwrap(),
                text: "Unlinked second step".into(),
            });
            store.save_draft(&draft).unwrap();
        },
        0,
    );
    assert_eq!(saved.checklist.criteria.len(), 1);
    assert_eq!(
        saved.checklist.criteria[0].text,
        "Place the item in the case."
    );
    assert_eq!(
        saved.references[0].criterion_ids,
        [saved.checklist.criteria[0].id]
    );
}

fn prepare_text_fields(store: &ExerciseStore) {
    let mut draft = store.create_draft("Prepared text exercise").unwrap();
    let criterion = Criterion {
        id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        text: "Place the item in the case.".into(),
    };
    draft.checklist.criteria.push(criterion.clone());
    draft.preparation.source = Some(draft.checklist.clone());
    draft.preparation.questions.push(Clarification {
        criterion_id: criterion.id,
        question: "Which case?".into(),
        source_text: criterion.text,
        answer: "The marked case".into(),
    });
    let image = store
        .import_reference(draft.id, &fixture("reference.png"))
        .unwrap();
    draft.references.push(ReferenceAsset {
        image,
        object_label: "Case".into(),
        state_label: "Open".into(),
        purpose: "Identify the storage location".into(),
        criterion_ids: vec![criterion.id],
    });
    store.save_draft(&draft).unwrap();
}

fn run_scenario(
    scenario: &str,
    prepare: impl FnOnce(&ExerciseStore),
    expected_requests: usize,
) -> (
    backend::exercise::ExerciseDraft,
    Vec<backend::exercise::ExerciseRevision>,
) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let store = ExerciseStore::new(data.join("exercises"));
    prepare(&store);
    let mock = MockOpenAi::start();
    let settings = write_desktop_settings(
        root.path(),
        &data,
        [
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
        ],
        5,
        "local-preparation-navigation",
        "local-preparation-navigation",
        Some(&format!("http://{}/v1", mock.address)),
    );
    let mut config: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    config["cameras"] = json!([]);
    if scenario == "checklist-replacement" {
        let mut selected_model = config["workflow"]["models"][0].clone();
        selected_model["id"] = json!("selected-model");
        selected_model["name"] = json!("Selected review model");
        selected_model["model"] = json!("selected-model");
        config["workflow"]["models"]
            .as_array_mut()
            .unwrap()
            .push(selected_model);
    }
    fs::write(&settings, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let logs = root.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let ready = root.path().join("driver-ready");
    let result = root.path().join("driver-result");
    let mut app = start_desktop_app(&settings, &ready, &result, scenario, true, &logs);
    let message = wait_for_desktop_result(&mut app, &ready, &result);
    assert!(message.starts_with("ok\n"), "{message}");
    let mut drafts = store.list().unwrap();
    assert_eq!(drafts.drafts.len(), 1);
    assert_eq!(mock.requests.lock().unwrap().len(), expected_requests);
    if scenario == "checklist-replacement" {
        assert!(
            mock.requests
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["model"] == "selected-model"),
            "each explicit review must use the selected model"
        );
    }
    let saved = drafts.drafts.pop().unwrap();
    let publications = store.revisions(saved.id).unwrap();
    (saved, publications)
}
