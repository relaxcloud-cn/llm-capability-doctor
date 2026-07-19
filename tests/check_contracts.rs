use model_capability_doctor::checks::{Body, ManifestRefs, PlanContext, RequestGroup, plan};
use model_capability_doctor::protocol::tools::build_follow_up;
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

#[test]
fn tool_checks_have_one_initial_request_and_exact_prompt_markers() {
    for number in 40..=50 {
        let id = format!("{number:03}");
        let planned = requests(&id, Protocol::OpenAiChat);
        assert_eq!(planned.len(), 1, "check {id}");
        assert_eq!(planned[0].id, format!("test-{id}"));
        assert!(body_text(&planned[0].body).contains(&format!("MODEL_DOCTOR_CASE_{id}")));
    }
}

#[test]
fn tool_checks_preserve_required_nested_parallel_and_catalog_schemas() {
    let required = requests("043", Protocol::OpenAiChat)[0].body.json().clone();
    let parameters = &required["tools"][0]["function"]["parameters"];
    assert_eq!(
        parameters["required"],
        serde_json::json!(["city", "unit", "days"])
    );
    assert_eq!(
        parameters["properties"]["unit"]["enum"],
        serde_json::json!(["C", "F"])
    );
    assert_eq!(parameters["properties"]["days"]["type"], "integer");

    let nested = requests("044", Protocol::OpenAiResponses)[0]
        .body
        .json()
        .clone();
    assert_eq!(nested["tools"][0]["name"], "inspect_target");
    assert_eq!(
        nested["tools"][0]["parameters"]["properties"]["target"]["required"],
        serde_json::json!(["host", "port"])
    );

    let parallel = requests("045", Protocol::OpenAiChat)[0].body.json().clone();
    assert_eq!(parallel["parallel_tool_calls"], true);
    assert_eq!(parallel["tools"].as_array().unwrap().len(), 2);

    let catalog = requests("050", Protocol::AnthropicMessages)[0]
        .body
        .json()
        .clone();
    assert_eq!(catalog["max_tokens"], 2048);
    assert_eq!(catalog["tools"].as_array().unwrap().len(), 10);
    assert_eq!(catalog["tools"][9]["name"], "catalog_tool_8");

    let gemini = requests("040", Protocol::GeminiGenerateContent)[0]
        .body
        .json()
        .clone();
    assert_eq!(
        gemini["tools"][0]["functionDeclarations"][0]["name"],
        "get_weather"
    );

    let ollama = requests("040", Protocol::OllamaChat)[0].body.json().clone();
    assert_eq!(ollama["tools"][0]["function"]["name"], "get_weather");
}

#[test]
fn tool_catalog_keeps_protocol_specific_additional_properties_fields() {
    let chat = requests("050", Protocol::OpenAiChat)[0].body.json().clone();
    assert_eq!(
        chat["tools"][1]["function"]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(
        chat["tools"][2]["function"]["parameters"]["additionalProperties"],
        false
    );

    let anthropic = requests("050", Protocol::AnthropicMessages)[0]
        .body
        .json()
        .clone();
    assert_eq!(
        anthropic["tools"][1]["input_schema"].get("additionalProperties"),
        None
    );
    assert_eq!(
        anthropic["tools"][2]["input_schema"].get("additionalProperties"),
        None
    );

    let gemini = requests("050", Protocol::GeminiGenerateContent)[0]
        .body
        .json()
        .clone();
    assert_eq!(
        gemini["tools"][0]["functionDeclarations"][1]["parameters"].get("additionalProperties"),
        None
    );
}

fn tool_response(protocol: Protocol) -> Value {
    match protocol {
        Protocol::OpenAiChat => serde_json::json!({
            "id": "chatcmpl-fixture",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-chat-1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Beijing\"}"}
                    }]
                }
            }]
        }),
        Protocol::OpenAiResponses => serde_json::json!({
            "id": "resp-fixture",
            "object": "response",
            "output": [{
                "type": "function_call",
                "call_id": "call-responses-1",
                "name": "get_weather",
                "arguments": "{\"city\":\"Beijing\"}"
            }]
        }),
        Protocol::AnthropicMessages => serde_json::json!({
            "id": "msg-fixture",
            "type": "message",
            "content": [{
                "type": "tool_use",
                "id": "toolu-anthropic-1",
                "name": "get_weather",
                "input": {"city": "Beijing"}
            }]
        }),
        Protocol::GeminiGenerateContent => serde_json::json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "id": "call-gemini-1",
                            "name": "get_weather",
                            "args": {"city": "Beijing"}
                        }
                    }]
                }
            }]
        }),
        Protocol::OllamaChat => serde_json::json!({
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "function": {"name": "get_weather", "arguments": {"city": "Beijing"}}
                }]
            },
            "done": true
        }),
        Protocol::Unknown => unreachable!(),
    }
}

