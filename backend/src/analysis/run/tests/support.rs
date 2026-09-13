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

use serde_json::{Value, json};
use uuid::Uuid;

use super::super::{jobs, model::CriterionWindows, store, *};
use crate::{
    analysis::{
        CandidateFrame, EligiblePeriod, EvidenceManifest, EvidenceWindow, RecordingSource,
        SentImage,
    },
    exercise::{Criterion, ExerciseStore},
    model::{ExecutionPolicy, ModelProfile, ModelSet, ProviderConnection, Transport},
};

pub struct Fixture {
    _directory: tempfile::TempDir,
    pub store: RunStore,
    pub run_id: Uuid,
    pub providers: Vec<ProviderConnection>,
}

impl Fixture {
    pub fn new(base_url: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let session_directory = directory.path().join("session");
        fs::create_dir(&session_directory).unwrap();
        // Frame extraction has separate FFmpeg checks. Cached bytes isolate durable job execution.
        fs::write(session_directory.join("source.mkv"), b"recording fixture").unwrap();
        let exercise_store = ExerciseStore::new(directory.path().join("exercises"));
        let mut draft = exercise_store.create_draft("Cup placement").unwrap();
        let criterion_id = Uuid::new_v4();
        draft.checklist.criteria.push(Criterion {
            id: criterion_id,
            text: "Place the blue cup on the table.".into(),
        });
        let draft = exercise_store.save_draft(&draft).unwrap();
        let revision = exercise_store.publish_revision(&draft).unwrap();
        let provider = ProviderConnection {
            id: "local".into(),
            name: "Loopback test".into(),
            base_url: base_url.into(),
            transport: Transport::ChatCompletions,
            api_key: "local-test".into(),
        };
        let profiles: Vec<_> = ["reader-a", "reader-b", "reviewer"]
            .into_iter()
            .map(|id| ModelProfile {
                id: id.into(),
                name: id.into(),
                provider_id: provider.id.clone(),
                model: id.into(),
                memory_cost_mib: 0,
                image_limit: 8,
                max_output_tokens: Some(256),
                reasoning_effort: None,
            })
            .collect();
        let model_set = ModelSet {
            id: "models".into(),
            name: "Readers".into(),
            reader_ids: vec!["reader-a".into(), "reader-b".into()],
            reviewer_id: "reviewer".into(),
            formatter_id: "reader-a".into(),
        }
        .resolve(&profiles, std::slice::from_ref(&provider))
        .unwrap();
        let frame = CandidateFrame {
            id: "C1-T0".into(),
            camera_id: 1,
            source_index: 0,
            session_offset_ms: 0,
            recording_offset_ms: 0,
        };
        let evidence = EvidenceManifest {
            session_id: Uuid::new_v4(),
            session_start_utc_ms: 0,
            duration_ms: 1000,
            sources: vec![RecordingSource {
                relative_path: "source.mkv".into(),
                camera_id: 1,
                start_utc_ms: 0,
                end_utc_ms: 1000,
                size_bytes: 17,
                sha256: store::hash(b"recording fixture"),
            }],
            frames: vec![frame.clone()],
            frame_sets: vec![vec![frame.id.clone()]],
            warnings: Vec::new(),
            eligible_periods: vec![EligiblePeriod {
                camera_id: 1,
                start_ms: 0,
                end_ms: 1000,
                sample_every_ms: 1000,
            }],
        };
        let run_id = Uuid::new_v4();
        let inputs = RunInputs {
            schema_version: 1,
            id: run_id,
            created_utc_ms: 1,
            prompt_version: super::super::selection_aliases::LEGACY_PROMPT.into(),
            exercise: revision,
            model_set,
            execution: ExecutionPolicy {
                memory_budget_mib: 0,
                max_concurrent_jobs: 2,
                request_timeout_seconds: 5,
                ..ExecutionPolicy::default()
            },
            evidence_policy: EvidencePolicy::default(),
            initial_evidence: evidence.clone(),
            windows: vec![CriterionWindows {
                criterion_id,
                windows: vec![EvidenceWindow {
                    frame_ids: vec![frame.id.clone()],
                    start_ms: 0,
                    end_ms: 0,
                }],
            }],
        };
        let jobs = inputs
            .model_set
            .readers
            .iter()
            .map(|model| {
                jobs::job_record(
                    &inputs,
                    criterion_id,
                    model,
                    JobKind::Reader,
                    0,
                    vec![frame.id.clone()],
                )
                .unwrap()
            })
            .collect();
        let checkpoint = RunCheckpoint {
            stage: RunStage::Preparing,
            jobs,
            evidence,
            gathered_criteria: Vec::new(),
            coverage_notes: Vec::new(),
            assessments: Vec::new(),
            error: None,
        };
        let store = RunStore::new(session_directory);
        store.create(&inputs, &checkpoint, &exercise_store).unwrap();
        let key =
            store::hash(&serde_json::to_vec(&(&frame, inputs.evidence_policy.image_size)).unwrap());
        let cache = store.directory(run_id).unwrap().join("cache");
        let jpeg = b"fixture jpeg";
        store::write_bytes(&cache.join(format!("{key}.jpg")), jpeg, false).unwrap();
        store::write_json(
            &cache.join(format!("{key}.json")),
            &SentImage {
                candidate_id: frame.id,
                sha256: store::hash(jpeg),
                size_bytes: jpeg.len() as u64,
            },
            false,
        )
        .unwrap();
        Self {
            _directory: directory,
            store,
            run_id,
            providers: vec![provider],
        }
    }

