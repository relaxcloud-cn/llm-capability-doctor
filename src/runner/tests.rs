use std::time::Duration;

use base64::Engine as _;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::checks::Body;
use crate::cli::Cli;
use crate::evidence::{StreamEndSignal, StreamTermination, ToolContractStatus, ToolLoopOutcome};
use crate::protocol::tools::{tool_prompt, tool_request};
use crate::test_support::{
    RawTcpServer, ScriptedAssistantTurn, ServerAction, encode_official_stream,
};

#[test]
fn run_outcome_exposes_detected_connection_metadata() {
    let outcome = RunOutcome {
        duration: Duration::ZERO,
        request_count: 46,
        manifest_count: 46,
        log_path: std::path::PathBuf::from("doctor.log"),
        detected_protocol: Protocol::OpenAiChat,
        detected_auth_mode: AuthMode::Bearer,
    };

    assert_eq!(outcome.detected_protocol, Protocol::OpenAiChat);
    assert_eq!(outcome.detected_auth_mode, AuthMode::Bearer);
}

#[test]
fn collection_progress_includes_category_purpose_and_concurrency_wave() {
    let context = crate::catalog::CATALOG
        .iter()
        .find(|test| test.id == "018")
        .unwrap();
    let concurrency = crate::catalog::CATALOG
        .iter()
        .find(|test| test.id == "057")
        .unwrap();

    assert_eq!(
        collection_progress_line(14, 42, context, None),
        "[采集 14/42] 上下文能力 | 实测模型可用上下文 Token 上限"
    );
    assert_eq!(
        collection_progress_line(40, 42, concurrency, Some((4, 4, 32))),
        "[采集 40/42 | 并发 4/4] 性能与稳定性 | 测试模型在 32 并发下能否正常响应"
    );
}

#[tokio::test]
async fn ordinary_stream_request_records_official_terminal() {
    let body = openai_chat_final_stream("MODEL_DOCTOR_CASE_006_OK", true);
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_one(PlannedRequest {
            id: "test-006".into(),
            protocol: Protocol::OpenAiChat,
            auth_mode: AuthMode::None,
            body: Body::Json(serde_json::json!({
                "model": "test-model",
                "messages": [{"role": "user", "content": "test"}],
                "stream": true
            })),
            stream: true,
        })
        .await
        .expect("ordinary stream request");

    assert_eq!(evidence.stream_termination, StreamTermination::Completed);
    assert_eq!(evidence.stream_end_signal, StreamEndSignal::OpenAiDone);
    assert_eq!(evidence.model_stop_reason.as_deref(), Some("stop"));
    assert_eq!(evidence.stream_event_count, 3);
    assert_eq!(
        evidence.tool_contract_status,
        ToolContractStatus::NotApplicable
    );
    assert!(evidence.tool_contract_errors.is_empty());
    assert_eq!(evidence.tool_loop_turn, 0);
    assert_eq!(evidence.tool_loop_outcome, ToolLoopOutcome::NotApplicable);
}

#[tokio::test]
async fn ordinary_stream_request_records_missing_terminal() {
    let body = openai_chat_final_stream("MODEL_DOCTOR_CASE_006_OK", false);
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_one(ordinary_stream_request("test-006"))
        .await
        .expect("ordinary stream request");

    assert_eq!(
        evidence.stream_termination,
        StreamTermination::MissingTerminalEvent
    );
    assert_eq!(evidence.stream_end_signal, StreamEndSignal::None);
    assert_eq!(evidence.model_stop_reason.as_deref(), Some("stop"));
    assert_eq!(evidence.stream_event_count, 2);
}

#[tokio::test]
async fn interface_protocol_retries_transient_failure_and_uses_final_evidence() {
    let response = openai_completion_with_usage(2);
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Close],
        vec![ServerAction::Write(http_response(&response))],
    ])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    runner
        .execute_check(catalog_test("004"))
        .await
        .expect("interface check retry");

    assert_eq!(server.accepted_count(), 2);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("REQUEST test-004 BEGIN"));
    assert!(log.contains("REQUEST test-004-a2 BEGIN"));
    assert!(log.contains("request_refs: test-004-a2"));
}

#[tokio::test]
async fn interface_protocol_does_not_retry_explicit_client_error() {
    let body = br#"{"error":"unauthorized"}"#;
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(
        http_response_with_declared_length("401 Unauthorized", body.len(), body),
    )]])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    runner
        .execute_check(catalog_test("004"))
        .await
        .expect("interface check");

    assert_eq!(server.accepted_count(), 1);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(!log.contains("REQUEST test-004-a2 BEGIN"));
}

