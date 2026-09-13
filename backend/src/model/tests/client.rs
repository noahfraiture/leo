use rig_core::test_utils::{MockCompletionModel, MockTurn};
use serde::Deserialize;

use super::*;
use crate::model::{ImageFormat, ImageRole, JobRole, StructuredImage};
use crate::profiles::ImageDetailPolicy;

#[derive(Debug, Deserialize, JsonSchema, PartialEq)]
#[schemars(crate = "rig_core::schemars")]
#[serde(deny_unknown_fields)]
struct Answer {
    observed: bool,
}

impl CompletionStatus for rig_core::test_utils::MockResponse {
    fn ensure_complete(&self) -> Result<()> {
        Ok(())
    }
}

fn job(role: JobRole) -> StructuredJob {
    StructuredJob {
        role,
        instructions: "Use only supplied inputs and return the requested JSON.".into(),
        text: "Check this criterion.".into(),
        images: Vec::new(),
        max_output_tokens: Some(128),
    }
}

#[tokio::test]
async fn structured_request_keeps_role_schema_and_reports_unknown_usage() {
    let model = MockCompletionModel::text(r#"{"observed":true}"#);
    let job = job(JobRole::ChecklistFormatter);
    let result: Completion<Answer> =
        complete(&model, &job, Duration::from_secs(2), Some(128), None)
            .await
            .unwrap();
    assert_eq!(result.value, Answer { observed: true });
    assert_eq!(result.usage, None);
    let requests = model.requests();
    assert!(
        serde_json::to_string(&requests[0].chat_history)
            .unwrap()
            .contains(&job.instructions)
    );
    assert_eq!(requests[0].max_tokens, Some(128));
    assert!(requests[0].output_schema.is_some());
}

#[tokio::test]
async fn malformed_json_never_becomes_a_candidate_assessment() {
    let model = MockCompletionModel::text(r#"{"observed":"secret-model-output"}"#);
    let result = complete::<_, Answer>(
        &model,
        &job(JobRole::VisualReviewer),
        Duration::from_secs(2),
        None,
        None,
    )
    .await;
    let error = result.unwrap_err();
    assert!(matches!(error, Error::InvalidResponse { .. }));
    assert!(!format!("{error:?}").contains("secret-model-output"));
}

#[tokio::test]
async fn provider_failure_is_sanitized_without_retrying() {
    let model = MockCompletionModel::new([MockTurn::error("secret-key and full request")]);
    let result = complete::<_, Answer>(
        &model,
        &job(JobRole::VisualReader),
        Duration::from_secs(2),
        None,
        None,
    )
    .await;
    assert!(matches!(result, Err(Error::RequestFailed)));
    assert_eq!(model.request_count(), 1);
}

#[test]
fn reference_and_candidate_images_have_distinct_prompt_roles() {
    let mut job = job(JobRole::VisualReviewer);
    job.images = [ImageRole::Reference, ImageRole::Candidate]
        .into_iter()
        .enumerate()
        .map(|(index, role)| StructuredImage {
            id: format!("image-{index}"),
            role,
            description: "Supplied image".into(),
            bytes: vec![1, 2, 3],
            format: ImageFormat::Jpeg,
            detail: ImageDetailPolicy::High,
        })
        .collect();
    let message = serde_json::to_string(&job.message().unwrap()).unwrap();
    assert!(message.contains("REFERENCE EXAMPLE: never evidence of candidate behavior"));
    assert!(message.contains("CANDIDATE RECORDING: cite this image ID"));
    assert!(message.contains("image-0"));
    assert!(message.contains("image-1"));
    assert!(message.contains("AQID"));
}

#[tokio::test]
async fn duplicate_image_ids_fail_before_a_model_request() {
    let mut job = job(JobRole::VisualReader);
    let image = StructuredImage {
        id: "same-id".into(),
        role: ImageRole::Candidate,
        description: String::new(),
        bytes: vec![1],
        format: ImageFormat::Png,
        detail: ImageDetailPolicy::ProviderDefault,
    };
    job.images = vec![image.clone(), image];
    let model = MockCompletionModel::default();
    let result = complete::<_, Answer>(&model, &job, Duration::from_secs(2), None, None).await;
    assert!(matches!(result, Err(Error::InvalidImage)));
    assert_eq!(model.request_count(), 0);
}

#[tokio::test]
async fn responses_transport_sends_the_explicit_model_and_json_schema() {
    wire_contract(Transport::Responses).await;
}

#[tokio::test]
async fn chat_transport_sends_the_explicit_model_and_json_schema() {
    wire_contract(Transport::ChatCompletions).await;
}

#[tokio::test]
async fn responses_output_limit_metadata_rejects_both_partial_and_valid_json() {
    for text in [r#"{"observed":"#, r#"{"observed":true}"#] {
        let error = wire_failure(Transport::Responses, move |response| {
            response["status"] = serde_json::json!("incomplete");
            response["incomplete_details"] = serde_json::json!({"reason":"max_output_tokens"});
            response["output"][0]["status"] = serde_json::json!("incomplete");
            response["output"][0]["content"][0]["text"] = serde_json::json!(text);
        })
        .await;
        assert!(matches!(error, Error::OutputLimitReached));
    }
}

#[tokio::test]
async fn chat_length_metadata_rejects_both_partial_and_valid_json() {
    for text in [r#"{"observed":"#, r#"{"observed":true}"#] {
        let error = wire_failure(Transport::ChatCompletions, move |response| {
            response["choices"][0]["finish_reason"] = serde_json::json!("length");
            response["choices"][0]["message"]["content"] = serde_json::json!(text);
        })
        .await;
        assert!(matches!(error, Error::OutputLimitReached));
    }
}

#[tokio::test]
async fn other_incomplete_reasons_stay_sanitized_and_are_not_mislabeled_as_token_limits() {
    for transport in [Transport::Responses, Transport::ChatCompletions] {
        let error = wire_failure(transport, move |response| match transport {
            Transport::Responses => {
                response["status"] = serde_json::json!("incomplete");
                response["incomplete_details"] =
                    serde_json::json!({"reason":"private-provider-detail"});
            }
            Transport::ChatCompletions => {
                response["choices"][0]["finish_reason"] =
                    serde_json::json!("private-provider-detail");
            }
        })
        .await;
        assert!(matches!(error, Error::IncompleteResponse));
        assert!(!format!("{error} {error:?}").contains("private-provider-detail"));
    }
}

#[tokio::test]
async fn partial_json_without_truncation_metadata_keeps_the_parse_diagnostic() {
    let error = wire_failure(Transport::Responses, |response| {
        response["output"][0]["content"][0]["text"] = serde_json::json!(r#"{"observed":"#);
    })
    .await;
    assert!(matches!(error, Error::InvalidResponse { .. }));
}

async fn wire_failure(
    transport: Transport,
    edit_response: impl FnOnce(&mut serde_json::Value) + Send + 'static,
) -> Error {
    let (base_url, received) = wire_server(transport, edit_response);
    let provider = ProviderConnection {
        id: "provider".into(),
        name: "Loopback incomplete response".into(),
        base_url,
        transport,
        api_key: "local-test-key".into(),
    };
    let profile = ModelProfile {
        id: "profile".into(),
        name: "Model status".into(),
        provider_id: provider.id.clone(),
        model: "explicit-model".into(),
        memory_cost_mib: 0,
        image_limit: 4,
        max_output_tokens: Some(128),
        reasoning_effort: None,
    };
    let client = ModelClient::new(&provider, &profile, Duration::from_secs(5)).unwrap();
    let error = client
        .complete::<Answer>(&job(JobRole::VisualReviewer))
        .await
        .unwrap_err();
    received.join().unwrap();
    error
}

async fn wire_contract(transport: Transport) {
    for role in [
        JobRole::ChecklistFormatter,
        JobRole::VisualReader,
        JobRole::VisualReviewer,
    ] {
        let (base_url, received) = wire_server(transport, |_| {});
        let provider = ProviderConnection {
            id: "provider".into(),
            name: "Loopback contract test".into(),
            base_url,
            transport,
            api_key: "local-test-key".into(),
        };
        let profile = ModelProfile {
            id: "profile".into(),
            name: "Model contract".into(),
            provider_id: provider.id.clone(),
            model: "explicit-model".into(),
            memory_cost_mib: 0,
            image_limit: 4,
            max_output_tokens: Some(128),
            reasoning_effort: Some("low".into()),
        };
        let client = ModelClient::new(&provider, &profile, Duration::from_secs(5)).unwrap();
        let mut request = job(role);
        if role != JobRole::ChecklistFormatter {
            request.images.push(StructuredImage {
                id: "candidate-1".into(),
                role: ImageRole::Candidate,
                description: "Camera 1 at 2 seconds".into(),
                bytes: vec![1, 2, 3],
                format: ImageFormat::Jpeg,
                detail: ImageDetailPolicy::High,
            });
        }
        let result = client.complete::<Answer>(&request).await.unwrap();
        assert!(result.value.observed);
        let (path, body) = received.join().unwrap();
        assert_eq!(body["model"], "explicit-model");
        let json = body.to_string();
        match transport {
            Transport::Responses => {
                assert_eq!(path, "/v1/responses");
                assert_eq!(body["text"]["format"]["type"], "json_schema");
                assert_eq!(body["max_output_tokens"], 128);
                assert_eq!(body["reasoning"]["effort"], "low");
            }
            Transport::ChatCompletions => {
                assert_eq!(path, "/v1/chat/completions");
                assert_eq!(body["response_format"]["type"], "json_schema");
                assert_eq!(body["max_tokens"], 128);
                assert_eq!(body["reasoning_effort"], "low");
            }
        }
        if role != JobRole::ChecklistFormatter {
            assert!(json.contains("data:image/jpeg;base64,AQID"));
            assert!(json.contains("candidate-1"));
        }
    }
}

fn wire_server(
    transport: Transport,
    edit_response: impl FnOnce(&mut serde_json::Value) + Send + 'static,
) -> (String, std::thread::JoinHandle<(String, serde_json::Value)>) {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let thread = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "model client never connected");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept fixture request: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
            assert!(bytes.len() < 100_000);
            if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
        let path = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        while bytes.len() < header_end + length {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            bytes.extend_from_slice(&buffer[..count]);
        }
        let request = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        let mut response = match transport {
            Transport::Responses => serde_json::json!({
                "id":"response-test", "object":"response", "created_at":1,
                "status":"completed", "model":"explicit-model", "tools":[],
                "output":[{"type":"message", "id":"message-test", "status":"completed",
                    "role":"assistant", "content":[{"type":"output_text", "annotations":[],
                    "text":"{\"observed\":true}"}]}]
            }),
            Transport::ChatCompletions => serde_json::json!({
                "id":"chat-test", "object":"chat.completion", "created":1,
                "model":"explicit-model", "choices":[{"index":0,"finish_reason":"stop",
                    "message":{"role":"assistant","content":"{\"observed\":true}"}}]
            }),
        };
        edit_response(&mut response);
        let response = response.to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        (path, request)
    });
    (format!("http://{address}/v1"), thread)
}