    pub fn execute_request(&self) -> ExecuteRun {
        ExecuteRun {
            session_directory: self.store.session_directory.clone(),
            run_id: self.run_id,
            providers: self.providers.clone(),
        }
    }
}

/// Loopback script accepts independent model jobs, failing reader A once when requested.
pub struct ModelServer {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ModelServer {
    pub fn new(fail_reader_once: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            let mut failed = !fail_reader_once;
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("accept mock model request: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let request = read_request(&mut stream);
                captured.lock().unwrap().push(request.clone());
                let (status, response) = if request["model"] == "reader-a" && !failed {
                    failed = true;
                    (503, json!({"error":{"message":"temporary failure"}}))
                } else {
                    let contents = request["messages"]
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
                        });
                    let body = contents
                        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
                        .find(|value| value.get("jobKind").is_some())
                        .unwrap();
                    let mut finding = json!({"judgment":"met","reason":"The cup is visibly on the table","evidenceIds":["C1-T0"],"facts":[]});
                    match body["jobKind"].as_str().unwrap() {
                        "selectEvidence" => {
                            finding = json!({"reason":"Use the visible placement", "reviews":[{"reviewIds":body["detail"]["visualReviews"].as_array().unwrap().iter().map(|review| review["jobId"].clone()).collect::<Vec<_>>(),"kind":"supporting","reason":"The placement is visible","decisiveIds":["C1-T0"],"requiredForDecision":true}], "supportingIds":["C1-T0"],"contradictingIds":[],"contextIds":[]});
                        }
                        "review" => {
                            finding["needsMoreEvidence"] = json!(false);
                            finding["requestedIntervals"] = json!([]);
                        }
                        "final" => {
                            finding.as_object_mut().unwrap().remove("facts");
                            finding["unresolvedFacts"] = json!([]);
                            finding["requiresUnseenComparison"] = json!(false);
                        }
                        "reader" => {}
                        value => panic!("unknown job kind {value}"),
                    }
                    (
                        200,
                        json!({"id":"completion","model":request["model"],"choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":finding.to_string()}}]}),
                    )
                };
                let text = response.to_string();
                write!(stream, "HTTP/1.1 {status} Result\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
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

fn read_request(stream: &mut std::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let header_end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 1_000_000);
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
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
