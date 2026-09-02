use std::fs;

use super::build_subscriber;
use crate::settings::LogLevel;
use dioxus::prelude::*;
use serde_json::Value;

const VNODE_SECRET: &str = "vnode-private-value-sentinel";

#[component]
fn SensitiveInput(value: String) -> Element {
    rsx! { input { value } }
}

#[test]
fn writes_structured_events_to_daily_json_log() {
    let directory = tempfile::tempdir().unwrap();
    let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Info).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(camera_id = 41, camera_count = 2, "preview ready");
    });
    drop(guard);

    let entries = fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("leo.jsonl.")
    );
    let contents = fs::read_to_string(&entries[0]).unwrap();
    let event: Value = contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .find(|event: &Value| event["fields"]["message"] == "preview ready")
        .expect("structured preview event should be written");

    assert_eq!(event["level"], "INFO");
    assert_eq!(event["fields"]["camera_id"], 41);
    assert_eq!(event["fields"]["camera_count"], 2);
}

#[test]
fn warn_level_omits_info_and_retains_warning() {
    let directory = tempfile::tempdir().unwrap();
    let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Warn).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        tracing::info!("filtered info event");
        tracing::warn!("retained warning event");
    });
    drop(guard);

    let path = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = fs::read_to_string(path).unwrap();

    assert!(!contents.contains("filtered info event"));
    assert!(contents.contains("retained warning event"));
}

#[test]
fn never_writes_provider_payloads_when_trace_is_enabled() {
    let directory = tempfile::tempdir().unwrap();
    let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Trace).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!(
            target: "rig::completions",
            payload = "private checklist and image bytes",
            "provider request"
        );
    });
    drop(guard);

    let path = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = fs::read_to_string(path).unwrap();

    assert!(!contents.contains("private checklist and image bytes"));
}

#[test]
fn never_writes_dynamic_vnode_values_when_trace_is_enabled() {
    let directory = tempfile::tempdir().unwrap();
    let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Trace).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        tracing::trace!("application trace marker");
        let mut dom = VirtualDom::new_with_props(
            SensitiveInput,
            SensitiveInputProps {
                value: VNODE_SECRET.into(),
            },
        );
        dom.rebuild_in_place();
    });
    drop(guard);

    let path = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = fs::read_to_string(path).unwrap();

    assert!(contents.contains("application trace marker"));
    assert!(!contents.contains(VNODE_SECRET));
}
