//! Queued DOM events must keep clarification identity when a neighboring row is removed.

use super::*;
use backend::exercise::{Clarification, Criterion, ExerciseStore};

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_question_events_keep_the_remaining_answer_intact() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let store = ExerciseStore::new(data.join("exercises"));
    let mut draft = store.create_draft("Queued question exercise").unwrap();
    let criterion = Criterion {
        id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        text: "Place the item in the case".into(),
    };
    draft.checklist.criteria.push(criterion.clone());
    draft.preparation.questions = [
        ("First question", "First answer"),
        ("Second question", "Second answer"),
    ]
    .into_iter()
    .map(|(question, answer)| Clarification {
        criterion_id: criterion.id,
        question: question.into(),
        source_text: criterion.text.clone(),
        answer: answer.into(),
    })
    .collect();
    let draft = store.save_draft(&draft).unwrap();
    let mock = MockOpenAi::start();
    let settings = write_desktop_settings(
        root.path(),
        &data,
        [
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
        ],
        5,
        "local-question-events",
        "local-question-events",
        Some(&format!("http://{}/v1", mock.address)),
    );
    let mut config: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    config["cameras"] = json!([]);
    fs::write(&settings, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let logs = root.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let ready = root.path().join("driver-ready");
    let result = root.path().join("driver-result");
    let mut app = start_desktop_app(&settings, &ready, &result, "question-events", true, &logs);
    let message = wait_for_desktop_result(&mut app, &ready, &result);

    assert!(message.starts_with("ok\n"), "{message}");
    let saved = store.load_draft(draft.id).unwrap();
    assert_eq!(saved.preparation.questions.len(), 1);
    assert_eq!(saved.preparation.questions[0].question, "Second question");
    assert_eq!(
        saved.preparation.questions[0].answer,
        "Updated second answer"
    );
    assert!(mock.requests.lock().unwrap().is_empty());
}
