use llm_capability_doctor::capability::*;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: replay <capability.input.json>");
    let raw = std::fs::read_to_string(path).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let payload = &doc["evidence"][0]["payload"]["payload"];
    let card = &payload["scorecard"];
    let catalog = fixed_capability_catalog();
    let mut mism = 0;
    for o in card["observations"].as_array().unwrap() {
        let id = o["sample_id"].as_str().unwrap();
        let Some(sample) = catalog.iter().find(|s| s.id == id) else {
            continue;
        };
        let Some(ev) = payload["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["sample_id"] == id)
        else {
            continue;
        };
        let body = &ev["payload"]["response"]["body"];
        let text = body["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string);
        let truncated = body["choices"][0]["finish_reason"].as_str() == Some("length");
        let obs = evaluate_response(
            sample,
            CapabilityResponse {
                execution: ExecutionState::Valid,
                text,
                tool_calls: vec![],
                truncated,
                evidence_refs: vec![],
            },
        );
        let stored = o["label"].as_str().unwrap();
        let mine = format!("{:?}", obs.label).to_lowercase();
        if mine != stored {
            mism += 1;
            println!(
                "{} mine={} stored={} reason={:?}",
                id, mine, stored, obs.reason
            );
        }
    }
    println!("total mismatches: {}", mism);
}
