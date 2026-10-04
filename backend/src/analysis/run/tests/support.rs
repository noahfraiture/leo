use super::super::{jobs, store, *};
use crate::{
    analysis::{
        CandidateFrame, EligiblePeriod, EvidenceManifest, EvidenceWindow, RecordingSource,
        SentImage,
    },
    exercise::{Criterion, ExerciseStore},
    model::{ExecutionPolicy, ModelProfile, ProviderConnection, Transport},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use uuid::Uuid;

pub struct Fixture {
    _directory: tempfile::TempDir,
    pub store: RunStore,
    pub run_id: Uuid,
    pub provider: ProviderConnection,
}
impl Fixture {
    pub fn new(base_url: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let session = directory.path().join("session");
        fs::create_dir(&session).unwrap();
        fs::write(session.join("source.mkv"), b"recording fixture").unwrap();
        let exercises = ExerciseStore::new(directory.path().join("exercises"));
        let mut draft = exercises.create_draft("Table placement").unwrap();
        draft.checklist.criteria = ["Place the cup.", "Then place the plate."]
            .into_iter()
            .map(|text| Criterion {
                id: Uuid::new_v4(),
                text: text.into(),
            })
            .collect();
        let saved = exercises.save_draft(&draft).unwrap();
        let exercise = exercises.publish_revision(&saved).unwrap();
        let provider = ProviderConnection {
            id: "local".into(),
            name: "Loopback only".into(),
            base_url: base_url.into(),
            transport: Transport::ChatCompletions,
            api_key: "fixture".into(),
        };
        let model = ModelProfile {
            id: "visual".into(),
            name: "Fixture".into(),
            provider_id: provider.id.clone(),
            model: "fixture".into(),
            memory_cost_mib: 0,
            image_limit: 8,
            max_output_tokens: Some(1500),
            reasoning_effort: None,
        }
        .resolve(std::slice::from_ref(&provider))
        .unwrap();
        let frames: Vec<_> = [0, 1000, 2000]
            .into_iter()
            .map(|time| CandidateFrame {
                id: format!("C1-T{time}"),
                camera_id: 1,
                source_index: 0,
                session_offset_ms: time,
                recording_offset_ms: time,
            })
            .collect();
        let evidence = EvidenceManifest {
            session_id: Uuid::new_v4(),
            session_start_utc_ms: 0,
            duration_ms: 3000,
            sources: vec![RecordingSource {
                relative_path: "source.mkv".into(),
                camera_id: 1,
                start_utc_ms: 0,
                end_utc_ms: 3000,
                size_bytes: 17,
                sha256: store::hash(b"recording fixture"),
            }],
            frame_sets: frames.iter().map(|frame| vec![frame.id.clone()]).collect(),
            frames,
            warnings: Vec::new(),
            eligible_periods: vec![EligiblePeriod {
                camera_id: 1,
                start_ms: 0,
                end_ms: 3000,
                sample_every_ms: 1000,
            }],
        };
        let windows = [
            vec!["C1-T0".into(), "C1-T1000".into()],
            vec!["C1-T1000".into(), "C1-T2000".into()],
        ]
        .into_iter()
        .enumerate()
        .map(|(index, frame_ids)| EvidenceWindow {
            frame_ids,
            start_ms: index as u64 * 1000,
            end_ms: index as u64 * 1000 + 1000,
        })
        .collect();
        let inputs = RunInputs {
            schema_version: 2,
            id: Uuid::new_v4(),
            created_utc_ms: 1,
            prompt_version: jobs::PROMPT_VERSION.into(),
            exercise,
            model,
            execution: ExecutionPolicy {
                memory_budget_mib: 0,
                request_timeout_seconds: 5,
                ..Default::default()
            },
            evidence_policy: EvidencePolicy::default(),
            initial_evidence: evidence.clone(),
            windows,
        };
        let jobs = inputs
            .windows
            .iter()
            .map(|window| {
                jobs::record(&inputs, JobKind::Scan, None, window.frame_ids.clone()).unwrap()
            })
            .collect();
        let checkpoint = RunCheckpoint {
            stage: RunStage::Preparing,
            jobs,
            evidence,
            assessments: Vec::new(),
            error: None,
        };
        let store = RunStore::new(session);
        store.create(&inputs, &checkpoint, &exercises).unwrap();
        let fixture = Self {
            _directory: directory,
            store,
            run_id: inputs.id,
            provider,
        };
        fixture.cache_frames();
        fixture
    }
    pub fn snapshot(&self) -> RunSnapshot {
        self.store.load(self.run_id).unwrap()
    }
    pub fn request(&self) -> ExecuteRun {
        ExecuteRun {
            session_directory: self.store.session_directory.clone(),
            run_id: self.run_id,
            providers: vec![self.provider.clone()],
        }
    }
    pub fn cache_frames(&self) {
        let snapshot = self.snapshot();
        let cache = self.store.directory(self.run_id).unwrap().join("cache");
        for frame in &snapshot.checkpoint.evidence.frames {
            let key = store::hash(
                &serde_json::to_vec(&(frame, snapshot.inputs.evidence_policy.image_size)).unwrap(),
            );
            let bytes = b"fixture jpeg bytes";
            store::write_bytes(&cache.join(format!("{key}.jpg")), bytes, true).unwrap();
            store::write_json(
                &cache.join(format!("{key}.json")),
                &SentImage {
                    candidate_id: frame.id.clone(),
                    sha256: store::hash(bytes),
                    size_bytes: bytes.len() as u64,
                },
                true,
            )
            .unwrap();
        }
    }
}

/// Strict loopback transport with a scripted single failure and captured request bodies.
pub struct ModelServer {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ModelServer {
    pub fn new(fail_once: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            let mut failed = !fail_once;
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_request(&mut stream);
                captured.lock().unwrap().push(request.clone());
                let (status, body) = if !failed {
                    failed = true;
                    (
                        503,
                        json!({"error":{"message":"scripted processing failure"}}),
                    )
                } else {
                    let body = request_body(&request);
                    let criteria = body["detail"]["checklist"]["criteria"].as_array();
                    let finding = |id: &Value, evidence: Value| json!({"criterionId":id,"judgment":"cannotVerify","reason":"The observations do not establish the entire required action.","evidenceIds":evidence,"configurationIssue":null,"conflictingEvidence":false});
                    let output = match body["jobKind"].as_str().unwrap() {
                        "scan" => {
                            json!({"observations":criteria.unwrap().iter().map(|item| json!({"criterionId":item["id"],"description":"The object is visible in this window.","evidenceIds":[body["candidateFrames"][0]["id"]]})).collect::<Vec<_>>()})
                        }
                        "consolidate" => json!({"findings":criteria.unwrap().iter().map(|item| {
                            let ids: Vec<_> = body["detail"]["observations"].as_array().unwrap().iter().filter(|observation| observation["criterionId"] == item["id"]).flat_map(|observation| observation["evidenceIds"].as_array().unwrap().clone()).collect();
                            finding(&item["id"], json!(ids))
                        }).collect::<Vec<_>>()}),
                        "recheck" => finding(
                            &body["detail"]["criterion"]["id"],
                            json!([body["candidateFrames"][0]["id"]]),
                        ),
                        other => panic!("unexpected request {other}"),
                    };
                    (
                        200,
                        json!({"id":"fixture","model":"fixture","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":output.to_string()}}]}),
                    )
                };
                let text = body.to_string();
                write!(stream,"HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",text.len()).unwrap();
            }
        });
        Self {
            base_url: format!("http://{address}/v1"),
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for ModelServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}
pub fn request_body(request: &Value) -> Value {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "user")
        .flat_map(|message| match &message["content"] {
            Value::String(text) => vec![text.as_str()],
            Value::Array(parts) => parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect(),
            _ => Vec::new(),
        })
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .find(|value| value.get("jobKind").is_some())
        .unwrap()
}
fn read_request(stream: &mut std::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 2_000_000);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
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
    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
}