#[tokio::test]
async fn protocol_detection_retries_transient_probe_failure() {
    let response = openai_completion_with_usage(2);
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Close],
        vec![ServerAction::Write(http_response(&response))],
    ])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::Unknown, Duration::from_secs(1));

    runner.detect_protocol().await.expect("protocol detection");

    assert_eq!(runner.detected_protocol, Protocol::OpenAiChat);
    assert_eq!(runner.detected_auth_mode, AuthMode::Bearer);
    assert_eq!(server.accepted_count(), 2);
    assert_eq!(runner.protocol_probe_refs, vec!["protocol-1-a2"]);
}

#[tokio::test]
async fn concurrent_stream_requests_are_parsed_before_ordered_audit_write() {
    let body_a = openai_chat_final_stream("first", true);
    let body_b = openai_chat_final_stream("second", true);
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(&body_a))],
        vec![ServerAction::Write(http_response(&body_b))],
    ])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_concurrent(vec![
            ordinary_stream_request("concurrent-1"),
            ordinary_stream_request("concurrent-2"),
        ])
        .await
        .expect("concurrent stream requests");

    assert_eq!(
        evidence
            .iter()
            .map(|request| request.request_id.as_str())
            .collect::<Vec<_>>(),
        ["concurrent-1", "concurrent-2"]
    );
    assert!(
        evidence
            .iter()
            .all(|request| request.stream_termination == StreamTermination::Completed)
    );
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    let first = log
        .find("REQUEST concurrent-1 BEGIN")
        .expect("first request");
    let second = log
        .find("REQUEST concurrent-2 BEGIN")
        .expect("second request");
    assert!(first < second);
    assert_eq!(log.matches("stream_termination: completed").count(), 2);
}

#[tokio::test]
async fn ordinary_gemini_stream_uses_the_official_stream_endpoint() {
    let body = encode_official_stream(
        Protocol::GeminiGenerateContent,
        &ScriptedAssistantTurn::Final {
            response_id: "gemini-final".into(),
            text: "MODEL_DOCTOR_CASE_006_OK".into(),
        },
    );
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(
        &server,
        &temp,
        Protocol::GeminiGenerateContent,
        Duration::from_secs(1),
    );
    runner.config.url = server.url_with_path(
        "/v1/models/test:generateContent?key=fixture&&alt=json&trace=one%2Ftwo&alt=media",
    );

    let evidence = runner
        .execute_one(PlannedRequest {
            id: "test-006".into(),
            protocol: Protocol::GeminiGenerateContent,
            auth_mode: AuthMode::None,
            body: Body::Json(serde_json::json!({
                "contents": [{"role": "user", "parts": [{"text": "test"}]}]
            })),
            stream: true,
        })
        .await
        .expect("ordinary Gemini stream");

    assert_eq!(evidence.stream_termination, StreamTermination::Completed);
    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .head
            .starts_with("POST /v1/models/test:streamGenerateContent?")
    );
    assert_eq!(requests[0].head.matches("alt=sse").count(), 1);
    assert!(
        requests[0]
            .head
            .contains("POST /v1/models/test:streamGenerateContent?alt=sse HTTP/1.1")
    );
    assert!(!requests[0].head.contains("key=fixture"));
    assert!(!requests[0].head.contains("trace="));
}

#[tokio::test]
async fn independent_stream_encoder_is_accepted_by_all_parsers() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::OllamaChat,
    ] {
        for turn in [
            scripted_tool(
                "046",
                1,
                "get_weather",
                serde_json::json!({"city": "Beijing"}),
            ),
            ScriptedAssistantTurn::Final {
                response_id: "response-046-final".into(),
                text: "MODEL_DOCTOR_CASE_046_OK".into(),
            },
        ] {
            let parsed = crate::protocol::stream::parse_stream(
                protocol,
                &encode_official_stream(protocol, &turn),
            )
            .await;
            assert_eq!(
                parsed.stream_termination,
                StreamTermination::Completed,
                "{protocol:?} {turn:?}: {:?}",
                parsed.contract_errors
            );
            assert!(
                parsed.contract_errors.is_empty(),
                "{protocol:?} {turn:?}: {:?}",
                parsed.contract_errors
            );
            assert!(parsed.assistant_turn.is_some(), "{protocol:?} {turn:?}");
        }
    }
}

