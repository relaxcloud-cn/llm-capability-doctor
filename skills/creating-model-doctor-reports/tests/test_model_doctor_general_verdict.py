from __future__ import annotations

import sys
import unittest
import json
from copy import deepcopy
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_general_verdict import (  # noqa: E402
    CORE_TEST_IDS,
    ENHANCED_TEST_IDS,
    derive_general_verdict,
)
from model_doctor_contracts import (  # noqa: E402
    CONTRACT_TEST_IDS,
    CONTRACT_VERDICT_PARTITIONS,
    V4_CONTRACT,
    V4_TEST_IDS,
)
from model_doctor_assessment import (  # noqa: E402
    assemble_assessment,
    validate_assessment,
    validate_reviews,
)
from model_doctor_log import RETAINED_TEST_IDS  # noqa: E402
from model_doctor_html import _capability_summary  # noqa: E402
from model_doctor_opencodex_compatibility import (  # noqa: E402
    COMPATIBILITY_PROFILE,
    REQUIRED_TEST_IDS,
    SCOPE_BOUNDARY as OPENCODEX_SCOPE_BOUNDARY,
    derive_opencodex_compatibility,
)


class GeneralVerdictTests(unittest.TestCase):
    def setUp(self) -> None:
        self.all_pass = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}

    def test_partition_covers_all_retained_tests_once(self) -> None:
        self.assertEqual(CORE_TEST_IDS & ENHANCED_TEST_IDS, frozenset())
        self.assertEqual(
            CORE_TEST_IDS | ENHANCED_TEST_IDS,
            frozenset(RETAINED_TEST_IDS),
        )
        self.assertEqual(len(CORE_TEST_IDS), 32)
        self.assertEqual(len(ENHANCED_TEST_IDS), 14)

    def test_all_checks_pass(self) -> None:
        verdict = derive_general_verdict(self.all_pass, V4_CONTRACT)

        self.assertEqual(verdict["level"], "PASS")
        self.assertEqual(verdict["label"], "通用能力通过")
        self.assertEqual(verdict["collectedTests"], 46)
        self.assertEqual(verdict["passedTests"], 46)
        self.assertEqual(verdict["passedCoreTests"], 32)
        self.assertEqual(verdict["passedEnhancedTests"], 14)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测全部通过，因此判定通用能力通过。",
        )

    def test_enhanced_failure_is_conditional_pass(self) -> None:
        statuses = dict(self.all_pass)
        statuses["060"] = "FAIL"

        verdict = derive_general_verdict(statuses, V4_CONTRACT)

        self.assertEqual(verdict["level"], "CONDITIONAL_PASS")
        self.assertEqual(verdict["label"], "通用能力有条件通过")
        self.assertEqual(verdict["passedTests"], 45)
        self.assertEqual(verdict["passedCoreTests"], 32)
        self.assertEqual(verdict["passedEnhancedTests"], 13)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测通过 45 项，32 项基础必过项全部通过；"
            "1 项增强能力存在限制，因此判定通用能力有条件通过。",
        )

    def test_core_failure_is_fail(self) -> None:
        statuses = dict(self.all_pass)
        statuses["001"] = "FAIL"

        verdict = derive_general_verdict(statuses, V4_CONTRACT)

        self.assertEqual(verdict["level"], "FAIL")
        self.assertEqual(verdict["label"], "通用能力未通过")
        self.assertEqual(verdict["passedTests"], 45)
        self.assertEqual(verdict["passedCoreTests"], 31)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测通过 45 项，其中 1 项基础必过能力未满足，"
            "因此判定通用能力未通过。",
        )

    def test_core_failure_takes_precedence_over_enhanced_failure(self) -> None:
        statuses = dict(self.all_pass)
        statuses["001"] = "FAIL"
        statuses["060"] = "FAIL"

        verdict = derive_general_verdict(statuses, V4_CONTRACT)

        self.assertEqual(verdict["level"], "FAIL")
        self.assertEqual(verdict["passedTests"], 44)
        self.assertEqual(verdict["passedCoreTests"], 31)
        self.assertEqual(verdict["passedEnhancedTests"], 13)

    def test_partial_collection_is_not_assessed(self) -> None:
        verdict = derive_general_verdict({"001": "PASS"}, V4_CONTRACT)

        self.assertEqual(verdict["level"], "NOT_ASSESSED")
        self.assertEqual(verdict["label"], "通用能力未评定")
        self.assertEqual(verdict["collectedTests"], 1)
        self.assertEqual(verdict["passedTests"], 1)
        self.assertEqual(
            verdict["statement"],
            "本轮仅采集 1/46 项，证据不足以生成通用能力等级，"
            "因此本轮通用能力未评定。",
        )

    def test_invalid_status_is_rejected(self) -> None:
        statuses = dict(self.all_pass)
        statuses["001"] = "UNKNOWN"

        with self.assertRaisesRegex(ValueError, "Invalid status for test 001"):
            derive_general_verdict(statuses, V4_CONTRACT)

    def test_v4_partition_has_32_core_and_14_enhanced(self) -> None:
        core_ids, enhanced_ids = CONTRACT_VERDICT_PARTITIONS[V4_CONTRACT]

        self.assertEqual(32, len(core_ids))
        self.assertEqual(14, len(enhanced_ids))
        self.assertIn("046", core_ids)
        self.assertEqual(frozenset(), core_ids & enhanced_ids)
        self.assertEqual(CONTRACT_TEST_IDS[V4_CONTRACT], core_ids | enhanced_ids)

    def test_v4_all_checks_pass_uses_46_totals(self) -> None:
        statuses = {
            test_id: "PASS" for test_id in CONTRACT_TEST_IDS[V4_CONTRACT]
        }

        verdict = derive_general_verdict(statuses, V4_CONTRACT)

        self.assertEqual("PASS", verdict["level"])
        self.assertEqual(46, verdict["totalTests"])
        self.assertEqual(46, verdict["passedTests"])
        self.assertEqual(32, verdict["totalCoreTests"])
        self.assertEqual(32, verdict["passedCoreTests"])
        self.assertEqual(14, verdict["totalEnhancedTests"])
        self.assertEqual(
            "本轮固定 46 项检测全部通过，因此判定通用能力通过。",
            verdict["statement"],
        )

    def test_v4_046_failure_is_core_failure(self) -> None:
        statuses = {
            test_id: "PASS" for test_id in CONTRACT_TEST_IDS[V4_CONTRACT]
        }
        statuses["046"] = "FAIL"

        verdict = derive_general_verdict(statuses, V4_CONTRACT)

        self.assertEqual("FAIL", verdict["level"])
        self.assertEqual(31, verdict["passedCoreTests"])
        self.assertEqual(14, verdict["passedEnhancedTests"])
        self.assertIn("1 项基础必过能力未满足", verdict["statement"])

    def test_v2_all_checks_pass_keeps_46_totals(self) -> None:
        verdict = derive_general_verdict(self.all_pass, V4_CONTRACT)

        self.assertEqual("PASS", verdict["level"])
        self.assertEqual(46, verdict["totalTests"])
        self.assertEqual(32, verdict["totalCoreTests"])
        self.assertEqual(14, verdict["totalEnhancedTests"])

    def test_v1_partial_uses_legacy_46_denominator(self) -> None:
        verdict = derive_general_verdict({"001": "PASS"}, V4_CONTRACT)

        self.assertEqual("NOT_ASSESSED", verdict["level"])
        self.assertEqual(46, verdict["totalTests"])
        self.assertEqual(32, verdict["totalCoreTests"])
        self.assertIn("1/46", verdict["statement"])