#[test]
fn tool_followups_round_trip_observed_assistant_turns_for_all_protocols() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::OllamaChat,
    ] {
        let initial = requests("048", protocol)[0].body.json().clone();
        let response = tool_response(protocol);
        let follow = build_follow_up(protocol, "fixture-model", "048", &initial, &response)
            .unwrap()
            .body;

        match protocol {
            Protocol::OpenAiChat => {
                assert_eq!(follow["messages"][1], response["choices"][0]["message"]);
                assert_eq!(follow["messages"][2]["tool_call_id"], "call-chat-1");
                assert_eq!(follow["messages"][2]["content"], "WEATHER_SUNNY");
            }
            Protocol::OpenAiResponses => {
                assert_eq!(follow["previous_response_id"], "resp-fixture");
                assert_eq!(follow["input"][0]["call_id"], "call-responses-1");
            }
            Protocol::AnthropicMessages => {
                assert_eq!(follow["messages"][1]["content"], response["content"]);
                assert_eq!(
                    follow["messages"][2]["content"][0]["tool_use_id"],
                    "toolu-anthropic-1"
                );
                assert_eq!(follow["max_tokens"], 2048);
            }
            Protocol::GeminiGenerateContent => {
                assert_eq!(follow["contents"][1], response["candidates"][0]["content"]);
                assert_eq!(
                    follow["contents"][2]["parts"][0]["functionResponse"]["id"],
                    "call-gemini-1"
                );
            }
            Protocol::OllamaChat => {
                assert_eq!(follow["messages"][1], response["message"]);
                assert_eq!(follow["messages"][2]["tool_name"], "get_weather");
            }
            Protocol::Unknown => unreachable!(),
        }
        assert_eq!(follow["tools"], initial["tools"], "{protocol}");
    }
}

#[test]
fn tool_failure_followups_use_protocol_native_error_shapes() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::OllamaChat,
    ] {
        let initial = requests("049", protocol)[0].body.json().clone();
        let follow = build_follow_up(
            protocol,
            "fixture-model",
            "049",
            &initial,
            &tool_response(protocol),
        )
        .unwrap()
        .body;
        let encoded = serde_json::to_string(&follow).unwrap();
        assert!(encoded.contains("timeout"), "{protocol}: {encoded}");
        if protocol == Protocol::AnthropicMessages {
            assert_eq!(follow["messages"][2]["content"][0]["is_error"], true);
        }
        if protocol == Protocol::GeminiGenerateContent {
            assert_eq!(
                follow["contents"][2]["parts"][0]["functionResponse"]["response"]["error"],
                "timeout"
            );
        }
    }
}

#[test]
fn malformed_tool_response_skips_followup() {
    let initial = requests("048", Protocol::OpenAiChat)[0].body.json().clone();
    assert!(
        build_follow_up(
            Protocol::OpenAiChat,
            "fixture-model",
            "048",
            &initial,
            &serde_json::json!({"choices": []}),
        )
        .is_err()
    );
}

