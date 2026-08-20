from __future__ import annotations

import json
import sys
import unittest
from copy import deepcopy
from html import escape
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_assessment import (  # noqa: E402
    ASSESSMENT_SCHEMA_VERSION,
    CAPABILITY_SCOPE_BOUNDARY,
    REVIEW_SCHEMA_VERSION,
    assemble_assessment,
    validate_assessment,
)
from model_doctor_contracts import (  # noqa: E402
    CONTRACT_TEST_IDS,
    V1_CONTRACT,
    V2_CONTRACT,
    V3_CONTRACT,
)
from model_doctor_html import (  # noqa: E402
    _request_evidence,
    render_report,
)


ASSESSMENT_V7 = "llm-capability-doctor.assessment.v8"
PARSED_EVIDENCE_V1 = "llm-capability-doctor.parsed-evidence.v1"
REVIEWS_V2 = "llm-capability-doctor.reviews.v2"
ASSET_DIR = SKILL_DIR / "assets"


class AssessmentV7ProtocolConformanceTests(unittest.TestCase):
    def _logic(self) -> dict:
        return {
            "purpose": "Check one collected capability.",
            "method": "Review the recorded request evidence.",
            "passCriteria": ["The recorded capability check passes."],
            "failCriteria": ["The recorded capability check fails."],
            "capabilityBoundary": "The conclusion covers only this run.",
        }

    def _verified_facts(self) -> dict:
        return {
            "interfaceProtocol": {
                "evidenceState": "NOT_COLLECTED",
                "family": "UNKNOWN",
                "requestFormat": "No interface request format was collected.",
                "responseFormat": "No interface response format was collected.",
                "statement": "No interface protocol fact was collected.",
                "evidenceRefs": [],
                "boundary": "No protocol can be inferred without evidence.",
            },
            "contextWindow": {
                "evidenceState": "NOT_COLLECTED",
                "highestVerifiedTier": None,
                "highestVerifiedInputTokens": None,
                "firstFailedTier": None,
                "firstFailedInputTokens": None,
                "statement": "No context tier evidence was collected.",
                "evidenceRefs": [],
                "boundary": "No context limit can be inferred.",
            },
            "concurrency": {
                "evidenceState": "NOT_COLLECTED",
                "highestVerifiedConcurrentRequests": None,
                "statement": "No concurrency wave was collected.",
                "evidenceRefs": [],
                "boundary": "No concurrency limit can be inferred.",
            },
        }

    def _chat_response(self, request_id: str) -> dict:
        return {
            "id": f"chatcmpl-{request_id}",
            "object": "chat.completion",
            "created": 1,
            "model": "fixture-model",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "ok"},
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

    def _request(self, request_id: str, response: dict) -> dict:
        return {
            "request_id": request_id,
            "protocol": "openai_chat",
            "stream": "0",
            "requestBody": json.dumps(
                {
                    "model": "fixture-model",
                    "messages": [{"role": "user", "content": "ping"}],
                }
            ),
            "responseHeaders": "content-type: application/json",
            "responseBody": json.dumps(response),
            "stderr": "",
            "metrics": {"http_status": "200", "curl_exit_code": "0"},
        }

    def _v3_request(self, request_id: str, response: dict) -> dict:
        request = self._request(request_id, response)
        request.update(
            {
                "transport_outcome": "completed_eof",
                "stream_termination": "completed",
                "stream_end_signal": "[DONE]",
                "model_stop_reason": "stop",
                "stream_event_count": "3",
                "tool_contract_status": "conformant",
                "tool_contract_errors_json": "[]",
                "tool_loop_turn": "1",
                "tool_loop_outcome": "completed",
            }
        )
        return request

    def _v3_tool_loop(self, test_id: str) -> list[dict]:
        turn_count = {"046": 2, "047": 3, "048": 2, "049": 3}[test_id]
        fixture_dir = SKILL_DIR.parents[1] / "src" / "protocol" / "fixtures"
        tool_stream = (fixture_dir / "openai_chat_tool.sse").read_text(
            encoding="utf-8"
        )
        final_stream = (fixture_dir / "openai_chat_final.sse").read_text(
            encoding="utf-8"
        )
        body = {
            "model": "gpt-test",
            "messages": [
                {"role": "user", "content": f"MODEL_DOCTOR_CASE_{test_id}"}
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather.",
                        "strict": True,
                        "parameters": {
                            "type": "object",
                            "properties": {"city": {"type": "string"}},
                            "required": ["city"],
                            "additionalProperties": False,
                        },
                    },
                }
            ],
            "tool_choice": "auto",
            "parallel_tool_calls": False,
            "stream": True,
        }
        requests = []
        for turn in range(1, turn_count + 1):
            final = turn == turn_count
            request_id = f"test-{test_id}-turn-{turn}"
            call_id = f"call-{test_id}-{turn}"
            response_body = (
                final_stream
                if final
                else tool_stream.replace("call_weather_046", call_id)
            )
            requests.append(
                {
                    "request_id": request_id,
                    "protocol": "openai_chat",
                    "stream": "1",
                    "requestBody": json.dumps(body),
                    "responseHeaders": "content-type: text/event-stream",
                    "responseBody": response_body,
                    "stderr": "",
                    "metrics": {"http_status": "200", "curl_exit_code": "0"},
                    "transport_outcome": "completed_eof",
                    "stream_termination": "completed",
                    "stream_end_signal": "[DONE]",
                    "model_stop_reason": "stop" if final else "tool_calls",
                    "stream_event_count": "4" if final else "5",
                    "tool_contract_status": "conformant",
                    "tool_contract_errors_json": "[]",
                    "tool_loop_turn": str(turn),
                    "tool_loop_outcome": "completed" if final else "continued",
                }
            )
            if not final:
                body = deepcopy(body)
                body["messages"].extend(
                    [
                        {
                            "role": "assistant",
                            "content": None,
                            "tool_calls": [
                                {
                                    "id": call_id,
                                    "type": "function",
                                    "function": {
                                        "name": "get_weather",
                                        "arguments": '{"city":"Beijing"}',
                                    },
                                }
                            ],
                        },
                        {
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": (
                                "ERROR: timeout"
                                if test_id == "049" and turn == 1
                                else "WEATHER_SUNNY"
                            ),
                        },
                    ]
                )
        return requests

    def _fixture(self, *, second_request_is_different: bool = True) -> tuple[dict, dict]:
        referenced_id = "request-referenced"
        unreferenced_id = "request-unreferenced"
        second_response = (
            {"id": "chatcmpl-incomplete"}
            if second_request_is_different
            else self._chat_response(unreferenced_id)
        )
        parsed = {
            "schemaVersion": PARSED_EVIDENCE_V1,
            "source": {
                "fileName": "fixture.log",
                "size": 1,
                "sha256": "0" * 64,
            },
            "run": {
                "model": "fixture-model",
                "api_key": "[MASKED]",
                "log_schema": V1_CONTRACT[0],
                "script_version": V1_CONTRACT[1],
            },
            "tokenTotals": {},
            "warnings": [],
            "tests": {
                "001": {
                    "category": "Core",
                    "name": "One capability check",
                    "requestRefs": [referenced_id],
                }
            },
            "requests": {
                referenced_id: self._request(
                    referenced_id,
                    self._chat_response(referenced_id),
                ),
                unreferenced_id: self._request(unreferenced_id, second_response),
            },
        }
        reviews = {
            "schemaVersion": REVIEWS_V2,
            "tests": {
                "001": {
                    "testId": "001",
                    "reviewedStatus": "PASS",
                    "conclusion": "The recorded capability check passes.",
                    "logic": self._logic(),
                    "evidenceRefs": [f"request:{referenced_id}"],
                    "evidenceExcerpts": ["The response contains the expected marker."],
                    "limitations": [],
                    "retestInstructions": [],
                }
            },
            "capabilitySummary": {
                "headline": "The collected capability evidence was reviewed.",
                "verifiedFacts": self._verified_facts(),
                "issues": [],
                "scopeBoundary": CAPABILITY_SCOPE_BOUNDARY,
            },
        }
        return parsed, reviews

    def _complete_fixture(
        self,
        contract: tuple[str, str],
    ) -> tuple[dict, dict]:
        parsed, reviews = self._fixture(second_request_is_different=False)
        parsed["run"]["log_schema"], parsed["run"]["script_version"] = contract
        parsed["tests"] = {}
        reviews["tests"] = {}
        for test_id in sorted(CONTRACT_TEST_IDS[contract]):
            request_refs = ["request-referenced"] if test_id == "001" else []
            parsed["tests"][test_id] = {
                "category": "Core",
                "name": f"Capability check {test_id}",
                "requestRefs": request_refs,
            }
            evidence_refs = (
                ["request:request-referenced"]
                if request_refs
                else [f"test:{test_id}:manifest"]
            )
            reviews["tests"][test_id] = {
                "testId": test_id,
                "reviewedStatus": "PASS",
                "conclusion": "The recorded capability check passes.",
                "logic": self._logic(),
                "evidenceRefs": evidence_refs,
                "evidenceExcerpts": ["The evidence satisfies this check."],
                "limitations": [],
                "retestInstructions": [],
            }
        if contract == V3_CONTRACT:
            parsed["requests"] = {
                request_id: self._v3_request(
                    request_id,
                    self._chat_response(request_id),
                )
                for request_id in parsed["requests"]
            }
            for test_id in ("046", "047", "048", "049"):
                requests = self._v3_tool_loop(test_id)
                request_ids = [request["request_id"] for request in requests]
                parsed["requests"].update(
                    {request["request_id"]: request for request in requests}
                )
                parsed["tests"][test_id]["requestRefs"] = request_ids
                reviews["tests"][test_id]["evidenceRefs"] = [
                    f"request:{request_id}" for request_id in request_ids
                ]
        return parsed, reviews

    def _v3_tool_turn_fixture(self) -> tuple[dict, dict]:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)
        tool_request_ids = {
            request_id
            for test_id in ("046", "047", "048", "049")
            for request_id in parsed["tests"][test_id]["requestRefs"]
        }
        parsed["requests"] = {
            request_id: request
            for request_id, request in parsed["requests"].items()
            if request_id in tool_request_ids
        }
        parsed["tests"]["001"]["requestRefs"] = []
        reviews["tests"]["001"]["evidenceRefs"] = ["test:001:manifest"]
        return parsed, reviews

    def _schema(self) -> dict:
        return json.loads(
            (SKILL_DIR / "references" / "assessment-schema.json").read_text(
                encoding="utf-8"
            )
        )

    def _resolve_ref(self, schema: dict, node: object) -> dict:
        self.assertIsInstance(node, dict)
        assert isinstance(node, dict)
        reference = node.get("$ref")
        self.assertIsInstance(reference, str)
        assert isinstance(reference, str)
        self.assertTrue(reference.startswith("#/$defs/"), reference)
        name = reference.removeprefix("#/$defs/")
        self.assertIn(name, schema["$defs"])
        return schema["$defs"][name]

    def _assert_closed_object(self, node: dict, fields: set[str]) -> None:
        self.assertEqual("object", node.get("type"))
        self.assertFalse(node.get("additionalProperties", True))
        self.assertEqual(fields, set(node.get("required", [])))
        self.assertEqual(fields, set(node.get("properties", {})))

    def test_assembly_emits_v7_without_changing_input_contract_versions(self) -> None:
        parsed, reviews = self._fixture()
        parsed_before = deepcopy(parsed)
        reviews_before = deepcopy(reviews)

        assessment = assemble_assessment(parsed, reviews)

        self.assertEqual(parsed_before, parsed)
        self.assertEqual(reviews_before, reviews)
        self.assertEqual(PARSED_EVIDENCE_V1, parsed["schemaVersion"])
        self.assertEqual(REVIEWS_V2, reviews["schemaVersion"])
        self.assertEqual(REVIEWS_V2, REVIEW_SCHEMA_VERSION)
        self.assertEqual(ASSESSMENT_V7, ASSESSMENT_SCHEMA_VERSION)
        self.assertEqual(ASSESSMENT_V7, assessment["schemaVersion"])

    def test_assessment_v8_schema_allows_only_contract_total_combinations(self) -> None:
        verdict_schema = self._schema()["$defs"]["generalVerdict"]

        combinations = set()
        maxima = set()
        for branch in verdict_schema["oneOf"]:
            properties = branch["properties"]
            combinations.add(
                (
                    properties["totalTests"]["const"],
                    properties["totalCoreTests"]["const"],
                    properties["totalEnhancedTests"]["const"],
                )
            )
            maxima.add(
                (
                    properties["collectedTests"]["maximum"],
                    properties["passedTests"]["maximum"],
                    properties["passedCoreTests"]["maximum"],
                    properties["passedEnhancedTests"]["maximum"],
                )
            )
        self.assertEqual({(46, 31, 15), (47, 32, 15)}, combinations)
        self.assertEqual({(46, 46, 31, 15), (47, 47, 32, 15)}, maxima)

    def test_assessment_validator_rejects_contract_total_mismatch(self) -> None:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)
        assessment = assemble_assessment(parsed, reviews)
        mismatched = deepcopy(assessment)
        mismatched["run"]["log_schema"] = V2_CONTRACT[0]
        mismatched["run"]["script_version"] = V2_CONTRACT[1]

        self.assertTrue(
            any(
                "generalVerdict cannot be derived" in error
                for error in validate_assessment(mismatched)
            )
        )

        malformed_runs = (
            {},
            [],
            {
                "log_schema": V3_CONTRACT[0],
                "script_version": V2_CONTRACT[1],
            },
            {
                "log_schema": [V3_CONTRACT[0]],
                "script_version": V3_CONTRACT[1],
            },
        )
        for run in malformed_runs:
            with self.subTest(run=run):
                malformed = deepcopy(assessment)
                malformed["run"] = run
                try:
                    errors = validate_assessment(malformed)
                except (AttributeError, TypeError) as error:
                    self.fail(f"invalid assessment run crashed: {error}")
                self.assertTrue(any("run contract" in error for error in errors), errors)

    def test_html_renders_v3_stream_and_tool_metadata(self) -> None:
        request = self._v3_request(
            "test-046-turn-1",
            self._chat_response("test-046-turn-1"),
        )
        request["tool_contract_status"] = "non_conformant"
        request["tool_contract_errors_json"] = '["tool.call:<invalid>"]'

        html = _request_evidence([request])

        for field in (
            "transport_outcome",
            "stream_termination",
            "stream_end_signal",
            "tool_contract_status",
            "tool_loop_turn",
            "tool_loop_outcome",
        ):
            with self.subTest(field=field):
                self.assertIn(field, html)
                self.assertIn(escape(request[field]), html)
                self.assertLess(
                    html.index(escape(request[field])),
                    html.index("发给模型的完整内容"),
                )
        self.assertIn("tool_contract_errors_json", html)
        self.assertIn("tool.call:&lt;invalid&gt;", html)
        self.assertGreater(
            html.index("tool.call:&lt;invalid&gt;"),
            html.index("模型返回的完整内容"),
        )

    def test_combined_v8_v3_derives_47_32_15_totals(self) -> None:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        verdict = assessment["capabilitySummary"]["generalVerdict"]
        self.assertEqual((47, 32, 15), (
            verdict["totalTests"],
            verdict["totalCoreTests"],
            verdict["totalEnhancedTests"],
        ))
        self.assertNotIn("protocolConformance", assessment)
        self.assertEqual([], validate_assessment(assessment))

    def test_combined_v8_v2_preserves_46_31_15_totals(self) -> None:
        parsed, reviews = self._complete_fixture(V2_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        verdict = assessment["capabilitySummary"]["generalVerdict"]
        self.assertEqual((46, 31, 15), (
            verdict["totalTests"],
            verdict["totalCoreTests"],
            verdict["totalEnhancedTests"],
        ))
        self.assertNotIn("protocolConformance", assessment)
        self.assertEqual([], validate_assessment(assessment))

    def test_runtime_contract_messages_use_assessment_v8(self) -> None:
        source = (
            SCRIPT_DIR / "model_doctor_assessment.py"
        ).read_text(encoding="utf-8")

        self.assertIn("assessment.v8 contract", source)
        self.assertNotIn("assessment.v6 contract", source)


if __name__ == "__main__":
    unittest.main()