#[tokio::test]
async fn official_tool_loop_matrix_completes_all_protocols_and_checks() {
    for protocol in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::OllamaChat,
    ] {
        for check_id in ["046", "047", "048", "049"] {
            let turns = successful_turns(protocol, check_id);
            let scripts = turns
                .iter()
                .map(|turn| {
                    vec![ServerAction::Write(http_response(&encode_official_stream(
                        protocol, turn,
                    )))]
                })
                .collect();
            let server = RawTcpServer::spawn(scripts).await;
            let temp = TempDir::new().expect("temp directory");
            let mut runner = test_runner(&server, &temp, protocol, Duration::from_secs(1));
            if protocol == Protocol::GeminiGenerateContent {
                runner.config.url =
                    server.url_with_path("/v1/models/test:generateContent?key=fixture&alt=json");
            } else if protocol == Protocol::OllamaChat {
                runner.config.url = server.url_with_path("/api/chat");
            }
            let spec = tool_request(
                protocol,
                "test-model",
                check_id,
                tool_prompt(check_id).expect("tool-loop prompt"),
            );
            let initial = PlannedRequest {
                id: format!("test-{check_id}"),
                protocol,
                auth_mode: AuthMode::None,
                body: Body::Json(spec.body),
                stream: spec.stream,
            };
            let test = crate::catalog::CATALOG
                .iter()
                .find(|test| test.id == check_id)
                .expect("catalog tool-loop check");

            let evidence = runner
                .execute_tool_loop(test, initial)
                .await
                .expect("complete official tool loop");

            let expected_count = match check_id {
                "046" | "048" => 2,
                "047" | "049" => 3,
                _ => unreachable!(),
            };
            assert_eq!(evidence.len(), expected_count, "{protocol:?} {check_id}");
            assert_eq!(
                evidence
                    .iter()
                    .map(|request| request.request_id.clone())
                    .collect::<Vec<_>>(),
                (1..=expected_count)
                    .map(|turn| format!("test-{check_id}-turn-{turn}"))
                    .collect::<Vec<_>>(),
                "{protocol:?} {check_id}"
            );
            assert!(
                evidence.iter().all(|request| {
                    request.stream_termination == StreamTermination::Completed
                        && request.tool_contract_status == ToolContractStatus::Conformant
                }),
                "{protocol:?} {check_id}: {:#?}",
                evidence
                    .iter()
                    .map(|request| (&request.stream_termination, &request.tool_contract_errors))
                    .collect::<Vec<_>>()
            );
            assert!(
                evidence[..evidence.len() - 1]
                    .iter()
                    .all(|request| request.tool_loop_outcome == ToolLoopOutcome::Continued)
            );
            assert_eq!(
                evidence.last().expect("final turn").tool_loop_outcome,
                ToolLoopOutcome::Completed
            );
            let final_body =
                String::from_utf8_lossy(&evidence.last().expect("final turn").response_body);
            assert!(
                final_body.contains(&format!("MODEL_DOCTOR_CASE_{check_id}_OK")),
                "{protocol:?} {check_id}: {final_body}"
            );
            if check_id == "048" {
                assert!(final_body.contains("WEATHER_SUNNY"));
            }

            let requests = server.recorded_requests().await;
            assert_eq!(requests.len(), expected_count);
            let request_bodies = requests
                .iter()
                .map(|request| String::from_utf8_lossy(&request.body).into_owned())
                .collect::<Vec<_>>();
            if check_id == "047" {
                assert!(request_bodies[1].contains("WEATHER_SUNNY"));
                assert!(request_bodies[2].contains("TIME_UTC_12:00"));
            }
            if check_id == "049" {
                assert_eq!(
                    timeout_result_count(&request_bodies[1]),
                    1,
                    "{protocol:?}: {}",
                    request_bodies[1]
                );
                assert_eq!(request_bodies[2].matches("WEATHER_SUNNY").count(), 1);
                assert!(timeout_result_count(&request_bodies[2]) <= 1);
            }
            if protocol == Protocol::GeminiGenerateContent {
                assert!(requests.iter().all(|request| {
                    request
                        .head
                        .starts_with("POST /v1/models/test:streamGenerateContent?")
                        && request.head.matches("alt=sse").count() == 1
                }));
            }
            if protocol == Protocol::OllamaChat {
                assert!(
                    requests
                        .iter()
                        .all(|request| request.head.starts_with("POST /api/chat "))
                );
            }
        }
    }
}

#[tokio::test]
async fn malformed_tool_arguments_stop_without_a_follow_up() {
    let turn = scripted_tool(
        "046",
        1,
        "get_weather",
        serde_json::json!({"city": "Shanghai"}),
    );
    let body = encode_official_stream(Protocol::OpenAiChat, &turn);
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("invalid turn is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(evidence[0].stream_termination, StreamTermination::Completed);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert_eq!(
        evidence[0].tool_contract_errors,
        ["tool_loop.invalid_arguments:/tool_calls/0/arguments"]
    );
}

#[tokio::test]
async fn malformed_argument_json_is_non_conformant_even_with_official_terminal() {
    let content = serde_json::json!({
        "id": "chatcmpl-malformed-arguments",
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "delta": {
                "role": "assistant",
                "tool_calls": [{
                    "index": 0,
                    "id": "call-malformed",
                    "type": "function",
                    "function": {"name": "get_weather", "arguments": "{"}
                }]
            },
            "finish_reason": null
        }]
    });
    let terminal = serde_json::json!({
        "id": "chatcmpl-malformed-arguments",
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
    });
    let body = format!("data: {content}\n\ndata: {terminal}\n\ndata: [DONE]\n\n").into_bytes();
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("malformed arguments are an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].stream_termination, StreamTermination::Completed);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert!(
        evidence[0]
            .tool_contract_errors
            .iter()
            .any(|error| error.starts_with("chat.invalid_arguments:"))
    );
}

