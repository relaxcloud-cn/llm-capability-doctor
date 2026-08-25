use std::time::Duration;

use thiserror::Error;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::checks::Body;
use crate::http::{HttpExecutor, RequestInput};
use crate::opencodex::contract::{Adapter, rule};
use crate::opencodex::evaluator::{
    AdapterResult, evaluate_http_failure, evaluate_shape_failure, evaluate_stream,
};
use crate::protocol::stream::parse_stream;
use crate::protocol::tool_loop::{LoopDecision, ToolLoopState};
use crate::protocol::tools::{ToolConversation, tool_prompt, tool_request};
use crate::protocol::{
    AuthMode, Protocol, RequestSpec, basic_request, matches_response, normalize_request_url,
};

pub struct OpenCodexSettings {
    pub url: Url,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
    pub insecure: bool,
    pub cancellation: CancellationToken,
}

pub struct OpenCodexOutcome {
    pub results: Vec<AdapterResult>,
}

impl OpenCodexOutcome {
    pub fn result(&self, adapter: Adapter) -> &AdapterResult {
        self.results
            .iter()
            .find(|result| result.adapter == adapter)
            .expect("the LLM gateway runner always evaluates every selected adapter")
    }
}

#[derive(Debug, Error)]
pub enum OpenCodexError {
    #[error("unable to initialize HTTP client: {0}")]
    Http(#[from] reqwest::Error),
}

pub async fn run(settings: OpenCodexSettings) -> Result<OpenCodexOutcome, OpenCodexError> {
    let executor = HttpExecutor::new(settings.timeout, settings.insecure)?;
    let mut results = Vec::with_capacity(3);
    for adapter in [Adapter::OpenAiChat, Adapter::Anthropic, Adapter::Google] {
        let mut result = merge_results(
            probe_response(adapter, &settings, &executor).await,
            probe_stream(adapter, &settings, &executor).await,
        );
        if result.passed {
            result = merge_results(
                result,
                probe_tool_loop(adapter, "046", &settings, &executor).await,
            );
        }
        if result.passed {
            result = merge_results(
                result,
                probe_tool_loop(adapter, "047", &settings, &executor).await,
            );
        }
        results.push(result);
    }
    Ok(OpenCodexOutcome { results })
}

async fn probe_response(
    adapter: Adapter,
    settings: &OpenCodexSettings,
    executor: &HttpExecutor,
) -> AdapterResult {
    let (protocol, auth_mode) = adapter_connection(adapter);
    let spec = basic_request(
        protocol,
        &settings.model,
        "Reply only MODEL_DOCTOR_LLM_GATEWAY_RESPONSE_OK",
        false,
    );
    let evidence = execute_spec(
        adapter, "response", protocol, auth_mode, spec, settings, executor,
    )
    .await;
    let successful_status = evidence
        .metrics
        .http_status
        .is_some_and(|status| (200..300).contains(&status));
    if !successful_status || !evidence.transport_outcome.is_success() {
        return evaluate_http_failure(adapter, evidence.metrics.http_status, &evidence.error);
    }
    if matches_response(protocol, &evidence.response_body) {
        pass_result(adapter)
    } else {
        evaluate_shape_failure(
            adapter,
            "response_body",
            &format!(
                "response did not match the {} response structure",
                adapter_display_name(adapter)
            ),
        )
    }
}

async fn probe_stream(
    adapter: Adapter,
    settings: &OpenCodexSettings,
    executor: &HttpExecutor,
) -> AdapterResult {
    let (protocol, auth_mode) = adapter_connection(adapter);
    let spec = basic_request(
        protocol,
        &settings.model,
        "Reply only MODEL_DOCTOR_LLM_GATEWAY_STREAM_OK",
        true,
    );
    let evidence = execute_spec(
        adapter, "stream", protocol, auth_mode, spec, settings, executor,
    )
    .await;

    evaluate_evidence(adapter, protocol, &evidence).await
}

async fn probe_tool_loop(
    adapter: Adapter,
    check_id: &'static str,
    settings: &OpenCodexSettings,
    executor: &HttpExecutor,
) -> AdapterResult {
    let (protocol, auth_mode) = adapter_connection(adapter);
    let prompt = tool_prompt(check_id).expect("LLM gateway probes use known tool checks");
    let initial = tool_request(protocol, &settings.model, check_id, prompt);
    let mut conversation = match ToolConversation::from_initial(protocol, initial.body.clone()) {
        Ok(conversation) => conversation,
        Err(error) => return tool_failure(adapter, check_id, "request", error.to_string()),
    };
    let mut state = ToolLoopState::for_protocol(check_id, protocol)
        .expect("LLM gateway probes only use known protocols and tool checks");
    let mut current = initial;
    let mut result = pass_result(adapter);

    for turn in 1..=crate::protocol::tool_loop::MAX_ASSISTANT_TURNS {
        let evidence = execute_spec(
            adapter,
            &format!("tool-{check_id}-turn-{turn}"),
            protocol,
            auth_mode,
            current,
            settings,
            executor,
        )
        .await;
        let turn_result = evaluate_evidence(adapter, protocol, &evidence).await;
        result = merge_results(result, turn_result);
        if !result.passed {
            return result;
        }

        let parsed = parse_stream(protocol, &evidence.response_body).await;
        let Some(assistant_turn) = parsed.assistant_turn else {
            return merge_results(
                result,
                tool_failure(
                    adapter,
                    check_id,
                    "assistant_turn",
                    "response contained no assistant turn",
                ),
            );
        };
        match state.advance(&assistant_turn) {
            LoopDecision::Complete => {
                let marker = format!("MODEL_DOCTOR_CASE_{check_id}_OK");
                if assistant_turn.final_text.trim() == marker {
                    return result;
                }
                return merge_results(
                    result,
                    tool_failure(
                        adapter,
                        check_id,
                        "assistant_turn.final_text",
                        &format!("expected {marker}, received {}", assistant_turn.final_text),
                    ),
                );
            }
            LoopDecision::Continue(tool_results) => {
                current = match conversation.append_follow_up(&assistant_turn, &tool_results) {
                    Ok(spec) => spec,
                    Err(error) => {
                        return merge_results(
                            result,
                            tool_failure(adapter, check_id, "tool_result", error.to_string()),
                        );
                    }
                };
            }
            LoopDecision::Stop {
                contract_errors, ..
            } => {
                return merge_results(
                    result,
                    tool_failure(adapter, check_id, "tool_calls", &contract_errors.join(", ")),
                );
            }
        }
    }

    merge_results(
        result,
        tool_failure(
            adapter,
            check_id,
            "tool_loop",
            "assistant turn limit exceeded",
        ),
    )
}

async fn execute_spec(
    adapter: Adapter,
    scenario: &str,
    protocol: Protocol,
    auth_mode: AuthMode,
    spec: RequestSpec,
    settings: &OpenCodexSettings,
    executor: &HttpExecutor,
) -> crate::audit::RequestEvidence {
    executor
        .execute(
            RequestInput {
                request_id: format!("llm-gateway-{}-{scenario}", adapter.id()),
                url: normalize_request_url(protocol, &settings.url, spec.stream),
                protocol,
                auth_mode,
                body: Body::Json(spec.body).to_bytes(),
                stream: spec.stream,
                api_key: settings.api_key.clone(),
            },
            settings.cancellation.child_token(),
        )
        .await
}

async fn evaluate_evidence(
    adapter: Adapter,
    protocol: Protocol,
    evidence: &crate::audit::RequestEvidence,
) -> AdapterResult {
    let successful_status = evidence
        .metrics
        .http_status
        .is_some_and(|status| (200..300).contains(&status));
    if !successful_status || !evidence.transport_outcome.is_success() {
        return evaluate_http_failure(adapter, evidence.metrics.http_status, &evidence.error);
    }
    let parsed = parse_stream(protocol, &evidence.response_body).await;
    evaluate_stream(adapter, &parsed)
}

fn pass_result(adapter: Adapter) -> AdapterResult {
    AdapterResult {
        adapter,
        passed: true,
        failures: Vec::new(),
    }
}

fn merge_results(mut left: AdapterResult, right: AdapterResult) -> AdapterResult {
    assert_eq!(left.adapter, right.adapter);
    for failure in right.failures {
        if left.failures.iter().all(|existing| {
            existing.rule_id != failure.rule_id
                || existing.observed_path != failure.observed_path
                || existing.actual != failure.actual
        }) {
            left.failures.push(failure);
        }
    }
    left.passed = left.failures.is_empty();
    left
}

fn tool_failure(
    adapter: Adapter,
    check_id: &str,
    observed_path: &str,
    actual: impl Into<String>,
) -> AdapterResult {
    let rule_id = match adapter {
        Adapter::OpenAiChat => "GW-CHAT-TOOL-004",
        Adapter::Anthropic => "GW-ANTH-TOOL-005",
        Adapter::Google => "GW-GOOGLE-TOOL-004",
    };
    let rule = rule(rule_id);
    AdapterResult {
        adapter,
        passed: false,
        failures: vec![crate::opencodex::evaluator::RuleFailure {
            rule_id,
            requirement: rule.requirement,
            observed_path: format!("{check_id}.{observed_path}"),
            actual: actual.into(),
            effect: rule.effect,
        }],
    }
}

const fn adapter_connection(adapter: Adapter) -> (Protocol, AuthMode) {
    match adapter {
        Adapter::OpenAiChat => (Protocol::OpenAiChat, AuthMode::Bearer),
        Adapter::Anthropic => (Protocol::AnthropicMessages, AuthMode::XApiKey),
        Adapter::Google => (Protocol::GeminiGenerateContent, AuthMode::XGoogApiKey),
    }
}

const fn adapter_display_name(adapter: Adapter) -> &'static str {
    match adapter {
        Adapter::OpenAiChat => "OpenAI Chat",
        Adapter::Anthropic => "Anthropic",
        Adapter::Google => "Google",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use httpmock::Method::POST;
    use httpmock::MockServer;
    use serde_json::json;
    use tokio_util::sync::CancellationToken;

    use crate::opencodex::contract::Adapter;

    use super::{OpenCodexSettings, run};

    #[tokio::test]
    async fn runner_evaluates_every_adapter_when_chat_tool_follow_up_fails() {
        let server = MockServer::start_async().await;
        let chat = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .header("authorization", "Bearer secret-key")
                    .body_includes("MODEL_DOCTOR_LLM_GATEWAY_STREAM_OK");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(format!(
                        "{}\n",
                        include_str!("../protocol/fixtures/openai_chat_final.sse")
                    ));
            })
            .await;

