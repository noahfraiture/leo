//! Deterministic provider contracts for exercising the real operator UI, never model quality.
use serde_json::{Value, json};

/// Extracts the one role-specific input document from a Responses request.
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

/// Produces source-linked proposals and valid candidate citations using the actual request IDs.
pub fn answer(request: &Value) -> Value {
    let document = input_document(request);
    if let Some(sources) = document.as_array() {
        let criteria: Vec<_> = sources
            .iter()
            .map(|source| {
                json!({
                    "source_id": source["id"], "text": source["text"],
                })
            })
            .collect();
        return json!({"criteria":criteria,"comments":[{
            "source_id":sources[0]["id"], "question":"Should pauses be allowed?"
        }]});
    }
    let kind = document["jobKind"].as_str().expect("typed workflow job");
    if kind == "selectEvidence" {
        let reviews = document["detail"]["visualReviews"]
            .as_array()
            .expect("preserved visual reviews");
        let review_ids: Vec<_> = reviews
            .iter()
            .map(|review| review["jobId"].clone())
            .collect();
        let evidence = reviews[0]["completion"]["value"]["evidenceIds"].clone();
        return json!({
            "reason":"One directly reviewed event establishes the fixture criterion.",
            "reviews":[{"reviewIds":review_ids,"kind":"supporting","reason":"Same visible event in the fixture windows.",
                "decisiveIds":evidence,"requiredForDecision":true}],
            "supportingIds":evidence,"contradictingIds":[],"contextIds":[]
        });
    }
    let evidence: Vec<_> = document["candidateImageIds"]
        .as_array()
        .expect("candidate IDs")
        .iter()
        .take(1)
        .cloned()
        .collect();
    assert!(
        !evidence.is_empty(),
        "visual fixture requests must include footage"
    );
    let mut finding =
        json!({"judgment":"met","reason":"E2E visual evidence reviewed.","evidenceIds":evidence});
    if kind == "final" {
        finding["unresolvedFacts"] = json!([]);
        finding["requiresUnseenComparison"] = json!(false);
    } else {
        finding["facts"] = json!([{"description":"Visible fixture movement.","evidenceIds":evidence,"attempt":null,"identity":null}]);
        if kind == "review" {
            finding["needsMoreEvidence"] = json!(false);
            finding["requestedIntervals"] = json!([]);
        } else {
            assert_eq!(kind, "reader");
        }
    }
    finding
}
