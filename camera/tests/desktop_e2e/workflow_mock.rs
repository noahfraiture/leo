//! Deterministic provider contracts for the real operator UI, never model quality evidence.
use serde_json::{Value, json};

pub fn input_document(request: &Value) -> Value {
    request["input"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|message| message["role"] == "user")
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .filter_map(|content| content["text"].as_str())
        .find_map(|text| serde_json::from_str(text).ok())
        .expect("workflow request includes a JSON input document")
}

pub fn answer(request: &Value) -> Value {
    let document = input_document(request);
    if let Some(sources) = document["source"].as_array() {
        return json!({"criteria":sources.iter().map(|source| json!({"source_id":source["id"],"text":source["text"]})).collect::<Vec<_>>(),"comments":[{"source_id":sources[0]["id"],"question":"Should pauses be allowed?"}]});
    }
    let finding = |criterion: &Value, evidence: Vec<Value>| json!({"criterionId":criterion["id"],"judgment":"met","reason":"Fixture action is visible.","evidenceIds":evidence,"configurationIssue":null,"conflictingEvidence":false});
    match document["jobKind"].as_str().expect("typed workflow job") {
        "scan" => {
            let evidence = document["candidateFrames"][0]["id"].clone();
            assert!(evidence.is_string());
            json!({"observations": document["detail"]["checklist"]["criteria"].as_array().unwrap().iter().map(|criterion| json!({"criterionId":criterion["id"],"description":"Visible fixture movement.","evidenceIds":[evidence]})).collect::<Vec<_>>()})
        }
        "consolidate" => {
            json!({"findings": document["detail"]["checklist"]["criteria"].as_array().unwrap().iter().map(|criterion| {
            let evidence = document["detail"]["observations"].as_array().unwrap().iter().find(|observation| observation["criterionId"] == criterion["id"]).unwrap()["evidenceIds"].as_array().unwrap().clone();
            finding(criterion, evidence)
        }).collect::<Vec<_>>()})
        }
        "recheck" => finding(
            &document["detail"]["criterion"],
            vec![document["candidateFrames"][0]["id"].clone()],
        ),
        other => panic!("unexpected workflow stage {other}"),
    }
}