        let outcome = run(OpenCodexSettings {
            url: server.url("/v1/chat/completions").parse().unwrap(),
            model: "test-model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            cancellation: CancellationToken::new(),
        })
        .await
        .unwrap();

        let chat_result = outcome.result(Adapter::OpenAiChat);
        assert!(!chat_result.passed);
        assert!(
            chat_result
                .failures
                .iter()
                .any(|failure| failure.observed_path == "http_status" && failure.actual == "404")
        );
        assert!(!outcome.result(Adapter::Anthropic).passed);
        assert!(!outcome.result(Adapter::Google).passed);
        chat.assert_async().await;
    }

    #[tokio::test]
    async fn runner_fails_chat_when_its_non_stream_response_has_the_wrong_shape() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_LLM_GATEWAY_RESPONSE_OK");
                then.status(200)
                    .header("content-type", "application/json")
                    .body(r#"{"unexpected":true}"#);
            })
            .await;

        let outcome = run(OpenCodexSettings {
            url: server.url("/v1/chat/completions").parse().unwrap(),
            model: "test-model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            cancellation: CancellationToken::new(),
        })
        .await
        .unwrap();

        assert!(
            outcome
                .result(Adapter::OpenAiChat)
                .failures
                .iter()
                .any(|failure| {
                    failure.rule_id == "GW-CHAT-SHAPE-001"
                        && failure.actual
                            == "response did not match the OpenAI Chat response structure"
                })
        );
    }

    #[tokio::test]
    async fn runner_completes_a_chat_tool_result_round_trip() {
        let server = MockServer::start_async().await;
        let response = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .header("authorization", "Bearer secret-key")
                    .body_includes("MODEL_DOCTOR_LLM_GATEWAY_RESPONSE_OK");
                then.status(200).json_body(json!({
                    "choices": [{"message": {"content": "MODEL_DOCTOR_LLM_GATEWAY_RESPONSE_OK"}}]
                }));
            })
            .await;
        let basic = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .header("authorization", "Bearer secret-key")
                    .body_includes("MODEL_DOCTOR_LLM_GATEWAY_STREAM_OK");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(format!(
                        "{}\n",
                        include_str!("../protocol/fixtures/openai_chat_final.sse")
                    ));
            })
            .await;
        let initial_tool_turn = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_CASE_046")
                    .body_excludes("\"role\":\"tool\"");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(format!(
                        "{}\n",
                        include_str!("../protocol/fixtures/openai_chat_tool.sse")
                    ));
            })
            .await;
        let final_tool_turn = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_CASE_046")
                    .body_includes("\"role\":\"tool\"");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(format!(
                        "{}\n",
                        include_str!("../protocol/fixtures/openai_chat_final.sse")
                    ));
            })
            .await;
        let serial_initial = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_CASE_047")
                    .body_excludes("\"role\":\"tool\"");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(chat_tool_stream(
                        "call_weather_047",
                        "doctor__get_weather",
                        r#"{"city":"Beijing"}"#,
                    ));
            })
            .await;
        let serial_time = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_CASE_047")
                    .body_includes("WEATHER_SUNNY")
                    .body_excludes("TIME_UTC_12:00");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(chat_tool_stream(
                        "call_time_047",
                        "get_time",
                        r#"{"zone":"UTC"}"#,
                    ));
            })
            .await;
        let serial_final = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/v1/chat/completions")
                    .body_includes("MODEL_DOCTOR_CASE_047")
                    .body_includes("TIME_UTC_12:00");
                then.status(200)
                    .header("content-type", "text/event-stream")
                    .body(chat_final_stream("MODEL_DOCTOR_CASE_047_OK"));
            })
            .await;

        let outcome = run(OpenCodexSettings {
            url: server.url("/v1/chat/completions").parse().unwrap(),
            model: "test-model".into(),
            api_key: "secret-key".into(),
            timeout: Duration::from_secs(2),
            insecure: false,
            cancellation: CancellationToken::new(),
        })
        .await
        .unwrap();

        let basic_calls = basic.calls_async().await;
        let response_calls = response.calls_async().await;
        let initial_tool_calls = initial_tool_turn.calls_async().await;
        let final_tool_calls = final_tool_turn.calls_async().await;
        let serial_initial_calls = serial_initial.calls_async().await;
        let serial_time_calls = serial_time.calls_async().await;
        let serial_final_calls = serial_final.calls_async().await;
        let chat_result = outcome.result(Adapter::OpenAiChat);
        assert!(
            chat_result.passed,
            "response={response_calls}, basic={basic_calls}, initial_tool={initial_tool_calls}, final_tool={final_tool_calls}, serial_initial={serial_initial_calls}, serial_time={serial_time_calls}, serial_final={serial_final_calls}; {:?}",
            chat_result
                .failures
                .iter()
                .map(|failure| format!(
                    "{} {} {}",
                    failure.rule_id, failure.observed_path, failure.actual
                ))
                .collect::<Vec<_>>()
        );
        basic.assert_async().await;
        response.assert_async().await;
        initial_tool_turn.assert_async().await;
        final_tool_turn.assert_async().await;
        serial_initial.assert_async().await;
        serial_time.assert_async().await;
        serial_final.assert_async().await;
    }

    fn chat_tool_stream(call_id: &str, name: &str, arguments: &str) -> String {
        format!(
            "{}{}{}data: [DONE]\n\n",
            sse(json!({
                "id": "chatcmpl-tool",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "test-model",
                "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}]
            })),
            sse(json!({
                "id": "chatcmpl-tool",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "test-model",
                "choices": [{"index": 0, "delta": {"tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]}, "finish_reason": null}]
            })),
            sse(json!({
                "id": "chatcmpl-tool",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "test-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
            }))
        )
    }

    fn chat_final_stream(marker: &str) -> String {
        format!(
            "{}{}data: [DONE]\n\n",
            sse(json!({
                "id": "chatcmpl-final",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "test-model",
                "choices": [{"index": 0, "delta": {"role": "assistant", "content": marker}, "finish_reason": null}]
            })),
            sse(json!({
                "id": "chatcmpl-final",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "test-model",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
            }))
        )
    }

    fn sse(value: serde_json::Value) -> String {
        format!("data: {value}\n\n")
    }
}