#[test]
fn performance_checks_preserve_request_counts_streaming_and_shared_samples() {
    for id in ["051", "052", "054"] {
        let planned = requests(id, Protocol::OpenAiChat);
        assert_eq!(planned.len(), 1, "check {id}");
        assert!(!planned[0].stream);
        assert!(body_text(&planned[0].body).contains(&format!("MODEL_DOCTOR_CASE_{id}_OK")));
    }

    let stream = requests("053", Protocol::OpenAiChat);
    assert_eq!(stream.len(), 1);
    assert!(stream[0].stream);
    assert_eq!(stream[0].body.json()["stream"], true);

    let repeat_055 = plan("055", &context(Protocol::OpenAiChat)).unwrap();
    let repeat_056 = plan("056", &context(Protocol::OpenAiChat)).unwrap();
    assert_eq!(repeat_055.manifest_refs, ManifestRefs::SharedRepeatSamples);
    assert_eq!(repeat_056.manifest_refs, ManifestRefs::SharedRepeatSamples);
    let ids_055: Vec<&str> = repeat_055
        .groups
        .iter()
        .flat_map(RequestGroup::requests)
        .map(|request| request.id.as_str())
        .collect();
    let ids_056: Vec<&str> = repeat_056
        .groups
        .iter()
        .flat_map(RequestGroup::requests)
        .map(|request| request.id.as_str())
        .collect();
    assert_eq!(ids_055, ids_056);
    assert_eq!(ids_055.len(), 5);
    assert_eq!(ids_055[0], "test-055-repeat-1");
    assert_eq!(ids_055[4], "test-055-repeat-5");
}

#[test]
fn concurrency_check_has_four_simultaneous_batches_and_sixty_unique_ids() {
    let plan = plan("057", &context(Protocol::OpenAiChat)).unwrap();
    assert_eq!(plan.groups.len(), 4);
    let expected_sizes = [4, 8, 16, 32];
    let mut ids = std::collections::BTreeSet::new();
    for (group, expected_size) in plan.groups.iter().zip(expected_sizes) {
        let RequestGroup::Concurrent(requests) = group else {
            panic!("check 057 batch was not concurrent");
        };
        assert_eq!(requests.len(), expected_size);
        for request in requests {
            assert!(ids.insert(request.id.clone()), "duplicate {}", request.id);
            assert!(
                body_text(&request.body).contains(&format!("MODEL_DOCTOR_057_C{expected_size}_OK"))
            );
        }
    }
    assert_eq!(ids.len(), 60);
    assert!(ids.contains("test-057-c4-1"));
    assert!(ids.contains("test-057-c32-32"));
}

#[test]
fn sustained_check_has_ten_requests_followed_by_recovery() {
    let planned = requests("058", Protocol::OpenAiChat);
    assert_eq!(planned.len(), 11);
    for (index, request) in planned[..10].iter().enumerate() {
        assert_eq!(request.id, format!("test-058-repeat-{}", index + 1));
        assert!(body_text(&request.body).contains("MODEL_DOCTOR_CASE_058_OK"));
    }
    assert_eq!(planned[10].id, "test-058-recovery");
    assert!(body_text(&planned[10].body).contains("MODEL_DOCTOR_CASE_058_RECOVERY_OK"));
}

#[test]
fn guardrail_checks_preserve_control_experiment_and_authorization_prompts() {
    let unauthorized = requests("059", Protocol::OpenAiChat);
    assert_eq!(unauthorized.len(), 1);
    assert!(
        body_text(&unauthorized[0].body)
            .contains("damage a third-party system without authorization")
    );

    let defensive = requests("060", Protocol::OpenAiChat);
    let defensive_body = body_text(&defensive[0].body);
    assert!(defensive_body.contains("authorized defensive analysis"));
    assert!(defensive_body.contains("203.0.113.7"));
    assert!(defensive_body.contains("lock-account-and-review-auth-logs"));

    let chinese = requests("061", Protocol::OpenAiChat);
    assert_eq!(chinese.len(), 2);
    assert_eq!(chinese[0].id, "test-061-control");
    assert_eq!(chinese[1].id, "test-061-experiment");
    assert!(body_text(&chinese[1].body).contains("恶意分析、漏洞、告警、木马、攻击、威胁"));

    let english = requests("062", Protocol::OpenAiChat);
    assert_eq!(english.len(), 2);
    assert!(
        body_text(&english[1].body)
            .contains("malware, threat, payload, exploit, alert, vulnerability")
    );
}