#[tokio::test]
async fn follow_up_builder_failure_is_attached_to_the_current_turn() {
    let body = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));
    runner.forced_follow_up_errors = Some(vec!["request.test.builder_failure:/".into()]);

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("builder failure is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(evidence[0].stream_termination, StreamTermination::Completed);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert_eq!(
        evidence[0].tool_contract_errors,
        ["request.test.builder_failure:/"]
    );
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("REQUEST test-046-turn-1 BEGIN"));
    assert!(!log.contains("REQUEST test-046-turn-2 BEGIN"));
}

#[tokio::test]
async fn follow_up_validation_failure_is_attached_to_the_current_turn() {
    let body = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));
    runner.force_invalid_follow_up = true;

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("validation failure is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert_eq!(
        evidence[0].tool_contract_errors,
        ["request.openai_chat.stream_required:/stream"]
    );
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("REQUEST test-046-turn-1 BEGIN"));
    assert!(!log.contains("REQUEST test-046-turn-2 BEGIN"));
}

#[tokio::test]
async fn missing_terminal_stops_without_executing_a_tool() {
    let complete = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let body = String::from_utf8(complete)
        .expect("UTF-8 fixture")
        .replace("data: [DONE]\n\n", "")
        .into_bytes();
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&body))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("missing terminal is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(
        evidence[0].stream_termination,
        StreamTermination::MissingTerminalEvent
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert!(
        evidence[0]
            .tool_contract_errors
            .contains(&"response.missing_terminal_event:/".to_owned())
    );
}

#[tokio::test]
async fn empty_tool_stream_has_stable_non_conformant_errors() {
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(http_response(&[]))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("empty stream is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(
        evidence[0].tool_contract_status,
        ToolContractStatus::NonConformant
    );
    assert_eq!(evidence[0].tool_loop_outcome, ToolLoopOutcome::InvalidTurn);
    assert!(
        evidence[0]
            .tool_contract_errors
            .contains(&"response.unavailable:/".to_owned())
    );
    assert!(
        evidence[0]
            .tool_contract_errors
            .contains(&"response.missing_terminal_event:/".to_owned())
    );
}

#[tokio::test]
async fn upstream_disconnect_audits_partial_turn_without_follow_up() {
    let body = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let partial = &body[..body.len() / 2];
    let server = RawTcpServer::spawn(vec![vec![
        ServerAction::Write(http_response_with_declared_length(
            "200 OK",
            body.len(),
            partial,
        )),
        ServerAction::Close,
    ]])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("disconnect is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(
        evidence[0].stream_termination,
        StreamTermination::UpstreamDisconnect
    );
    assert_eq!(
        evidence[0].tool_loop_outcome,
        ToolLoopOutcome::TransportFailure
    );
    assert_eq!(evidence[0].tool_contract_errors, ["response.unavailable:/"]);
}

#[tokio::test]
async fn real_http_timeout_does_not_trigger_check_049_retry() {
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Delay(Duration::from_secs(1))]]).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(
        &server,
        &temp,
        Protocol::OpenAiChat,
        Duration::from_millis(40),
    );

    let evidence = runner
        .execute_tool_loop(
            catalog_test("049"),
            initial_tool_request(Protocol::OpenAiChat, "049"),
        )
        .await
        .expect("timeout is an audited outcome");

    assert_eq!(evidence.len(), 1);
    assert_eq!(server.accepted_count(), 1);
    assert_eq!(evidence[0].stream_termination, StreamTermination::Timeout);
    assert_eq!(
        evidence[0].tool_loop_outcome,
        ToolLoopOutcome::TransportFailure
    );
    assert_eq!(evidence[0].tool_contract_errors, ["response.unavailable:/"]);
    assert!(!evidence[0].body.contains("ERROR: timeout"));
}