class GeneralVerdictAssessmentTests(unittest.TestCase):
    def _logic(self) -> dict:
        return {
            "purpose": "验证固定能力。",
            "method": "检查本项记录。",
            "passCriteria": ["满足目标契约。"],
            "failCriteria": ["未满足目标契约。"],
            "capabilityBoundary": "只证明本轮固定输入。",
        }

    def _verified_facts(self) -> dict:
        return {
            "interfaceProtocol": {
                "evidenceState": "NOT_COLLECTED",
                "family": "UNKNOWN",
                "requestFormat": "本轮未采集接口请求格式。",
                "responseFormat": "本轮未采集接口响应格式。",
                "statement": "本轮未采集接口协议证据。",
                "evidenceRefs": [],
                "boundary": "未采集时不能推断接口协议。",
            },
            "contextWindow": {
                "evidenceState": "NOT_COLLECTED",
                "highestVerifiedTier": None,
                "highestVerifiedInputTokens": None,
                "firstFailedTier": None,
                "firstFailedInputTokens": None,
                "statement": "本轮未采集上下文档位证据。",
                "evidenceRefs": [],
                "boundary": "未采集时不能推断上下文上限。",
            },
            "concurrency": {
                "evidenceState": "NOT_COLLECTED",
                "highestVerifiedConcurrentRequests": None,
                "statement": "本轮未采集并发波次证据。",
                "evidenceRefs": [],
                "boundary": "未采集时不能推断并发上限。",
            },
        }

    def _fixture(
        self,
        statuses: dict[str, str],
        contract: tuple[str, str],
    ) -> tuple[dict, dict]:
        tests = {
            test_id: {
                "category": "固定能力",
                "name": f"检测项 {test_id}",
                "requestRefs": [],
            }
            for test_id in statuses
        }
        reviews = {}
        fail_ids = []
        for test_id, status in statuses.items():
            review = {
                "testId": test_id,
                "reviewedStatus": status,
                "conclusion": (
                    "本项满足目标契约。"
                    if status == "PASS"
                    else "本项未满足目标契约。"
                ),
                "logic": self._logic(),
                "evidenceRefs": [f"test:{test_id}:manifest"],
                "evidenceExcerpts": ["本项保留了检测清单证据。"],
                "limitations": [],
                "retestInstructions": [],
            }
            if status == "FAIL":
                fail_ids.append(test_id)
                review["failureAnalysis"] = {
                    "failureKind": "DIRECT",
                    "evidenceSufficiency": "SUFFICIENT",
                    "supportedClaim": "本项未满足固定契约。",
                    "unsupportedClaims": ["不能扩大推断到未测试场景。"],
                    "dependsOnTestIds": [],
                    "evidenceRefs": [f"test:{test_id}:manifest"],
                }
            reviews[test_id] = review

        issues = []
        if fail_ids:
            issues.append(
                {
                    "title": "固定能力存在限制",
                    "statement": "本轮有检测项未满足固定契约。",
                    "testRefs": fail_ids,
                    "evidenceRefs": [
                        f"test:{test_id}:manifest" for test_id in fail_ids
                    ],
                    "boundary": "结论仅覆盖关联检测项。",
                }
            )
        parsed = {
            "schemaVersion": "llm-capability-doctor.parsed-evidence.v1",
            "source": {
                "fileName": "fixture.log",
                "size": 1,
                "sha256": "0" * 64,
            },
            "run": {
                "model": "fixture-model",
                "api_key": "[MASKED]",
                "log_schema": contract[0],
                "script_version": contract[1],
            },
            "tokenTotals": {},
            "warnings": [],
            "tests": tests,
            "requests": {},
        }
        authored_reviews = {
            "schemaVersion": "llm-capability-doctor.reviews.v2",
            "tests": reviews,
            "capabilitySummary": {
                "headline": "本轮固定检测结果已完成证据复核。",
                "verifiedFacts": self._verified_facts(),
                "issues": issues,
                "scopeBoundary": (
                    "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。"
                ),
            },
        }
        return parsed, authored_reviews

    def _v4_fixture(
        self,
        statuses: dict[str, str] | None = None,
        family: str = "OPENAI_CHAT_COMPLETIONS",
    ) -> tuple[dict, dict]:
        final_statuses = statuses or {
            test_id: "PASS" for test_id in V4_TEST_IDS
        }
        parsed, reviews = self._fixture(final_statuses, V4_CONTRACT)
        parsed["run"].update(
            {
                "compatibilityProfile": COMPATIBILITY_PROFILE,
            }
        )
        parsed["requests"]["protocol"] = {
            "request_id": "protocol",
            "metrics": {},
        }
        parsed["tests"]["002"]["requestRefs"] = ["protocol"]
        reviews["tests"]["002"]["evidenceRefs"] = ["request:protocol"]
        reviews["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"] = {
            "evidenceState": "VERIFIED",
            "family": family,
            "requestFormat": "OpenCodex 协议请求格式。",
            "responseFormat": "OpenCodex 协议响应格式。",
            "statement": "本轮已验证接口协议。",
            "evidenceRefs": ["request:protocol"],
            "boundary": "仅覆盖本轮模型端接口。",
        }
        for test_id, total_turns in {
            "046": 2,
            "047": 3,
            "048": 2,
            "049": 3,
        }.items():
            if final_statuses.get(test_id) != "PASS":
                continue
            request_refs = [
                f"test-{test_id}-turn-{turn}"
                for turn in range(1, total_turns + 1)
            ]
            parsed["tests"][test_id]["requestRefs"] = request_refs
            parsed["requests"].update(
                {
                    request_id: self._v4_tool_request(
                        test_id,
                        turn,
                        total_turns,
                    )
                    for turn, request_id in enumerate(request_refs, start=1)
                }
            )
            reviews["tests"][test_id]["evidenceRefs"] = [
                f"request:{request_id}" for request_id in request_refs
            ]
        return parsed, reviews

    @staticmethod
    def _v4_tool_request(test_id: str, turn: int, total_turns: int) -> dict:
        request_id = f"test-{test_id}-turn-{turn}"
        body = {
            "model": "gpt-test",
            "messages": [
                {
                    "role": "user",
                    "content": f"MODEL_DOCTOR_CASE_{test_id}",
                }
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
        for previous_turn in range(1, turn):
            call_id = f"call_{test_id}_{previous_turn}"
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
                        "content": "WEATHER_SUNNY",
                    },
                ]
            )
        final = turn == total_turns
        fixture_name = "openai_chat_final.sse" if final else "openai_chat_tool.sse"
        response_body = (
            SKILL_DIR.parents[1] / "src" / "protocol" / "fixtures" / fixture_name
        ).read_text(encoding="utf-8")
        if final:
            response_body = response_body.replace(
                "MODEL_DOCTOR_CASE_046_OK",
                f"MODEL_DOCTOR_CASE_{test_id}_OK",
            )
        else:
            response_body = response_body.replace(
                "call_weather_046",
                f"call_{test_id}_{turn}",
            )
        return {
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

    def test_assembler_injects_general_verdict_without_mutating_reviews(self) -> None:
        statuses = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}
        for test_id in ("017", "018", "020", "024", "035", "036"):
            statuses[test_id] = "FAIL"
        parsed, reviews = self._v4_fixture(statuses)

        assessment = assemble_assessment(parsed, reviews)

        self.assertNotIn("generalVerdict", reviews["capabilitySummary"])
        self.assertEqual(
            derive_general_verdict(statuses, V4_CONTRACT),
            assessment["capabilitySummary"]["generalVerdict"],
        )
        self.assertEqual([], validate_assessment(assessment))

    def test_assembler_injects_v4_opencodex_compatibility_without_mutating_reviews(
        self,
    ) -> None:
        parsed, reviews = self._v4_fixture()
        original_reviews = deepcopy(reviews)

        assessment = assemble_assessment(parsed, reviews)

        statuses = {
            item["testId"]: item["reviewedStatus"]
            for item in assessment["tests"]
        }
        expected = derive_opencodex_compatibility(
            assessment["run"],
            statuses,
            "OPENAI_CHAT_COMPLETIONS",
        )
        self.assertEqual(
            expected,
            assessment["capabilitySummary"]["openCodexCompatibility"],
        )
        self.assertEqual("PASS", expected["level"])
        self.assertEqual(original_reviews, reviews)
        self.assertEqual([], validate_assessment(assessment))

    def test_partial_v4_evidence_assembles_not_assessed_compatibility(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        compatibility = assessment["capabilitySummary"][
            "openCodexCompatibility"
        ]
        self.assertEqual("NOT_ASSESSED", compatibility["level"])
        self.assertEqual([], compatibility["failedTestIds"])
        self.assertEqual([], validate_assessment(assessment))

    def test_v4_required_failure_assembles_failed_compatibility(self) -> None:
        statuses = {test_id: "PASS" for test_id in V4_TEST_IDS}
        statuses["004"] = "FAIL"
        parsed, reviews = self._v4_fixture(statuses)

        assessment = assemble_assessment(parsed, reviews)

        compatibility = assessment["capabilitySummary"][
            "openCodexCompatibility"
        ]
        self.assertEqual("FAIL", compatibility["level"])
        self.assertEqual(["004"], compatibility["failedTestIds"])
        self.assertEqual([], validate_assessment(assessment))

    def test_reviews_cannot_author_opencodex_compatibility(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)
        reviews["capabilitySummary"]["openCodexCompatibility"] = {}

        self.assertIn(
            "capabilitySummary field openCodexCompatibility is not allowed",
            validate_reviews(parsed, reviews),
        )

    def test_reviews_reject_non_object_parsed_run_before_assembly(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)
        parsed["run"] = []

        self.assertIn("Parsed run must be an object", validate_reviews(parsed, reviews))
        with self.assertRaisesRegex(ValueError, "Parsed run must be an object"):
            assemble_assessment(parsed, reviews)

    def test_assessment_rejects_every_tampered_opencodex_field(self) -> None:
        parsed, reviews = self._v4_fixture()
        assessment = assemble_assessment(parsed, reviews)
        invalid_values = {
            "profile": "opencodex-next",
            "level": "FAIL",
            "label": "人工改写标签",
            "protocolFamily": "OLLAMA_CHAT",
            "requiredTestIds": ["002"],
            "failedTestIds": ["002"],
            "statement": "人工改写结论。",
            "scopeBoundary": "人工扩大范围。",
        }

        for field, invalid_value in invalid_values.items():
            with self.subTest(field=field):
                tampered = deepcopy(assessment)
                tampered["capabilitySummary"]["openCodexCompatibility"][
                    field
                ] = invalid_value
                self.assertIn(
                    "capabilitySummary openCodexCompatibility does not match "
                    "run metadata, protocol family, and test statuses",
                    validate_assessment(tampered),
                )

    def test_assessment_recomputes_opencodex_compatibility_from_sources(self) -> None:
        parsed, reviews = self._v4_fixture()
        assessment = assemble_assessment(parsed, reviews)
        mutations = (
            lambda value: value["run"].update(
                {"compatibilityProfile": "opencodex-next"}
            ),
            lambda value: next(
                item for item in value["tests"] if item["testId"] == "002"
            ).update({"reviewedStatus": "FAIL"}),
            lambda value: value["capabilitySummary"]["verifiedFacts"][
                "interfaceProtocol"
            ].update({"family": "OLLAMA_CHAT"}),
        )

        for mutate in mutations:
            with self.subTest(mutate=mutate):
                tampered = deepcopy(assessment)
                mutate(tampered)
                self.assertIn(
                    "capabilitySummary openCodexCompatibility does not match "
                    "run metadata, protocol family, and test statuses",
                    validate_assessment(tampered),
                )

    def test_assessment_validation_handles_non_object_run(self) -> None:
        parsed, reviews = self._v4_fixture()
        assessment = assemble_assessment(parsed, reviews)
        assessment["run"] = []

        errors = validate_assessment(assessment)

        self.assertIn("run must be an object", errors)
        self.assertIn(
            "capabilitySummary openCodexCompatibility does not match run metadata, "
            "protocol family, and test statuses",
            errors,
        )

    def test_reviews_cannot_author_general_verdict(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)
        reviews["capabilitySummary"]["generalVerdict"] = derive_general_verdict(
            {"001": "PASS"}, V4_CONTRACT
        )

        errors = validate_reviews(parsed, reviews)

        self.assertIn(
            "capabilitySummary field generalVerdict is not allowed",
            errors,
        )

    def test_partial_assessment_is_not_assessed(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        verdict = assessment["capabilitySummary"]["generalVerdict"]
        self.assertEqual("NOT_ASSESSED", verdict["level"])
        self.assertEqual(1, verdict["collectedTests"])
        self.assertEqual([], validate_assessment(assessment))

    def test_assessment_rejects_every_tampered_verdict_field(self) -> None:
        statuses = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}
        parsed, reviews = self._v4_fixture(statuses)
        assessment = assemble_assessment(parsed, reviews)
        invalid_values = {
            "level": "FAIL",
            "label": "通用能力未通过",
            "collectedTests": 45,
            "passedTests": 45,
            "totalTests": 45,
            "passedCoreTests": 30,
            "totalCoreTests": 30,
            "passedEnhancedTests": 13,
            "totalEnhancedTests": 13,
            "statement": "人工改写的结论。",
        }

        for field, invalid_value in invalid_values.items():
            with self.subTest(field=field):
                tampered = deepcopy(assessment)
                tampered["capabilitySummary"]["generalVerdict"][field] = invalid_value
                self.assertIn(
                    "capabilitySummary generalVerdict does not match test statuses",
                    validate_assessment(tampered),
                )

    def test_assessment_rejects_boolean_verdict_counts(self) -> None:
        parsed, reviews = self._fixture({"001": "PASS"}, V4_CONTRACT)
        assessment = assemble_assessment(parsed, reviews)
        count_fields = (
            "collectedTests",
            "passedTests",
            "totalTests",
            "passedCoreTests",
            "totalCoreTests",
            "passedEnhancedTests",
            "totalEnhancedTests",
        )

        for field in count_fields:
            with self.subTest(field=field):
                tampered = deepcopy(assessment)
                verdict = tampered["capabilitySummary"]["generalVerdict"]
                verdict[field] = bool(verdict[field])
                self.assertIn(
                    f"capabilitySummary generalVerdict {field} must be an integer",
                    validate_assessment(tampered),
                )

    def test_assessment_schema_requires_closed_general_verdict(self) -> None:
        schema = json.loads(
            (SKILL_DIR / "references" / "assessment-schema.json").read_text(
                encoding="utf-8"
            )
        )

        summary_schema = schema["$defs"]["capabilitySummary"]
        self.assertIn("generalVerdict", summary_schema["required"])
        self.assertEqual(
            {"$ref": "#/$defs/generalVerdict"},
            summary_schema["properties"]["generalVerdict"],
        )
        verdict_schema = schema["$defs"]["generalVerdict"]
        self.assertFalse(verdict_schema["additionalProperties"])
        self.assertEqual(1, len(verdict_schema["oneOf"]))
        self.assertEqual(
            ["PASS", "CONDITIONAL_PASS", "FAIL", "NOT_ASSESSED"],
            verdict_schema["properties"]["level"]["enum"],
        )

    def test_assessment_schema_requires_closed_opencodex_compatibility(self) -> None:
        schema = json.loads(
            (SKILL_DIR / "references" / "assessment-schema.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual("llm-capability-doctor.assessment.v9", schema["$id"])
        summary_schema = schema["$defs"]["capabilitySummary"]
        self.assertIn("openCodexCompatibility", summary_schema["required"])
        self.assertEqual(
            {"$ref": "#/$defs/openCodexCompatibility"},
            summary_schema["properties"]["openCodexCompatibility"],
        )
        compatibility_schema = schema["$defs"]["openCodexCompatibility"]
        self.assertFalse(compatibility_schema["additionalProperties"])
        self.assertEqual(
            {
                "profile",
                "level",
                "label",
                "protocolFamily",
                "requiredTestIds",
                "failedTestIds",
                "statement",
                "scopeBoundary",
            },
            set(compatibility_schema["required"]),
        )
        properties = compatibility_schema["properties"]
        self.assertEqual(COMPATIBILITY_PROFILE, properties["profile"]["const"])
        self.assertEqual(
            list(REQUIRED_TEST_IDS),
            properties["requiredTestIds"]["const"],
        )
        self.assertEqual(
            OPENCODEX_SCOPE_BOUNDARY,
            properties["scopeBoundary"]["const"],
        )
        self.assertEqual(
            ["PASS", "FAIL", "NOT_ASSESSED"],
            properties["level"]["enum"],
        )
        self.assertEqual(
            [
                "OpenCodex 数据格式兼容",
                "OpenCodex 数据格式不兼容",
                "OpenCodex 数据格式未评定",
            ],
            properties["label"]["enum"],
        )
        self.assertEqual(
            [
                "OPENAI_CHAT_COMPLETIONS",
                "OPENAI_RESPONSES",
                "ANTHROPIC_MESSAGES",
                "GEMINI_GENERATE_CONTENT",
                "OLLAMA_CHAT",
                "CUSTOM",
                "UNKNOWN",
            ],
            properties["protocolFamily"]["enum"],
        )
        failed_ids = properties["failedTestIds"]
        self.assertEqual(list(REQUIRED_TEST_IDS), failed_ids["items"]["enum"])
        self.assertTrue(failed_ids["uniqueItems"])
        self.assertEqual(8, failed_ids["maxItems"])


class GeneralVerdictHtmlTests(unittest.TestCase):
    def _summary(self, verdict: dict) -> dict:
        return {
            "generalVerdict": verdict,
            "headline": "已验证能力摘要。",
            "verifiedFacts": {
                "interfaceProtocol": {
                    "evidenceState": "VERIFIED",
                    "family": "OPENAI_CHAT_COMPLETIONS",
                    "requestFormat": "OpenAI Chat 请求格式。",
                    "responseFormat": "OpenAI Chat 响应格式。",
                    "statement": "本轮已验证 OpenAI Chat 接口。",
                    "evidenceRefs": ["request:protocol"],
                    "boundary": "仅覆盖本轮接口请求。",
                },
                "contextWindow": {
                    "evidenceState": "VERIFIED",
                    "highestVerifiedTier": "32K Token 近似档",
                    "highestVerifiedInputTokens": 80175,
                    "firstFailedTier": "64K Token 近似档",
                    "firstFailedInputTokens": 131072,
                    "statement": "最高通过 32K Token 近似档。",
                    "evidenceRefs": ["request:context"],
                    "boundary": "不代表真实硬上限。",
                },
                "concurrency": {
                    "evidenceState": "VERIFIED",
                    "highestVerifiedConcurrentRequests": 32,
                    "statement": "32 并发波次 32/32 成功且无 429。",
                    "evidenceRefs": ["request:concurrency"],
                    "boundary": "仅覆盖本轮短时并发。",
                },
            },
            "issues": [
                {
                    "title": "一项增强能力受限",
                    "statement": "固定样本未满足契约。",
                    "testRefs": ["060"],
                    "evidenceRefs": ["test:060:manifest"],
                    "boundary": "不扩大到未测试场景。",
                }
            ],
            "scopeBoundary": (
                "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。"
            ),
        }

    def _assessment(self, summary: dict, different: int = 4) -> dict:
        verdict = summary["generalVerdict"]
        collected = verdict.get("collectedTests", 0)
        passed = verdict.get("passedTests", 0)
        return {
            "summary": {
                "counts": {
                    "PASS": passed,
                    "FAIL": max(collected - passed, 0),
                }
            },
            "capabilitySummary": summary,
            "protocolConformance": {
                "summary": {
                    "totalRequests": 121,
                    "checkedRequests": 121,
                    "consistentRequests": 121 - different,
                    "differentRequests": different,
                }
            },
        }

    def test_final_conclusion_uses_fixed_customer_information_order(self) -> None:
        statuses = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}
        for test_id in ("017", "018", "020", "024", "035", "036"):
            statuses[test_id] = "FAIL"
        summary = self._summary(derive_general_verdict(statuses, V4_CONTRACT))

        html = _capability_summary(self._assessment(summary))

        markers = (
            "本次检测结果",
            '<p class="verdict-value">通用能力有条件通过</p>',
            summary["generalVerdict"]["statement"],
            "40 通过 · 6 未通过",
            "32 / 32 通过",
            "32K Token 近似档",
            "当前主要有 1 类问题：一项增强能力受限。",
            "OpenAI Chat",
            "32 并发",
        )
        positions = tuple(html.find(marker) for marker in markers)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)
        self.assertEqual(1, html.count('<p class="verdict-value">通用能力有条件通过</p>'))
        self.assertNotIn(summary["headline"], html)
        self.assertIn(summary["issues"][0]["title"], html)
        self.assertNotIn(summary["scopeBoundary"], html)

    def test_all_verdict_labels_render(self) -> None:
        scenarios = []
        all_pass = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}
        scenarios.append((all_pass, "通用能力通过"))
        conditional = dict(all_pass)
        conditional["060"] = "FAIL"
        scenarios.append((conditional, "通用能力有条件通过"))
        failed = dict(all_pass)
        failed["001"] = "FAIL"
        scenarios.append((failed, "通用能力未通过"))
        scenarios.append(({"001": "PASS"}, "通用能力未评定"))

        for statuses, label in scenarios:
            with self.subTest(label=label):
                summary = self._summary(
                    derive_general_verdict(
                        statuses,
                        V4_CONTRACT if len(statuses) < 46 else V4_CONTRACT,
                    )
                )
                html = _capability_summary(self._assessment(summary))
                self.assertIn(f'<p class="verdict-value">{label}</p>', html)

    def test_partial_report_shows_collected_count_instead_of_pass_ratio(self) -> None:
        summary = self._summary(derive_general_verdict({"001": "PASS"}, V4_CONTRACT))

        html = _capability_summary(self._assessment(summary))

        self.assertIn("已采集 1/46", html)
        self.assertNotIn("1/46 通过", html)

    def test_new_conclusion_fields_are_html_escaped(self) -> None:
        summary = self._summary(
            derive_general_verdict({"001": "PASS"}, V4_CONTRACT)
        )
        summary["generalVerdict"]["label"] = '<script data-x="1">label</script>'
        summary["generalVerdict"]["statement"] = "<b>statement</b>"
        summary["verifiedFacts"]["contextWindow"]["highestVerifiedTier"] = (
            "<img src=x onerror=alert(1)>"
        )

        html = _capability_summary(self._assessment(summary))

        self.assertNotIn("<script data-x", html)
        self.assertNotIn("<b>statement</b>", html)
        self.assertNotIn("<img src=x", html)
        self.assertIn("&lt;script data-x=&quot;1&quot;&gt;", html)
        self.assertIn("&lt;b&gt;statement&lt;/b&gt;", html)
        self.assertIn("&lt;img src=x onerror=alert(1)&gt;", html)

    def test_customer_verdict_css_has_responsive_and_print_rules(self) -> None:
        css = (SKILL_DIR / "assets" / "report.css").read_text(encoding="utf-8")

        for selector in (
            ".verdict-block",
            ".verdict-value",
            ".fact-strip",
            ".fact-strip > div",
        ):
            self.assertIn(selector, css)
        mobile = css[css.index("@media (max-width: 680px)") :]
        self.assertIn(".fact-strip", mobile)
        printing = css[css.index("@media print") :]
        self.assertIn(".verdict-block", printing)


