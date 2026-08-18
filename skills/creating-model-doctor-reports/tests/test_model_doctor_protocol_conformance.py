from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_protocol_conformance import (  # noqa: E402
    BASELINE_DATE,
    DIFFERENCE_KINDS,
    RULE_SET_VERSION,
    SUPPORTED_PROTOCOLS,
    analyze_protocol_conformance,
    validate_protocol_conformance,
)


class ProtocolConformanceTests(unittest.TestCase):
    def _request(
        self,
        request_id: str,
        protocol: str | None,
        response: object = None,
        *,
        raw_response: str | None = None,
        request_body: object | str | None = None,
        http_status: object = "200",
        curl_exit_code: object = "0",
        stream: object = "0",
        response_headers: str = "content-type: application/json; charset=utf-8",
    ) -> dict:
        request = {
            "request_id": request_id,
            "stream": stream,
            "requestBody": (
                request_body
                if isinstance(request_body, str)
                else json.dumps(request_body or {"model": "fixture"})
            ),
            "metrics": {
                "http_status": http_status,
                "curl_exit_code": curl_exit_code,
            },
            "responseHeaders": response_headers,
            "stderr": "",
            "responseBody": (
                raw_response
                if raw_response is not None
                else json.dumps(response)
            ),
        }
        if protocol is not None:
            request["protocol"] = protocol
        return request

    def _parsed(self, requests: dict, tests: dict | None = None) -> dict:
        return {
            "requests": requests,
            "tests": tests or {},
        }

    def _chat(self) -> dict:
        return {
            "id": "chatcmpl-fixture",
            "object": "chat.completion",
            "created": 1,
            "model": "gpt-fixture",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "ok",
                    },
                    "finish_reason": "stop",
                    "logprobs": None,
                }
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2,
            },
        }

    def _responses(self) -> dict:
        return {
            "id": "resp_fixture",
            "object": "response",
            "created_at": 1,
            "error": None,
            "incomplete_details": None,
            "instructions": None,
            "model": "gpt-fixture",
            "tools": [],
            "output": [
                {
                    "id": "msg_fixture",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "ok",
                            "annotations": [],
                        }
                    ],
                    "status": "completed",
                }
            ],
            "parallel_tool_calls": True,
            "metadata": {},
            "tool_choice": "auto",
            "temperature": 1.0,
            "top_p": 1.0,
            "status": "completed",
        }

    def _anthropic(self) -> dict:
        return {
            "id": "msg_fixture",
            "type": "message",
            "role": "assistant",
            "model": "claude-fixture",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "stop_sequence": None,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        }

    def _gemini(self) -> dict:
        return {
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [{"text": "ok"}],
                    },
                    "finishReason": "STOP",
                    "index": 0,
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 1,
                "totalTokenCount": 2,
            },
            "modelVersion": "gemini-fixture",
            "responseId": "resp-fixture",
        }

    def _ollama(self) -> dict:
        return {
            "model": "ollama-fixture",
            "created_at": "2026-08-18T00:00:00Z",
            "message": {"role": "assistant", "content": "ok"},
            "done": True,
            "done_reason": "stop",
            "total_duration": 10,
            "load_duration": 2,
            "prompt_eval_count": 1,
            "prompt_eval_duration": 3,
            "eval_count": 1,
            "eval_duration": 4,
        }

    def _single_result(self, request: dict) -> dict:
        report = analyze_protocol_conformance(
            self._parsed({request["request_id"]: request})
        )
        self.assertEqual([], validate_protocol_conformance(report))
        return report["results"][0]

    def _assert_one_difference(
        self,
        result: dict,
        location: str,
        difference_kind: str,
    ) -> None:
        self.assertEqual("DIFFERENT", result["status"])
        self.assertEqual(1, len(result["differences"]), result["differences"])
        self.assertEqual(location, result["differences"][0]["location"])
        self.assertEqual(
            difference_kind,
            result["differences"][0]["differenceKind"],
        )

    def test_public_constants_and_pinned_baselines(self) -> None:
        self.assertEqual(
            "official-protocol-conformance.2026-08-18",
            RULE_SET_VERSION,
        )
        self.assertEqual("2026-08-18", BASELINE_DATE)
        self.assertEqual(
            (
                "openai_chat",
                "openai_responses",
                "anthropic_messages",
                "gemini_generate_content",
                "ollama_chat",
            ),
            SUPPORTED_PROTOCOLS,
        )
        self.assertEqual(
            (
                "EVIDENCE_GAP",
                "FRAMING",
                "INVALID_JSON",
                "MISSING_FIELD",
                "UNEXPECTED_FIELD",
                "TYPE_MISMATCH",
                "ENUM_MISMATCH",
                "VALUE_MISMATCH",
                "SEQUENCE",
                "CORRELATION",
            ),
            DIFFERENCE_KINDS,
        )

        report = analyze_protocol_conformance(self._parsed({}))
        self.assertEqual(list(SUPPORTED_PROTOCOLS), [
            item["protocol"] for item in report["baselines"]
        ])
        self.assertTrue(all(
            item["referenceDate"] == BASELINE_DATE
            and item["sourceUrl"].startswith("https://")
            and item["supportingReferences"]
            for item in report["baselines"]
        ))
        self.assertEqual([], validate_protocol_conformance(report))

    def test_each_nonstream_success_profile_is_consistent(self) -> None:
        fixtures = {
            "openai_chat": self._chat(),
            "openai_responses": self._responses(),
            "anthropic_messages": self._anthropic(),
            "gemini_generate_content": self._gemini(),
            "ollama_chat": self._ollama(),
        }

        for protocol, response in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(f"req-{protocol}", protocol, response)
                )
                self.assertEqual("CONSISTENT", result["status"])
                self.assertEqual([], result["differences"])

    def test_chat_function_arguments_must_be_json_object_string(self) -> None:
        response = self._chat()
        response["choices"][0]["message"] = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": {"city": "Beijing"},
                    },
                }
            ],
        }
        response["choices"][0]["finish_reason"] = "tool_calls"
        result = self._single_result(
            self._request("req-chat-tool", "openai_chat", response)
        )
        self._assert_one_difference(
            result,
            "/choices/0/message/tool_calls/0/function/arguments",
            "TYPE_MISMATCH",
        )

    def test_responses_function_arguments_must_be_json_object_string(self) -> None:
        response = self._responses()
        response["output"] = [
            {
                "id": "fc_fixture",
                "type": "function_call",
                "call_id": "call_1",
                "name": "get_weather",
                "arguments": {"city": "Beijing"},
                "status": "completed",
            }
        ]
        result = self._single_result(
            self._request("req-responses-tool", "openai_responses", response)
        )
        self._assert_one_difference(
            result,
            "/output/0/arguments",
            "TYPE_MISMATCH",
        )

    def test_anthropic_tool_input_must_be_object(self) -> None:
        response = self._anthropic()
        response["content"] = [
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "get_weather",
                "input": '{"city":"Beijing"}',
            }
        ]
        response["stop_reason"] = "tool_use"
        result = self._single_result(
            self._request("req-anthropic-tool", "anthropic_messages", response)
        )
        self._assert_one_difference(
            result,
            "/content/0/input",
            "TYPE_MISMATCH",
        )

    def test_gemini_required_function_args_must_be_object(self) -> None:
        response = self._gemini()
        response["candidates"][0]["content"]["parts"] = [
            {
                "functionCall": {
                    "name": "get_weather",
                    "args": '{"city":"Beijing"}',
                }
            }
        ]
        request_body = {
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "get_weather",
                            "parameters": {
                                "type": "OBJECT",
                                "properties": {"city": {"type": "STRING"}},
                                "required": ["city"],
                            },
                        }
                    ]
                }
            ]
        }
        result = self._single_result(
            self._request(
                "req-gemini-tool",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self._assert_one_difference(
            result,
            "/candidates/0/content/parts/0/functionCall/args",
            "TYPE_MISMATCH",
        )

    def test_ollama_function_arguments_must_be_object(self) -> None:
        response = self._ollama()
        response["message"]["content"] = ""
        response["message"]["tool_calls"] = [
            {
                "function": {
                    "name": "get_weather",
                    "arguments": '{"city":"Beijing"}',
                }
            }
        ]
        result = self._single_result(
            self._request("req-ollama-tool", "ollama_chat", response)
        )
        self._assert_one_difference(
            result,
            "/message/tool_calls/0/function/arguments",
            "TYPE_MISMATCH",
        )

    def test_optional_omission_and_missing_content_type_pass(self) -> None:
        response = self._chat()
        response.pop("usage")
        response["choices"][0].pop("logprobs")
        result = self._single_result(
            self._request(
                "req-optional",
                "openai_chat",
                response,
                response_headers="x-request-id: req-fixture",
            )
        )
        self.assertEqual("CONSISTENT", result["status"])

    def test_undocumented_field_is_rejected(self) -> None:
        response = self._chat()
        response["undocumented"] = "do not echo this"
        result = self._single_result(
            self._request("req-extra", "openai_chat", response)
        )
        self._assert_one_difference(
            result,
            "/undocumented",
            "UNEXPECTED_FIELD",
        )
        self.assertEqual("present", result["differences"][0]["actual"])

    def test_invalid_json_is_rejected_without_echoing_body(self) -> None:
        result = self._single_result(
            self._request(
                "req-json",
                "openai_chat",
                raw_response='{"id":',
            )
        )
        self._assert_one_difference(result, "/responseBody", "INVALID_JSON")
        self.assertNotIn('{"id":', json.dumps(result))

    def test_valid_error_envelopes_for_all_protocols(self) -> None:
        fixtures = {
            "openai_chat": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "openai_responses": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "anthropic_messages": {
                "type": "error",
                "error": {"type": "invalid_request_error", "message": "bad"},
                "request_id": "req_fixture",
            },
            "gemini_generate_content": {
                "error": {"code": 400, "message": "bad", "status": "INVALID_ARGUMENT"}
            },
            "ollama_chat": {"error": "bad request"},
        }
        for protocol, response in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-error-{protocol}",
                        protocol,
                        response,
                        http_status="400",
                    )
                )
                self.assertEqual("CONSISTENT", result["status"])

    def test_invalid_error_envelopes_for_all_four_families(self) -> None:
        fixtures = {
            "openai_chat": (
                {"error": {"type": "bad", "message": "bad", "code": None}},
                "/error/param",
                "MISSING_FIELD",
            ),
            "anthropic_messages": (
                {"type": "error", "error": {"type": "bad", "message": 1}},
                "/error/message",
                "TYPE_MISMATCH",
            ),
            "gemini_generate_content": (
                {"error": {"code": 401, "message": "bad"}},
                "/error/code",
                "VALUE_MISMATCH",
            ),
            "ollama_chat": (
                {"error": {"message": "bad"}},
                "/error",
                "TYPE_MISMATCH",
            ),
        }
        for protocol, (response, location, kind) in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-invalid-error-{protocol}",
                        protocol,
                        response,
                        http_status="400",
                    )
                )
                self._assert_one_difference(result, location, kind)

    def test_unknown_and_missing_protocol_are_evidence_gaps(self) -> None:
        for protocol in ("custom_protocol", None):
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(f"req-{protocol}", protocol, self._chat())
                )
                self._assert_one_difference(result, "/protocol", "EVIDENCE_GAP")

    def test_transport_empty_body_and_invalid_status_are_evidence_gaps(self) -> None:
        fixtures = (
            self._request(
                "req-transport",
                "openai_chat",
                raw_response="",
                curl_exit_code="7",
            ),
            self._request(
                "req-empty",
                "openai_chat",
                raw_response="",
            ),
            self._request(
                "req-status",
                "openai_chat",
                self._chat(),
                http_status="not-a-status",
            ),
        )
        locations = (
            "/metrics/curl_exit_code",
            "/responseBody",
            "/metrics/http_status",
        )
        for request, location in zip(fixtures, locations):
            with self.subTest(request_id=request["request_id"]):
                result = self._single_result(request)
                self._assert_one_difference(result, location, "EVIDENCE_GAP")

    def test_bool_is_not_an_integer(self) -> None:
        response = self._chat()
        response["created"] = True
        result = self._single_result(
            self._request("req-bool", "openai_chat", response)
        )
        self._assert_one_difference(result, "/created", "TYPE_MISMATCH")

    def test_root_json_type_mismatches_remain_self_validating(self) -> None:
        for protocol in SUPPORTED_PROTOCOLS:
            for http_status in ("200", "400"):
                with self.subTest(protocol=protocol, http_status=http_status):
                    request = self._request(
                        f"req-root-{protocol}-{http_status}",
                        protocol,
                        raw_response="[]",
                        http_status=http_status,
                    )
                    report = analyze_protocol_conformance(
                        self._parsed({request["request_id"]: request})
                    )
                    self.assertEqual([], validate_protocol_conformance(report))
                    self._assert_one_difference(
                        report["results"][0],
                        "/responseBody",
                        "TYPE_MISMATCH",
                    )

    def test_difference_actual_does_not_echo_untrusted_enum_values(self) -> None:
        marker = "SECRET_MARKER_42"
        response = self._chat()
        response["choices"][0]["finish_reason"] = marker
        report = analyze_protocol_conformance(
            self._parsed(
                {
                    "req-bounded-actual": self._request(
                        "req-bounded-actual",
                        "openai_chat",
                        response,
                    )
                }
            )
        )
        self.assertEqual([], validate_protocol_conformance(report))
        self.assertNotIn(marker, json.dumps(report))
        self._assert_one_difference(
            report["results"][0],
            "/choices/0/finish_reason",
            "ENUM_MISMATCH",
        )

    def test_gemini_args_are_required_only_for_required_parameters(self) -> None:
        response = self._gemini()
        response["candidates"][0]["content"]["parts"] = [
            {"functionCall": {"name": "optional_lookup"}}
        ]
        request_body = {
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "optional_lookup",
                            "parameters": {
                                "type": "OBJECT",
                                "properties": {"query": {"type": "STRING"}},
                            },
                        }
                    ]
                }
            ]
        }
        result = self._single_result(
            self._request(
                "req-gemini-optional-args",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self.assertEqual("CONSISTENT", result["status"])

        request_body["tools"][0]["functionDeclarations"][0]["parameters"][
            "required"
        ] = ["query"]
        result = self._single_result(
            self._request(
                "req-gemini-required-args",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self._assert_one_difference(
            result,
            "/candidates/0/content/parts/0/functionCall/args",
            "MISSING_FIELD",
        )

    def test_gemini_missing_args_need_valid_request_evidence(self) -> None:
        for request_body in ("{bad", "[]"):
            with self.subTest(request_body=request_body):
                response = self._gemini()
                response["candidates"][0]["content"]["parts"] = [
                    {"functionCall": {"name": "get_weather"}}
                ]
                result = self._single_result(
                    self._request(
                        f"req-gemini-invalid-request-{len(request_body)}",
                        "gemini_generate_content",
                        response,
                        request_body=request_body,
                    )
                )
                self._assert_one_difference(
                    result,
                    "/requestBody",
                    "EVIDENCE_GAP",
                )

    def test_request_identity_mismatch_is_reported(self) -> None:
        request = self._request("inner-id", "openai_chat", self._chat())
        report = analyze_protocol_conformance(
            self._parsed({"map-key": request})
        )
        self.assertEqual([], validate_protocol_conformance(report))
        self._assert_one_difference(
            report["results"][0],
            "/request_id",
            "CORRELATION",
        )

    def test_validator_rejects_unpinned_difference_reference(self) -> None:
        response = self._chat()
        response["created"] = "one"
        report = analyze_protocol_conformance(
            self._parsed(
                {"req-reference": self._request("req-reference", "openai_chat", response)}
            )
        )
        report["results"][0]["differences"][0]["officialReference"] = (
            "https://example.invalid/not-the-pinned-reference"
        )
        errors = validate_protocol_conformance(report)
        self.assertTrue(
            any("officialReference" in error for error in errors),
            errors,
        )

    def test_responses_nested_collections_validate_each_item(self) -> None:
        mutations = (
            ("tools", "/tools/0"),
            ("annotations", "/output/0/content/0/annotations/0"),
            ("logprobs", "/output/0/content/0/logprobs/0"),
        )
        for field, location in mutations:
            with self.subTest(field=field):
                response = self._responses()
                if field == "tools":
                    response["tools"] = ["not-an-object"]
                else:
                    response["output"][0]["content"][0][field] = [
                        "not-an-object"
                    ]
                result = self._single_result(
                    self._request(
                        f"req-responses-nested-{field}",
                        "openai_responses",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "TYPE_MISMATCH")

    def test_responses_nested_required_fields_report_without_crashing(self) -> None:
        fixtures = []

        missing_tool_name = self._responses()
        missing_tool_name["tools"] = [
            {
                "type": "function",
                "strict": True,
                "parameters": {},
            }
        ]
        fixtures.append((missing_tool_name, "/tools/0/name"))

        missing_logprob_token = self._responses()
        missing_logprob_token["output"][0]["content"][0]["logprobs"] = [
            {
                "logprob": -0.1,
                "bytes": [111],
                "top_logprobs": [],
            }
        ]
        fixtures.append(
            (
                missing_logprob_token,
                "/output/0/content/0/logprobs/0/token",
            )
        )

        missing_top_logprob_token = self._responses()
        missing_top_logprob_token["output"][0]["content"][0]["logprobs"] = [
            {
                "token": "o",
                "logprob": -0.1,
                "bytes": [111],
                "top_logprobs": [{"logprob": -0.2, "bytes": [120]}],
            }
        ]
        fixtures.append(
            (
                missing_top_logprob_token,
                "/output/0/content/0/logprobs/0/top_logprobs/0/token",
            )
        )

        for index, (response, location) in enumerate(fixtures):
            with self.subTest(location=location):
                result = self._single_result(
                    self._request(
                        f"req-responses-missing-nested-{index}",
                        "openai_responses",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "MISSING_FIELD")

    def test_anthropic_citations_validate_each_item(self) -> None:
        response = self._anthropic()
        response["content"][0]["citations"] = ["not-an-object"]
        result = self._single_result(
            self._request("req-anthropic-citation", "anthropic_messages", response)
        )
        self._assert_one_difference(
            result,
            "/content/0/citations/0",
            "TYPE_MISMATCH",
        )

    def test_anthropic_missing_block_fields_report_without_crashing(self) -> None:
        fixtures = (
            ({"type": "text"}, "text"),
            ({"type": "thinking", "signature": "sig"}, "thinking"),
            ({"type": "thinking", "thinking": "trace"}, "signature"),
            ({"type": "redacted_thinking"}, "data"),
            (
                {"type": "tool_use", "name": "tool", "input": {}},
                "id",
            ),
            (
                {"type": "tool_use", "id": "toolu_1", "input": {}},
                "name",
            ),
            (
                {"type": "tool_use", "id": "toolu_1", "name": "tool"},
                "input",
            ),
        )
        for index, (block, missing_field) in enumerate(fixtures):
            with self.subTest(block_type=block["type"], missing=missing_field):
                response = self._anthropic()
                response["content"] = [block]
                if block["type"] == "tool_use":
                    response["stop_reason"] = "tool_use"
                result = self._single_result(
                    self._request(
                        f"req-anthropic-missing-block-{index}",
                        "anthropic_messages",
                        response,
                    )
                )
                self._assert_one_difference(
                    result,
                    f"/content/0/{missing_field}",
                    "MISSING_FIELD",
                )

    def test_gemini_nested_objects_validate_their_fields(self) -> None:
        mutations = (
            (
                "citationMetadata",
                {"citationSources": "not-an-array"},
                "/candidates/0/citationMetadata/citationSources",
            ),
            (
                "executableCode",
                {"language": "PYTHON", "code": 1},
                "/candidates/0/content/parts/0/executableCode/code",
            ),
        )
        for field, value, location in mutations:
            with self.subTest(field=field):
                response = self._gemini()
                if field == "citationMetadata":
                    response["candidates"][0][field] = value
                else:
                    response["candidates"][0]["content"]["parts"] = [
                        {field: value}
                    ]
                result = self._single_result(
                    self._request(
                        f"req-gemini-nested-{field}",
                        "gemini_generate_content",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "TYPE_MISMATCH")

    def test_invalid_stream_evidence_cannot_pass_as_nonstream(self) -> None:
        invalid_values = (None, "", "wat", [], {})
        for index, stream_value in enumerate(invalid_values):
            with self.subTest(stream=stream_value):
                request = self._request(
                    f"req-invalid-stream-{index}",
                    "openai_chat",
                    self._chat(),
                )
                if stream_value is None:
                    request.pop("stream")
                else:
                    request["stream"] = stream_value
                result = self._single_result(request)
                self.assertFalse(result["stream"])
                self._assert_one_difference(result, "/stream", "EVIDENCE_GAP")

    def test_streaming_is_explicitly_deferred_as_framing_difference(self) -> None:
        result = self._single_result(
            self._request(
                "req-stream",
                "openai_chat",
                raw_response='data: {"id":"chunk"}\n\ndata: [DONE]\n\n',
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertTrue(result["stream"])
        self._assert_one_difference(result, "/responseBody", "FRAMING")

    def test_content_type_is_validated_only_when_present(self) -> None:
        result = self._single_result(
            self._request(
                "req-content-type",
                "openai_chat",
                self._chat(),
                response_headers="content-type: text/plain",
            )
        )
        self._assert_one_difference(
            result,
            "/responseHeaders/content-type",
            "VALUE_MISMATCH",
        )

    def test_all_requests_are_reported_once_with_check_associations(self) -> None:
        requests = {
            "req-z": self._request("req-z", "openai_chat", self._chat()),
            "req-unreferenced": self._request(
                "req-unreferenced", "custom_protocol", self._chat()
            ),
            "req-a": self._request("req-a", "openai_chat", self._chat()),
        }
        tests = {
            "010": {"requestRefs": ["req-z"]},
            "002": {"requestRefs": ["req-z", "req-a"]},
        }
        report = analyze_protocol_conformance(self._parsed(requests, tests))

        self.assertEqual(
            ["req-z", "req-unreferenced", "req-a"],
            [item["requestId"] for item in report["results"]],
        )
        self.assertEqual(["002", "010"], report["results"][0]["checkIds"])
        self.assertEqual([], report["results"][1]["checkIds"])
        self.assertEqual(["002"], report["results"][2]["checkIds"])
        self.assertEqual(
            {
                "totalRequests": 3,
                "checkedRequests": 3,
                "consistentRequests": 2,
                "differentRequests": 1,
            },
            {key: report["summary"][key] for key in (
                "totalRequests",
                "checkedRequests",
                "consistentRequests",
                "differentRequests",
            )},
        )
        self.assertEqual(
            ["custom_protocol", "openai_chat"],
            [item["protocol"] for item in report["summary"]["byProtocol"]],
        )
        self.assertEqual(
            ["002", "010"],
            [item["checkId"] for item in report["summary"]["byCheck"]],
        )
        self.assertEqual(
            [{"differenceKind": "EVIDENCE_GAP", "count": 1}],
            report["summary"]["byDifferenceKind"],
        )
        self.assertEqual([], validate_protocol_conformance(report))

    def test_validator_rejects_tampered_shape_counts_and_status(self) -> None:
        report = analyze_protocol_conformance(
            self._parsed(
                {"req-chat": self._request("req-chat", "openai_chat", self._chat())}
            )
        )
        mutations = []

        extra_field = copy.deepcopy(report)
        extra_field["extra"] = True
        mutations.append((extra_field, "field extra"))

        wrong_count = copy.deepcopy(report)
        wrong_count["summary"]["totalRequests"] = 2
        mutations.append((wrong_count, "totalRequests"))

        impossible_status = copy.deepcopy(report)
        impossible_status["results"][0]["status"] = "DIFFERENT"
        mutations.append((impossible_status, "differences"))

        wrong_group = copy.deepcopy(report)
        wrong_group["summary"]["byProtocol"][0]["consistentRequests"] = 0
        mutations.append((wrong_group, "byProtocol"))

        for value, expected_error in mutations:
            with self.subTest(expected_error=expected_error):
                errors = validate_protocol_conformance(value)
                self.assertTrue(errors)
                self.assertTrue(
                    any(expected_error in error for error in errors),
                    errors,
                )


if __name__ == "__main__":
    unittest.main()