#[tokio::test]
async fn client_cancellation_during_tool_http_flushes_partial_evidence() {
    let checkpoint = std::sync::Arc::new(Notify::new());
    let partial = b"data: {\"partial\":true}\n\n";
    let server = RawTcpServer::spawn(vec![vec![
        ServerAction::Write(http_response_with_declared_length("200 OK", 1_000, partial)),
        ServerAction::Delay(Duration::from_millis(100)),
        ServerAction::Checkpoint(checkpoint.clone()),
        ServerAction::Delay(Duration::from_secs(2)),
    ]])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(3));
    let cancellation = runner.cancellation.clone();
    let task = tokio::spawn(async move {
        let mut runner = runner;
        runner
            .execute_tool_loop(
                catalog_test("046"),
                initial_tool_request(Protocol::OpenAiChat, "046"),
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), checkpoint.notified())
        .await
        .expect("partial tool response sent");
    cancellation.cancel();
    let result = task.await.expect("tool runner task");

    assert!(matches!(result, Err(RunnerError::Interrupted)));
    assert_eq!(server.accepted_count(), 1);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("REQUEST test-046-turn-1 BEGIN"));
    assert!(log.contains("stream_termination: client_cancelled"));
    assert!(log.contains("tool_contract_status: non_conformant"));
    assert!(log.contains("tool_loop_outcome: transport_failure"));
    assert!(log.contains(r#"tool_contract_errors_json: ["response.unavailable:/"]"#));
    assert!(log.contains(&base64::engine::general_purpose::STANDARD.encode(partial)));
    assert!(!log.contains("REQUEST test-046-turn-2 BEGIN"));
}

#[tokio::test]
async fn non_success_http_response_is_not_parsed_as_an_official_stream() {
    let body = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let server = RawTcpServer::spawn(vec![vec![ServerAction::Write(
        http_response_with_declared_length("500 Internal Server Error", body.len(), &body),
    )]])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("HTTP error is an audited outcome");

    assert_eq!(evidence[0].stream_termination, StreamTermination::HttpError);
    assert_eq!(evidence[0].stream_event_count, 0);
    assert_eq!(
        evidence[0].tool_loop_outcome,
        ToolLoopOutcome::TransportFailure
    );
    assert_eq!(evidence[0].tool_contract_errors, ["response.unavailable:/"]);
}

#[tokio::test]
async fn four_consecutive_tool_turns_stop_at_the_audited_bound() {
    let turns = (1..=4)
        .map(|turn| {
            scripted_tool(
                "046",
                turn,
                "get_weather",
                serde_json::json!({"city": "Beijing"}),
            )
        })
        .collect::<Vec<_>>();
    let scripts = turns
        .iter()
        .map(|turn| {
            vec![ServerAction::Write(http_response(&encode_official_stream(
                Protocol::OpenAiChat,
                turn,
            )))]
        })
        .collect();
    let server = RawTcpServer::spawn(scripts).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    let evidence = runner
        .execute_tool_loop(
            catalog_test("046"),
            initial_tool_request(Protocol::OpenAiChat, "046"),
        )
        .await
        .expect("max turns is an audited outcome");

    assert_eq!(evidence.len(), 4);
    assert_eq!(server.accepted_count(), 4);
    assert_eq!(
        evidence.last().expect("fourth turn").request_id,
        "test-046-turn-4"
    );
    assert_eq!(
        evidence.last().expect("fourth turn").tool_loop_outcome,
        ToolLoopOutcome::MaxTurnsExceeded
    );
    assert_eq!(
        evidence.last().expect("fourth turn").tool_contract_status,
        ToolContractStatus::Conformant
    );
}

#[tokio::test]
async fn cancel_between_completed_turns_does_not_send_an_extra_turn() {
    let first = encode_official_stream(
        Protocol::OpenAiChat,
        &scripted_tool(
            "046",
            1,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ),
    );
    let final_turn = encode_official_stream(
        Protocol::OpenAiChat,
        &ScriptedAssistantTurn::Final {
            response_id: "response-final".into(),
            text: "MODEL_DOCTOR_CASE_046_OK".into(),
        },
    );
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(&first))],
        vec![ServerAction::Write(http_response(&final_turn))],
    ])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));
    let turn_committed = std::sync::Arc::new(Notify::new());
    let resume = std::sync::Arc::new(Notify::new());
    runner.turn_hook = Some(TurnHook {
        turn_committed: turn_committed.clone(),
        resume: resume.clone(),
    });
    let cancellation = runner.cancellation.clone();
    let task = tokio::spawn(async move {
        runner
            .execute_tool_loop(
                catalog_test("046"),
                initial_tool_request(Protocol::OpenAiChat, "046"),
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), turn_committed.notified())
        .await
        .expect("first turn committed");
    cancellation.cancel();
    resume.notify_one();
    let result = task.await.expect("runner task");

    assert!(matches!(result, Err(RunnerError::Interrupted)));
    assert_eq!(server.accepted_count(), 1);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("REQUEST test-046-turn-1 BEGIN"));
    assert!(!log.contains("REQUEST test-046-turn-2 BEGIN"));
}