class GeneralVerdictSkillContractTests(unittest.TestCase):
    def test_skill_forbids_authored_verdict_and_requires_program_output(self) -> None:
        skill = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")

        for required in (
            "`reviews.v2` 不得填写 `generalVerdict`",
            "总体等级由 `assemble_assessment` 程序生成",
            "32 项基础必过项",
            "14 项增强能力项",
            "通用能力通过",
            "通用能力有条件通过",
            "通用能力未通过",
            "通用能力未评定",
        ):
            self.assertIn(required, skill)

    def test_rules_define_fixed_gates_and_customer_statements(self) -> None:
        rules = (SKILL_DIR / "references" / "evaluation-rules.md").read_text(
            encoding="utf-8"
        )

        for required in (
            "## 6. Deterministic General Capability Verdict",
            "`reviews.v2` must not contain `generalVerdict`",
            "32 core checks",
            "14 enhanced checks",
            "`PASS`：46 项全部 PASS",
            "`CONDITIONAL_PASS`：32 项基础必过项全部 PASS",
            "`FAIL`：任意基础必过项 FAIL",
            "`NOT_ASSESSED`：仅用于内部验证不完整状态",
            "本轮固定 46 项检测全部通过，因此判定通用能力通过。",
            "不构成项目 READY/BLOCKED 或可上线/不可上线判定",
        ):
            self.assertIn(required, rules)

        core_section = rules.split("### Core checks (32)", 1)[1].split(
            "### Enhanced checks (14)", 1
        )[0]
        enhanced_section = rules.split("### Enhanced checks (14)", 1)[1].split(
            "### Fixed statements", 1
        )[0]
        for test_id in CORE_TEST_IDS:
            self.assertIn(f"`{test_id}`", core_section)
        for test_id in ENHANCED_TEST_IDS:
            self.assertIn(f"`{test_id}`", enhanced_section)


if __name__ == "__main__":
    unittest.main()
