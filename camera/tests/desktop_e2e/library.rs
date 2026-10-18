//! Native library recovery using retained malformed files and a provider that must stay unused.

use super::*;
use backend::exercise::{Criterion, ExerciseStore};

#[test]
#[ignore = "requires a macOS GUI session and disposable local fixture settings"]
fn desktop_exercise_library_keeps_healthy_configurations_available() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    fs::create_dir(&data).unwrap();
    let store = ExerciseStore::new(data.join("exercises"));
    let mut healthy = store.create_draft("Healthy saved configuration").unwrap();
    healthy.checklist.criteria.push(Criterion {
        id: "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        text: "Place the item in the case".into(),
    });
    let healthy = store.save_draft(&healthy).unwrap();
    store.publish_revision(&healthy).unwrap();
    let mut broken_publication = store.create_draft("Publication needs repair").unwrap();
    broken_publication.checklist = healthy.checklist.clone();
    let broken_publication = store.save_draft(&broken_publication).unwrap();
    let revision = store.publish_revision(&broken_publication).unwrap();
    let revision_path = store
        .root
        .join(broken_publication.id.to_string())
        .join("revisions")
        .join(format!("{}.json", revision.id));
    fs::write(&revision_path, b"damaged publication").unwrap();
    let broken_draft = store.create_draft("Damaged draft").unwrap();
    let draft_path = store
        .root
        .join(broken_draft.id.to_string())
        .join("draft.json");
    fs::write(&draft_path, b"damaged draft").unwrap();

    let mock = MockOpenAi::start();
    let settings = write_desktop_settings(
        root.path(),
        &data,
        [
            "127.0.0.1:1".parse().unwrap(),
            "127.0.0.1:2".parse().unwrap(),
        ],
        5,
        "local-exercise-recovery",
        "local-exercise-recovery",
        Some(&format!("http://{}/v1", mock.address)),
    );
    let mut config: Value = serde_json::from_slice(&fs::read(&settings).unwrap()).unwrap();
    config["cameras"] = json!([]);
    fs::write(&settings, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
    let logs = root.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let ready = root.path().join("driver-ready");
    let result = root.path().join("driver-result");
    let mut app = start_desktop_app(&settings, &ready, &result, "exercise-recovery", true, &logs);
    let message = wait_for_desktop_result(&mut app, &ready, &result);

    assert!(message.starts_with("ok\n"), "{message}");
    assert_eq!(
        store.load_draft(healthy.id).unwrap().name,
        "Healthy configuration updated"
    );
    assert_eq!(store.revisions(healthy.id).unwrap().len(), 2);
    assert_eq!(fs::read(draft_path).unwrap(), b"damaged draft");
    assert_eq!(fs::read(revision_path).unwrap(), b"damaged publication");
    assert!(mock.requests.lock().unwrap().is_empty());
}