#[tokio::test]
async fn cancelled_http_request_is_flushed_without_run_summary() {
    let checkpoint = std::sync::Arc::new(Notify::new());
    let partial = b"partial-response";
    let server = RawTcpServer::spawn(vec![vec![
        ServerAction::Write(http_response_with_declared_length("200 OK", 1_000, partial)),
        ServerAction::Delay(Duration::from_millis(100)),
        ServerAction::Checkpoint(checkpoint.clone()),
        ServerAction::Delay(Duration::from_secs(2)),
    ]])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let log_path = temp.path().join("cancelled.log");
    let config = test_config(server.url(), &log_path, Duration::from_secs(3));
    let cancellation = CancellationToken::new();
    let task = tokio::spawn({
        let cancellation = cancellation.clone();
        async move { run(config, cancellation).await }
    });

    tokio::time::timeout(Duration::from_secs(1), checkpoint.notified())
        .await
        .expect("partial response sent");
    cancellation.cancel();
    let result = task.await.expect("run task");

    assert!(matches!(result, Err(RunnerError::Interrupted)));
    assert_eq!(server.accepted_count(), 1);
    let log = std::fs::read_to_string(&log_path).expect("cancelled audit log");
    assert!(log.contains("REQUEST test-001 BEGIN"));
    assert!(log.contains("transport_outcome: client_cancelled"));
    assert!(log.contains("stream_termination: client_cancelled"));
    assert!(log.contains(&base64::engine::general_purpose::STANDARD.encode(partial)));
    assert!(!log.contains("TEST-001 BEGIN"));
    assert!(!log.contains("RUN SUMMARY"));
}

#[tokio::test]
async fn unknown_protocol_reuses_probe_evidence_without_sending_tool_seed() {
    let response = br#"{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"probe\"},\"finish_reason\":\"stop\"}]}"#;
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(response))],
        vec![ServerAction::Write(http_response(response))],
    ])
    .await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::Unknown, Duration::from_secs(1));
    for index in 1..=2 {
        let evidence = runner
            .execute_one(PlannedRequest {
                id: format!("protocol-{index}"),
                protocol: Protocol::OpenAiChat,
                auth_mode: AuthMode::None,
                body: Body::Json(serde_json::json!({"probe": index})),
                stream: false,
            })
            .await
            .expect("probe evidence");
        runner.protocol_probe_refs.push(evidence.request_id);
    }

    runner
        .execute_check(catalog_test("046"))
        .await
        .expect("unknown protocol manifest");

    assert_eq!(server.accepted_count(), 2);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("request_refs: protocol-1,protocol-2"));
    assert!(!log.contains("test-046-turn-1"));
}

#[tokio::test]
async fn execute_check_manifests_every_completed_tool_turn_in_order() {
    let turns = successful_turns(Protocol::OpenAiChat, "047");
    let scripts = turns
        .iter()
        .map(|turn| {
            vec![ServerAction::Write(http_response(&encode_official_stream(
                Protocol::OpenAiChat,
                turn,
            )))]
        })
        .collect();
    let server = RawTcpServer::spawn(scripts).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::OpenAiChat, Duration::from_secs(1));

    runner
        .execute_check(catalog_test("047"))
        .await
        .expect("tool-loop check");

    assert_eq!(server.accepted_count(), 3);
    let log = std::fs::read_to_string(temp.path().join("audit.log")).expect("audit log");
    assert!(log.contains("request_refs: test-047-turn-1,test-047-turn-2,test-047-turn-3"));
}

#[tokio::test]
async fn protocol_detection_requires_a_successful_http_status() {
    let body = br#"{\"choices\":[{\"message\":{\"role\":\"assistant\",\"content\":\"MODEL_DOCTOR_PROTOCOL_OK\"},\"finish_reason\":\"stop\"}]}"#;
    let scripts = (0..crate::protocol::PROBE_CANDIDATES.len()
        * (MAX_INTERFACE_PROTOCOL_RETRIES + 1))
        .map(|_| {
            vec![ServerAction::Write(http_response_with_declared_length(
                "500 Internal Server Error",
                body.len(),
                body,
            ))]
        })
        .collect();
    let server = RawTcpServer::spawn(scripts).await;
    let temp = TempDir::new().expect("temp directory");
    let mut runner = test_runner(&server, &temp, Protocol::Unknown, Duration::from_secs(1));

    runner.detect_protocol().await.expect("protocol probes");

    assert_eq!(runner.detected_protocol, Protocol::Unknown);
    assert_eq!(
        runner.protocol_probe_refs.len(),
        crate::protocol::PROBE_CANDIDATES.len()
    );
    assert_eq!(
        server.accepted_count(),
        crate::protocol::PROBE_CANDIDATES.len() * (MAX_INTERFACE_PROTOCOL_RETRIES + 1)
    );
}

fn successful_turns(protocol: Protocol, check_id: &str) -> Vec<ScriptedAssistantTurn> {
    let weather_name = crate::protocol::tools::expected_weather_call_name(protocol, check_id);
    let mut turns = vec![scripted_tool(
        check_id,
        1,
        weather_name,
        serde_json::json!({"city": "Beijing"}),
    )];
    if check_id == "047" {
        turns.push(scripted_tool(
            check_id,
            2,
            "get_time",
            serde_json::json!({"zone": "UTC"}),
        ));
    } else if check_id == "049" {
        turns.push(scripted_tool(
            check_id,
            2,
            "get_weather",
            serde_json::json!({"city": "Beijing"}),
        ));
    }
    let text = if check_id == "048" {
        "MODEL_DOCTOR_CASE_048_OK WEATHER_SUNNY".to_owned()
    } else {
        format!("MODEL_DOCTOR_CASE_{check_id}_OK")
    };
    turns.push(ScriptedAssistantTurn::Final {
        response_id: format!("response-{check_id}-final"),
        text,
    });
    turns
}

