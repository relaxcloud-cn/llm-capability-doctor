from __future__ import annotations

import base64
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Iterable, Mapping


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import RETAINED_TEST_IDS, parse_log  # noqa: E402
from model_doctor_contracts import V4_TEST_IDS  # noqa: E402
from model_doctor_opencodex_compatibility import (  # noqa: E402
    COMPATIBILITY_PROFILE,
)
from model_doctor_assessment import (  # noqa: E402
    CAPABILITY_SCOPE_BOUNDARY,
    assemble_assessment,
    validate_assessment,
    validate_reviews,
)

def valid_v4_request_metadata() -> dict[str, str]:
    """Return a complete, conformant request-metadata fixture."""

    return {
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


def build_evidence_log(
    *,
    log_schema: str = "llm-capability-doctor.evidence.v4",
    script_version: str = "0.12.0",
    test_ids: Iterable[str] = V4_TEST_IDS,
    request_metadata: Mapping[str, str] | None = None,
    collection_profile: str | None = None,
    compatibility_profile: str | None = COMPATIBILITY_PROFILE,
    include_request: bool | None = None,
) -> str:
    """Build a complete evidence log for parser contract tests."""

    selected_ids = sorted(test_ids)
    is_v4 = log_schema == "llm-capability-doctor.evidence.v4"
    if include_request is None:
        include_request = is_v4
    metadata = dict(
        valid_v4_request_metadata() if request_metadata is None and is_v4
        else request_metadata or {}
    )
    request_test_id = "046" if "046" in selected_ids else selected_ids[0]
    request_id = f"test-{request_test_id}-turn-1"
    request = ""
    if include_request:
        metadata_lines = "".join(
            f"{field}: {value}\n" for field, value in metadata.items()
        )
        empty_b64 = base64.b64encode(b"").decode("ascii")
        json_b64 = base64.b64encode(b"{}").decode("ascii")
        request = (
            f"========== REQUEST {request_id} BEGIN ==========\n"
            f"request_id: {request_id}\n"
            "started_at: 2026-08-19T00:00:00+0800\n"
            "completed_at: 2026-08-19T00:00:01+0800\n"
            "protocol: openai_chat\n"
            "auth_mode: bearer\n"
            "stream: 1\n"
            f"{metadata_lines}\n"
            "----- CURL COMMAND BEGIN -----\n"
            "curl https://example.invalid/v1/chat/completions\n"
            "----- CURL COMMAND END -----\n\n"
            "----- REQUEST BODY BEGIN -----\n"
            f"{json_b64}\n"
            "----- REQUEST BODY END -----\n\n"
            "----- RESPONSE METRICS BEGIN -----\n"
            "curl_exit_code: 0\n"
            "http_status: 200\n"
            "time_total: 0.100000\n"
            "time_starttransfer: 0.050000\n"
            "size_download: 2\n"
            "----- RESPONSE METRICS END -----\n\n"
            "----- RESPONSE HEADERS BEGIN -----\n"
            "content-type: text/event-stream\n"
            "----- RESPONSE HEADERS END -----\n\n"
            "----- CURL STDERR BEGIN -----\n"
            f"{empty_b64}\n"
            "----- CURL STDERR END -----\n\n"
            "----- RESPONSE BODY BEGIN -----\n"
            f"{json_b64}\n"
            "----- RESPONSE BODY END -----\n\n"
            f"========== REQUEST {request_id} END ==========\n"
        )

    manifests = "".join(
        "========== TEST-{0} BEGIN ==========\n"
        "name: fixture-{0}\n"
        "category: fixture\n"
        "request_refs: {1}\n"
        "========== TEST-{0} END ==========\n".format(
            test_id,
            request_id if test_id == request_test_id and request else "",
        )
        for test_id in selected_ids
    )
    profile_line = (
        f"collection_profile: {collection_profile}\n"
        if collection_profile is not None
        else ""
    )
    compatibility_profile_line = (
        f"compatibility_profile: {compatibility_profile}\n"
        if is_v4 and compatibility_profile is not None
        else ""
    )
    return (
        "========== MODEL DOCTOR RUN ==========\n"
        f"script_version: {script_version}\n"
        "section_encoding: base64\n"
        f"log_schema: {log_schema}\n"
        f"{profile_line}"
        f"{compatibility_profile_line}"
        f"selected_test_count: {len(selected_ids)}\n"
        f"{request}"
        f"{manifests}"
        "========== RUN SUMMARY ==========\n"
        f"request_count: {1 if request else 0}\n"
        f"test_manifest_count: {len(selected_ids)}\n"
        "========== END ==========\n"
    )


class ModelDoctorEvidenceV4Tests(unittest.TestCase):
    def parse_text_log(self, value: str, name: str = "fixture.log") -> dict:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / name
        path.write_text(value, encoding="utf-8")
        return parse_log(path)

    @staticmethod
    def _tool_declaration() -> dict:
        return {
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

    @staticmethod
    def _tool_stream() -> str:
        fixture = SKILL_DIR.parents[1] / "src/protocol/fixtures/openai_chat_tool.sse"
        return fixture.read_text(encoding="utf-8")

    @staticmethod
    def _final_stream() -> str:
        fixture = SKILL_DIR.parents[1] / "src/protocol/fixtures/openai_chat_final.sse"
        return fixture.read_text(encoding="utf-8")

    def _guard_request(
        self,
        test_id: str,
        turn: int,
        total_turns: int,
    ) -> dict:
        request_id = f"test-{test_id}-turn-{turn}"
        initial_message = {
            "role": "user",
            "content": f"MODEL_DOCTOR_CASE_{test_id}",
        }
        body = {
            "model": "gpt-test",
            "messages": [initial_message],
            "tools": [self._tool_declaration()],
            "tool_choice": "auto",
            "parallel_tool_calls": False,
            "stream": True,
        }
        if turn > 1:
            body["messages"].extend([
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [{
                        "id": "call_weather_046",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": '{"city":"Beijing"}',
                        },
                    }],
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_weather_046",
                    "content": "WEATHER_SUNNY",
                },
            ])
        final = turn == total_turns
        return {
            "request_id": request_id,
            "protocol": "openai_chat",
            "stream": "1",
            "requestBody": json.dumps(body),
            "responseHeaders": "content-type: text/event-stream",
            "responseBody": self._final_stream() if final else self._tool_stream(),
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

    @staticmethod
    def _logic() -> dict:
        return {
            "purpose": "Check the tool loop.",
            "method": "Inspect all recorded turns.",
            "passCriteria": ["The official tool loop completes."],
            "failCriteria": ["Any required turn is incomplete."],
            "capabilityBoundary": "This conclusion covers only this run.",
        }

    @staticmethod
    def _verified_facts() -> dict:
        unavailable = {
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
        return unavailable

    def _guard_fixture(
        self,
        test_id: str = "046",
        turns: int | None = None,
        *,
        contract: tuple[str, str] = (
            "llm-capability-doctor.evidence.v4",
            "0.12.0",
        ),
    ) -> tuple[dict, dict]:
        turn_count = turns if turns is not None else {"046": 2, "047": 3, "048": 2, "049": 3}[test_id]
        request_refs = [f"test-{test_id}-turn-{turn}" for turn in range(1, turn_count + 1)]
        parsed = {
            "run": {"log_schema": contract[0], "script_version": contract[1]},
            "tests": {
                test_id: {
                    "name": f"Tool loop {test_id}",
                    "category": "Core",
                    "requestRefs": request_refs,
                }
            },
            "requests": {
                request_id: self._guard_request(test_id, turn, turn_count)
                for turn, request_id in enumerate(request_refs, start=1)
            },
            "source": {},
            "tokenTotals": {},
            "warnings": [],
        }
        reviews = {
            "schemaVersion": "llm-capability-doctor.reviews.v2",
            "tests": {
                test_id: {
                    "testId": test_id,
                    "reviewedStatus": "PASS",
                    "conclusion": "The official tool loop completed.",
                    "logic": self._logic(),
                    "evidenceRefs": [f"request:{request_id}" for request_id in request_refs],
                    "evidenceExcerpts": ["All recorded turns completed."],
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

    def test_v4_tool_pass_guard_accepts_complete_ordered_loop(self) -> None:
        parsed, reviews = self._guard_fixture()

        self.assertEqual([], validate_reviews(parsed, reviews))

    def test_v4_tool_pass_guard_rejects_non_contiguous_turn_ids(self) -> None:
        parsed, reviews = self._guard_fixture()
        request = parsed["requests"].pop("test-046-turn-2")
        request["request_id"] = "test-046-turn-3"
        parsed["requests"]["test-046-turn-3"] = request
        parsed["tests"]["046"]["requestRefs"][1] = "test-046-turn-3"
        reviews["tests"]["046"]["evidenceRefs"][1] = "request:test-046-turn-3"

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("contiguous ordered turns" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_incomplete_stream(self) -> None:
        mutations = (
            ("stream_termination", "timeout", "did not complete its stream"),
            (
                "transport_outcome",
                "timeout",
                "clean EOF or a valid immediate protocol terminal",
            ),
            ("stream_end_signal", "none", "terminal signal"),
        )
        for field, value, expected_error in mutations:
            with self.subTest(field=field):
                parsed, reviews = self._guard_fixture()
                parsed["requests"]["test-046-turn-1"][field] = value

                errors = validate_reviews(parsed, reviews)

                self.assertTrue(
                    any(expected_error in error for error in errors), errors
                )

    def test_v4_tool_pass_guard_rejects_non_conformant_contract(self) -> None:
        parsed, reviews = self._guard_fixture()
        request = parsed["requests"]["test-046-turn-1"]
        request["tool_contract_status"] = "non_conformant"
        request["tool_contract_errors_json"] = '["chat.invalid:/"]'

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("not contract-conformant" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_incomplete_final_loop_outcome(self) -> None:
        parsed, reviews = self._guard_fixture()
        parsed["requests"]["test-046-turn-2"]["tool_loop_outcome"] = "continued"

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("tool_loop_outcome=completed" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_non_continued_intermediate_outcome(self) -> None:
        parsed, reviews = self._guard_fixture()
        parsed["requests"]["test-046-turn-1"]["tool_loop_outcome"] = "completed"

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("tool_loop_outcome=continued" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_non_contiguous_manifest_refs_with_matching_embedded_ids(self) -> None:
        parsed, reviews = self._guard_fixture()
        request = parsed["requests"].pop("test-046-turn-2")
        request["request_id"] = "test-046-turn-4"
        request["tool_loop_turn"] = "4"
        parsed["requests"]["test-046-turn-4"] = request
        parsed["tests"]["046"]["requestRefs"][1] = "test-046-turn-4"
        reviews["tests"]["046"]["evidenceRefs"][1] = "request:test-046-turn-4"

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("contiguous ordered turns" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_non_mapping_request(self) -> None:
        parsed, reviews = self._guard_fixture()
        parsed["requests"]["test-046-turn-1"] = "not-an-object"

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("must be an object" in error for error in errors), errors)

    def test_v4_tool_pass_guard_rejects_integer_turn_metadata(self) -> None:
        parsed, reviews = self._guard_fixture()
        parsed["requests"]["test-046-turn-1"]["tool_loop_turn"] = 1

        errors = validate_reviews(parsed, reviews)

        self.assertTrue(any("mismatched tool_loop_turn" in error for error in errors), errors)

    def test_assessment_validator_reports_a_damaged_run_contract_without_crashing(self) -> None:
        parsed, reviews = self._guard_fixture()
        assessment = assemble_assessment(parsed, reviews)
        assessment["run"] = []

        errors = validate_assessment(assessment)

        self.assertTrue(any("run contract is invalid" in error for error in errors), errors)

    def test_skill_documents_v4_contract_and_assessment_v9(self) -> None:
        skill = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")

        for required in (
            "collector v0.12.0 with `llm-capability-doctor.evidence.v4`",
            "require all 46 manifests",
            "llm-capability-doctor.assessment.v9",
            "Only the exact v0.12.0/evidence.v4 pair is accepted",
        ):
            with self.subTest(required=required):
                self.assertIn(required, skill)

        self.assertNotIn(
            "This generated `protocolConformance` result never changes a manifest's "
            "PASS/FAIL or the general capability verdict.",
            skill,
        )

    def test_rules_define_strict_v4_tool_loop_checks(self) -> None:
        rules = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        heading = "### Evidence v4 tool-loop rules"
        next_heading = "### 050 大工具目录"
        self.assertIn(heading, rules)
        v4_rules_with_tail = rules.partition(heading)[2]
        self.assertIn(next_heading, v4_rules_with_tail)
        v4_rules = v4_rules_with_tail.partition(next_heading)[0]

        for required in (
            "`stream_termination=completed`",
            "MODEL_DOCTOR_CASE_046_OK",
            "weather, then time, then `MODEL_DOCTOR_CASE_047_OK`",
            "`MODEL_DOCTOR_CASE_048_OK` plus the exact `WEATHER_SUNNY`",
            "exactly one timeout retry",
            "MODEL_DOCTOR_CASE_049_OK",
            "every ordered request",
            "runtime-conformant",
            "final loop is completed",
        ):
            with self.subTest(required=required):
                self.assertIn(required, v4_rules)

        self.assertNotIn("protocolConformance", rules)

    def test_readme_describes_012_evidence_v4_and_46_checks(self) -> None:
        readme = (SKILL_DIR.parents[1] / "README.md").read_text(encoding="utf-8")

        for required in (
            "v0.12.0",
            "`llm-capability-doctor.evidence.v4`",
            "默认执行全部 46 个检测项",
            "输出完整 46 项目录",
            "evidence-v4 日志路径",
            "官方协议结构",
            "完整工具闭环",
            "`:generateContent`",
            "`:streamGenerateContent`",
            "`alt=sse`",
        ):
            with self.subTest(required=required):
                self.assertIn(required, readme)

    def test_parser_accepts_exact_v4_profile_with_46_manifests(self) -> None:
        parsed = self.parse_text_log(build_evidence_log())

        self.assertEqual(46, len(parsed["tests"]))
        self.assertNotIn("collection_profile", parsed["run"])
        self.assertEqual(
            COMPATIBILITY_PROFILE,
            parsed["run"]["compatibilityProfile"],
        )
        self.assertEqual(
            "llm-capability-doctor.evidence.v4",
            parsed["run"]["log_schema"],
        )
        with self.assertRaisesRegex(ValueError, "collection_profile"):
            self.parse_text_log(
                build_evidence_log(collection_profile="full"),
                "profile-v4.log",
            )
        count_mutations = {
            "selected": ("selected_test_count: 46", "selected_test_count: 45"),
            "requests": ("request_count: 1", "request_count: 0"),
            "manifests": ("test_manifest_count: 46", "test_manifest_count: 45"),
        }
        for name, (before, after) in count_mutations.items():
            with self.subTest(count=name), self.assertRaisesRegex(
                ValueError,
                "does not match discovered",
            ):
                self.parse_text_log(
                    build_evidence_log().replace(before, after, 1),
                    f"{name}-count-v4.log",
                )

    def test_parser_rejects_missing_or_unknown_v4_profile(self) -> None:
        for name, profile in {
            "missing": None,
            "unknown": "opencodex-next",
        }.items():
            with self.subTest(name=name), self.assertRaisesRegex(
                ValueError,
                "Unsupported or missing compatibility_profile",
            ):
                self.parse_text_log(
                    build_evidence_log(compatibility_profile=profile),
                    f"{name}-profile-v4.log",
                )

    def test_parser_rejects_v4_missing_046(self) -> None:
        with self.assertRaisesRegex(ValueError, "46"):
            self.parse_text_log(
                build_evidence_log(test_ids=set(RETAINED_TEST_IDS) - {"046"}),
                "missing-046.log",
            )

    def test_parser_rejects_mixed_v4_contract_variants(self) -> None:
        variants = (
            ("llm-capability-doctor.evidence.v4", "0.10.0"),
            ("llm-capability-doctor.evidence.v2", "0.12.0"),
        )
        for schema, version in variants:
            with self.subTest(schema=schema, version=version):
                with self.assertRaisesRegex(ValueError, "schema/version pair"):
                    self.parse_text_log(
                        build_evidence_log(
                            log_schema=schema,
                            script_version=version,
                        ),
                        "mixed.log",
                    )

    def test_parser_validates_v4_request_metadata(self) -> None:
        parsed = self.parse_text_log(build_evidence_log())

        request = parsed["requests"]["test-046-turn-1"]
        self.assertEqual(valid_v4_request_metadata(), {
            field: request[field] for field in valid_v4_request_metadata()
        })

        for field in valid_v4_request_metadata():
            metadata = valid_v4_request_metadata()
            del metadata[field]
            with self.subTest(missing=field), self.assertRaisesRegex(
                ValueError,
                field,
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    f"missing-{field}.log",
                )

        invalid_enums = {
            "transport_outcome": "http_error",
            "stream_termination": "clean",
            "stream_end_signal": "finishReason:",
            "tool_contract_status": "valid",
            "tool_loop_outcome": "stopped",
        }
        for field, value in invalid_enums.items():
            metadata = valid_v4_request_metadata()
            metadata[field] = value
            with self.subTest(invalid_enum=field), self.assertRaisesRegex(
                ValueError,
                field,
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    f"invalid-{field}.log",
                )

        valid_enums = {
            "transport_outcome": (
                "completed_eof",
                "protocol_terminated",
                "timeout",
                "upstream_disconnect",
                "client_cancelled",
                "transport_error",
            ),
            "stream_termination": (
                "not_applicable",
                "completed",
                "missing_terminal_event",
                "timeout",
                "upstream_disconnect",
                "client_cancelled",
                "transport_error",
                "http_error",
                "malformed_stream",
                "protocol_error",
                "model_incomplete",
            ),
            "stream_end_signal": (
                "none",
                "[DONE]",
                "response.completed",
                "message_stop",
                "finishReason:STOP",
                "done:true",
            ),
            "tool_contract_status": (
                "not_applicable",
                "conformant",
                "non_conformant",
            ),
            "tool_loop_outcome": (
                "not_applicable",
                "continued",
                "completed",
                "invalid_turn",
                "transport_failure",
                "max_turns_exceeded",
            ),
        }
        for field, values in valid_enums.items():
            for value in values:
                metadata = valid_v4_request_metadata()
                metadata[field] = value
                if field == "tool_contract_status" and value == "non_conformant":
                    metadata["tool_contract_errors_json"] = '["fixture.error:/"]'
                with self.subTest(valid_enum=field, value=value):
                    parsed_variant = self.parse_text_log(
                        build_evidence_log(request_metadata=metadata),
                        f"valid-{field}.log",
                    )
                    self.assertEqual(
                        value,
                        parsed_variant["requests"]["test-046-turn-1"][field],
                    )

        for field in ("stream_event_count", "tool_loop_turn"):
            for value in ("", "00", "+1", "-1", " 1", "1 "):
                metadata = valid_v4_request_metadata()
                metadata[field] = value
                with self.subTest(integer=field, value=value), self.assertRaisesRegex(
                    ValueError,
                    field,
                ):
                    self.parse_text_log(
                        build_evidence_log(request_metadata=metadata),
                        f"invalid-{field}.log",
                    )

        metadata = valid_v4_request_metadata()
        metadata["model_stop_reason"] = ""
        with self.assertRaisesRegex(ValueError, "model_stop_reason"):
            self.parse_text_log(
                build_evidence_log(request_metadata=metadata),
                "empty-stop-reason.log",
            )

        invalid_error_arrays = (
            "[",
            "{}",
            "null",
            "1",
            '"error"',
            '[""]',
            "[1]",
            "[null]",
        )
        for value in invalid_error_arrays:
            metadata = valid_v4_request_metadata()
            metadata["tool_contract_errors_json"] = value
            with self.subTest(errors_json=value), self.assertRaisesRegex(
                ValueError,
                "tool_contract_errors_json",
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    "invalid-errors-json.log",
                )

        status_error_mismatches = (
            ("conformant", '["fixture.error:/"]'),
            ("non_conformant", "[]"),
        )
        for status, errors_json in status_error_mismatches:
            metadata = valid_v4_request_metadata()
            metadata["tool_contract_status"] = status
            metadata["tool_contract_errors_json"] = errors_json
            with self.subTest(status=status), self.assertRaisesRegex(
                ValueError,
                "tool_contract",
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    "invalid-status-errors.log",
                )

    def test_parser_redacts_discovered_secrets_from_v4_dynamic_metadata(self) -> None:
        secret = "sk-v4-metadata-secret-123456"
        metadata = valid_v4_request_metadata()
        metadata.update({
            "stream_end_signal": f"finishReason:STOP-{secret}",
            "model_stop_reason": f"stop-{secret}",
            "tool_contract_status": "non_conformant",
            "tool_contract_errors_json": json.dumps([
                f"anthropic.dynamic_error:{secret}"
            ]),
        })
        log = build_evidence_log(request_metadata=metadata).replace(
            "script_version: 0.12.0\n",
            f"script_version: 0.12.0\napi_key: {secret}\n",
            1,
        )

        parsed = self.parse_text_log(log, "v4-dynamic-metadata-secret.log")
        request = parsed["requests"]["test-046-turn-1"]

        self.assertNotIn(secret, json.dumps(parsed, ensure_ascii=False))
        self.assertEqual("[REDACTED]", parsed["run"]["api_key"])
        self.assertEqual(
            "finishReason:STOP-[REDACTED]",
            request["stream_end_signal"],
        )
        self.assertEqual("stop-[REDACTED]", request["model_stop_reason"])
        self.assertEqual(
            ["anthropic.dynamic_error:[REDACTED]"],
            json.loads(request["tool_contract_errors_json"]),
        )

    def test_contract_key_rejects_non_mapping_run_without_crashing(self) -> None:
        from model_doctor_contracts import V4_CONTRACT, contract_key  # noqa: PLC0415

        class UnhashableString(str):
            __hash__ = None

        self.assertEqual(
            V4_CONTRACT,
            contract_key({
                "log_schema": "llm-capability-doctor.evidence.v4",
                "script_version": "0.12.0",
            }),
        )
        malformed_runs = (
            None,
            [],
            "not-a-mapping",
            1,
            {},
            {"log_schema": "llm-capability-doctor.evidence.v4"},
            {
                "log_schema": None,
                "script_version": "0.12.0",
            },
            {
                "log_schema": ["llm-capability-doctor.evidence.v4"],
                "script_version": "0.12.0",
            },
            {
                "log_schema": "llm-capability-doctor.evidence.v4",
                "script_version": {"value": "0.12.0"},
            },
            {
                "log_schema": UnhashableString("llm-capability-doctor.evidence.v4"),
                "script_version": "0.12.0",
            },
            {
                "log_schema": "llm-capability-doctor.evidence.v4",
                "script_version": "0.10.0",
            },
        )
        for run in malformed_runs:
            with self.subTest(run=run):
                try:
                    contract_key(run)
                except ValueError:
                    pass
                except (AttributeError, TypeError) as error:
                    self.fail(f"contract_key crashed for {run!r}: {error}")
                else:
                    self.fail(f"contract_key accepted malformed run: {run!r}")


if __name__ == "__main__":
    unittest.main()
