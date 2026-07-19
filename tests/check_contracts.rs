use model_capability_doctor::checks::{Body, ManifestRefs, PlanContext, RequestGroup, plan};
use model_capability_doctor::protocol::{AuthMode, Protocol};
use serde_json::Value;

fn context(protocol: Protocol) -> PlanContext<'static> {
    PlanContext {
        protocol,
        auth_mode: AuthMode::Bearer,
        model: "fixture-model",
    }
}

fn requests(id: &str, protocol: Protocol) -> Vec<model_capability_doctor::checks::PlannedRequest> {
    plan(id, &context(protocol))
        .unwrap()
        .groups
        .into_iter()
        .flat_map(|group| match group {
            RequestGroup::Sequential(requests) | RequestGroup::Concurrent(requests) => requests,
        })
        .collect()
}

fn body_text(body: &Body) -> String {
    String::from_utf8(body.to_bytes()).unwrap()
}

#[test]
fn checks_001_through_039_have_expected_request_counts_and_ids() {
    for number in 1..=39 {
        let id = format!("{number:03}");
        let plan = plan(&id, &context(Protocol::OpenAiChat)).unwrap();
        let request_ids: Vec<String> = plan
            .groups
            .iter()
            .flat_map(RequestGroup::requests)
            .map(|request| request.id.clone())
            .collect();
        let expected = match id.as_str() {
            "002" | "003" | "007" => 0,
            "033" => 2,
            _ => 1,
        };
        assert_eq!(request_ids.len(), expected, "check {id}");
        match id.as_str() {
            "002" => assert_eq!(plan.manifest_refs, ManifestRefs::AllProtocolProbes),
            "003" | "007" => {
                assert_eq!(plan.manifest_refs, ManifestRefs::SelectedProtocolProbe)
            }
            "033" => assert_eq!(request_ids, ["test-033-low", "test-033-high"]),
            _ => assert_eq!(request_ids, [format!("test-{id}")]),
        }
    }
}

#[test]
fn checks_001_through_039_keep_their_prompt_markers() {
    let expected_fragments = [
        ("001", "MODEL_DOCTOR_OK"),
        ("004", "MODEL_DOCTOR_CASE_004_OK"),
        ("005", "MODEL_DOCTOR_CASE_005_OK"),
        ("006", "MODEL_DOCTOR_CASE_006_OK"),
        ("008", "{\"model\":"),
        ("009", "MODEL_DOCTOR_CASE_009"),
        ("010", "MODEL_DOCTOR_CASE_010"),
        ("011", "MODEL_DOCTOR_CASE_011"),
        ("012", "MODEL_DOCTOR_CASE_012"),
        ("013", "MODEL_DOCTOR_CASE_013"),
        ("014", "CTX_014_OK"),
        ("015", "CTX_015_OK"),
        ("016", "CTX_016_OK"),
        ("017", "CTX_017_OK"),
        ("018", "CTX_018_OK"),
        ("019", "MODEL_DOCTOR_CASE_019"),
        ("020", "MODEL_DOCTOR_CASE_020"),
        ("021", "MODEL_DOCTOR_CASE_021"),
        ("022", "MODEL_DOCTOR_CASE_022"),
        ("023", "MODEL_DOCTOR_CASE_023"),
        ("024", "MODEL_DOCTOR_CASE_024"),
        ("025", "MODEL_DOCTOR_CASE_025"),
        ("026", "CTX_026_OK"),
        ("027", "CTX_027_OK"),
        ("028", "CTX_028_OK"),
        ("029", "<first-marker>;<second-marker>;<prefix>-<suffix>"),
        ("030", "ZX-7319"),
        ("031", "NEW_STATE"),
        ("032", "MODEL_DOCTOR_THINKING_OK"),
        ("033", "MODEL_DOCTOR_THINKING_OK"),
        ("034", "MODEL_DOCTOR_THINKING_OK"),
        ("035", "Compute 19 + 23 internally"),
        ("036", "MODEL_DOCTOR_CASE_036_OK"),
        ("037", "MODEL_DOCTOR_CASE_037"),
        ("038", "MODEL_DOCTOR_CASE_038"),
        ("039", "MODEL_DOCTOR_CASE_039"),
    ];

    for (id, fragment) in expected_fragments {
        let planned = requests(id, Protocol::OpenAiChat);
        assert!(
            planned
                .iter()
                .any(|request| body_text(&request.body).contains(fragment)),
            "check {id} is missing {fragment:?}"
        );
    }
}

#[test]
fn context_capacity_checks_preserve_shell_character_targets() {
    for (id, target) in [
        ("014", 32_000),
        ("015", 64_000),
        ("016", 128_000),
        ("017", 256_000),
        ("018", 512_000),
    ] {
        let body = body_text(&requests(id, Protocol::OpenAiChat)[0].body);
        assert!(
            body.len() >= target,
            "check {id}: {} < {target}",
            body.len()
        );
        assert!(body.contains(&format!("CTX_{id}_OK")));
        assert!(body.contains("FILLER_BLOCK_0123456789 "));
    }
}

#[test]
fn special_interface_context_and_thinking_payloads_match_contracts() {
    let reachability = requests("001", Protocol::AnthropicMessages);
    assert_eq!(reachability[0].protocol, Protocol::Unknown);
    assert_eq!(reachability[0].auth_mode, AuthMode::Bearer);
    assert_eq!(
        reachability[0].body.json()["messages"][0]["content"],
        "Reply only MODEL_DOCTOR_OK"
    );

    let malformed = requests("008", Protocol::OpenAiChat);
    assert!(matches!(malformed[0].body, Body::Raw(_)));
    assert_eq!(malformed[0].body.to_bytes(), br#"{"model":"#);

    let middle = body_text(&requests("027", Protocol::OpenAiChat)[0].body);
    assert!(middle.matches("FILLER_BLOCK_0123456789 ").count() >= 1_200);
    assert!(middle.contains("Hidden value: CTX_027_OK"));

    let multi_turn = requests("031", Protocol::GeminiGenerateContent);
    let body = multi_turn[0].body.json();
    assert_eq!(body["contents"][1]["role"], "model");
    assert_eq!(
        body["contents"][2]["parts"][0]["text"],
        "Correction: current state is NEW_STATE. Reply only NEW_STATE."
    );

    let effort = requests("033", Protocol::OpenAiResponses);
    assert_eq!(effort[0].body.json()["reasoning"]["effort"], "low");
    assert_eq!(effort[1].body.json()["reasoning"]["effort"], "high");

    let stream = requests("036", Protocol::OpenAiChat);
    assert!(stream[0].stream);
    assert_eq!(stream[0].body.json()["stream"], true);
}

#[test]
fn every_static_json_body_is_valid_for_each_protocol() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::OllamaChat,
    ] {
        for number in 1..=39 {
            let id = format!("{number:03}");
            for request in requests(&id, protocol) {
                if let Body::Json(value) = request.body {
                    let bytes = serde_json::to_vec(&value).unwrap();
                    let reparsed: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(reparsed, value, "{protocol} check {id}");
                }
            }
        }
    }
}