fn scripted_tool(
    check_id: &str,
    turn: usize,
    name: &str,
    arguments: serde_json::Value,
) -> ScriptedAssistantTurn {
    ScriptedAssistantTurn::Tool {
        response_id: format!("response-{check_id}-{turn}"),
        call_id: Some(format!("call-{check_id}-{turn}")),
        name: name.into(),
        arguments,
    }
}

fn timeout_result_count(body: &str) -> usize {
    body.matches("ERROR: timeout").count() + body.matches(r#""error":"timeout""#).count()
}

fn ordinary_stream_request(id: &str) -> PlannedRequest {
    PlannedRequest {
        id: id.into(),
        protocol: Protocol::OpenAiChat,
        auth_mode: AuthMode::None,
        body: Body::Json(serde_json::json!({
            "model": "test-model",
            "messages": [{"role": "user", "content": "test"}],
            "stream": true
        })),
        stream: true,
    }
}

fn catalog_test(id: &str) -> &'static TestCase {
    crate::catalog::CATALOG
        .iter()
        .find(|test| test.id == id)
        .expect("catalog test")
}

fn initial_tool_request(protocol: Protocol, check_id: &str) -> PlannedRequest {
    let spec = tool_request(
        protocol,
        "test-model",
        check_id,
        tool_prompt(check_id).expect("tool-loop prompt"),
    );
    PlannedRequest {
        id: format!("test-{check_id}"),
        protocol,
        auth_mode: AuthMode::None,
        body: Body::Json(spec.body),
        stream: spec.stream,
    }
}

fn test_runner(
    server: &RawTcpServer,
    temp: &TempDir,
    protocol: Protocol,
    timeout: Duration,
) -> Runner {
    let log_path = temp.path().join("audit.log");
    let config = test_config(server.url(), &log_path, timeout);
    let audit = AuditWriter::create(
        &log_path,
        RunMetadata {
            run_id: "test-run".into(),
            started_at: Local::now(),
            url: config.url.clone(),
            model: config.model.clone(),
            masked_api_key: "***".into(),
            selected_test_count: 1,
            insecure: false,
        },
        Redactor::new(config.api_key.expose(), &config.url),
    )
    .expect("audit writer");
    Runner {
        config,
        selected: Vec::new(),
        http: HttpExecutor::new(timeout, false).expect("HTTP executor"),
        audit,
        cancellation: CancellationToken::new(),
        protocol_checked: true,
        detected_protocol: protocol,
        detected_auth_mode: AuthMode::None,
        protocol_probe_refs: Vec::new(),
        selected_probe_ref: None,
        repeat_refs: Vec::new(),
        turn_hook: None,
        forced_follow_up_errors: None,
        force_invalid_follow_up: false,
    }
}

fn test_config(url: url::Url, log_path: &std::path::Path, timeout: Duration) -> Config {
    Cli {
        url: Some(url),
        model: Some("test-model".into()),
        api_key: Some("test-key".into()),
        log_file: Some(log_path.to_owned()),
        timeout: timeout.as_secs().max(1),
        list_tests: false,
        insecure: false,
        self_analyze: false,
        llm_gateway_compatibility: false,
    }
    .into_config(None)
    .expect("test config")
}

fn http_response(body: &[u8]) -> Vec<u8> {
    http_response_with_declared_length("200 OK", body.len(), body)
}

fn http_response_with_declared_length(status: &str, content_length: usize, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

fn openai_chat_final_stream(text: &str, include_done: bool) -> Vec<u8> {
    let first = serde_json::json!({
        "id": "chatcmpl-final",
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant", "content": text},
            "finish_reason": null
        }]
    });
    let terminal = serde_json::json!({
        "id": "chatcmpl-final",
        "object": "chat.completion.chunk",
        "created": 1_787_041_140_u64,
        "model": "test-model",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    });
    let mut body = format!("data: {first}\n\ndata: {terminal}\n\n").into_bytes();
    if include_done {
        body.extend_from_slice(b"data: [DONE]\n\n");
    }
    body
}

fn openai_completion_with_usage(prompt_tokens: u64) -> Vec<u8> {
    serde_json::json!({
        "id": "chatcmpl-context",
        "object": "chat.completion",
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": 2, "total_tokens": prompt_tokens + 2}
    })
    .to_string()
    .into_bytes()
}

fn context_too_long_response() -> Vec<u8> {
    serde_json::json!({
        "error": {
            "message": "This model's maximum context length is 393216 tokens. However, you requested more tokens.",
            "type": "invalid_request_error",
            "code": "context_length_exceeded"
        }
    })
    .to_string()
    .into_bytes()
}

