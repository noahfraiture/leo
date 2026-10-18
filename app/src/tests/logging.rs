use std::fs;

use super::build_subscriber;
use crate::settings::LogLevel;
use dioxus::prelude::*;
use serde_json::Value;

const VNODE_SECRET: &str = "vnode-private-value-sentinel";
const PROVIDER_ERROR_SECRET: &str = "provider-error-private-value-sentinel";

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
fn provider_library_errors_cannot_bypass_application_log_redaction() {
    use backend::model::{JobRole, ModelClient, StructuredJob};
    use std::time::Duration;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|| async {
            axum::Json(serde_json::json!({"error": {"message": PROVIDER_ERROR_SECRET}}))
        }),
    );
    let server = runtime.spawn(async move { axum::serve(listener, router).await.unwrap() });
    let mut workflow = crate::test_workflow(Some(crate::test_openai_config()), 8, 0);
    workflow.providers[0].base_url = format!("http://{address}/v1");
    workflow.providers[0].transport = backend::model::Transport::ChatCompletions;
    let client = ModelClient::new(
        &workflow.providers[0],
        &workflow.models[0],
        Duration::from_secs(2),
    )
    .unwrap();
    let request = StructuredJob {
        role: JobRole::ChecklistFormatter,
        instructions: "Return structured JSON.".into(),
        text: "Local log-redaction fixture.".into(),
        images: Vec::new(),
        max_output_tokens: Some(128),
    };
    let directory = tempfile::tempdir().unwrap();
    let (subscriber, guard) = build_subscriber(directory.path(), LogLevel::Warn).unwrap();

    tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            tracing::warn!("application warning remains visible");
            let error = client.complete::<Value>(&request).await.unwrap_err();
            assert!(matches!(
                error,
                backend::model::Error::ProviderStatus { status: 200 }
            ));
            assert!(!format!("{error:?}").contains(PROVIDER_ERROR_SECRET));
        });
    });
    server.abort();
    drop(guard);

    let path = fs::read_dir(directory.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let contents = fs::read_to_string(path).unwrap();
    assert!(contents.contains("application warning remains visible"));
    assert!(!contents.contains(PROVIDER_ERROR_SECRET));
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