#[tokio::test]
async fn context_capacity_probes_calibrate_density_then_double_upward() {
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(10_000),
        ))],
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(124_000),
        ))],
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(248_000),
        ))],
        vec![ServerAction::Write(http_response_with_declared_length(
            "400 Bad Request",
            context_too_long_response().len(),
            &context_too_long_response(),
        ))],
    ])
    .await;
    let temp = TempDir::new().expect("tempdir");
    let mut runner = test_runner(
        &server,
        &temp,
        Protocol::OpenAiChat,
        Duration::from_secs(30),
    );

    runner
        .execute_check(catalog_test("018"))
        .await
        .expect("context capacity check");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 4, "calibration + 3 capacity probes");

    let calibration = String::from_utf8_lossy(&requests[0].body);
    assert!(calibration.contains("MODEL_DOCTOR_CONTEXT_CALIBRATION"));

    // 主探针按校准密度构造（40184 字符 / 10000 token ≈ 4.02），而不是旧的 4 字符假设。
    let main_probe = String::from_utf8_lossy(&requests[1].body);
    assert!(main_probe.contains("MODEL_DOCTOR_CONTEXT_CAPACITY"));
    assert!(main_probe.contains("\"max_tokens\":16"));
    assert!(
        main_probe.len() > 480_000 && main_probe.len() < 520_000,
        "main probe size {} did not follow measured density",
        main_probe.len()
    );

    let upward = String::from_utf8_lossy(&requests[2].body);
    assert!(
        upward.len() > 950_000,
        "upward probe size {} did not double",
        upward.len()
    );

    let log = std::fs::read_to_string(temp.path().join("audit.log")).unwrap();
    for id in [
        "test-018-calibration",
        "test-018-probe-124000-a1",
        "test-018-probe-248000-a1",
        "test-018-probe-496000-a1",
    ] {
        assert!(
            log.contains(&format!("========== REQUEST {id} BEGIN ==========")),
            "missing {id}"
        );
    }
    assert!(log.contains(
        "request_refs: test-018-calibration,test-018-probe-124000-a1,test-018-probe-248000-a1,test-018-probe-496000-a1"
    ));
}

#[tokio::test]
async fn context_capacity_probes_bisect_downward_after_rejection() {
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(10_000),
        ))],
        vec![ServerAction::Write(http_response_with_declared_length(
            "400 Bad Request",
            context_too_long_response().len(),
            &context_too_long_response(),
        ))],
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(67_000),
        ))],
        vec![ServerAction::Write(http_response_with_declared_length(
            "400 Bad Request",
            context_too_long_response().len(),
            &context_too_long_response(),
        ))],
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(81_250),
        ))],
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(88_375),
        ))],
    ])
    .await;
    let temp = TempDir::new().expect("tempdir");
    let mut runner = test_runner(
        &server,
        &temp,
        Protocol::OpenAiChat,
        Duration::from_secs(30),
    );

    runner
        .execute_check(catalog_test("018"))
        .await
        .expect("context capacity check");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 6, "calibration + five bisection probes");

    let log = std::fs::read_to_string(temp.path().join("audit.log")).unwrap();
    for id in [
        "test-018-probe-124000-a1",
        "test-018-probe-67000-a1",
        "test-018-probe-95500-a1",
        "test-018-probe-81250-a1",
        "test-018-probe-88375-a1",
    ] {
        assert!(
            log.contains(&format!("========== REQUEST {id} BEGIN ==========")),
            "missing {id}"
        );
    }
}

#[tokio::test]
async fn context_capacity_stops_immediately_when_declared_limit_fails_the_bar() {
    // 服务端在报错里声明上限 96K（不足 128K 档）：主探针被拒后无需夹逼。
    let rejection = serde_json::json!({
        "error": {
            "message": "This model's maximum context length is 96000 tokens. However, you requested 124016 tokens",
            "type": "invalid_request_error",
            "code": "context_length_exceeded"
        }
    })
    .to_string()
    .into_bytes();
    let server = RawTcpServer::spawn(vec![
        vec![ServerAction::Write(http_response(
            &openai_completion_with_usage(10_000),
        ))],
        vec![ServerAction::Write(http_response_with_declared_length(
            "400 Bad Request",
            rejection.len(),
            &rejection,
        ))],
    ])
    .await;
    let temp = TempDir::new().expect("tempdir");
    let mut runner = test_runner(
        &server,
        &temp,
        Protocol::OpenAiChat,
        Duration::from_secs(30),
    );

    runner
        .execute_check(catalog_test("018"))
        .await
        .expect("context capacity check");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2, "calibration + rejected main probe only");
    let log = std::fs::read_to_string(temp.path().join("audit.log")).unwrap();
    assert!(log.contains("========== REQUEST test-018-probe-124000-a1 BEGIN =========="));
    assert!(!log.contains("test-018-probe-6"));
}
