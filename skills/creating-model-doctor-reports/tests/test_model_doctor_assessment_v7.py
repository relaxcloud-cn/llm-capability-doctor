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
from model_doctor_protocol_conformance import (  # noqa: E402
    BASELINE_DATE,
    DIFFERENCE_KINDS,
    RULE_SET_VERSION,
    SUPPORTED_PROTOCOLS,
    analyze_protocol_conformance,
)
from model_doctor_html import (  # noqa: E402
    _protocol_difference,
    _request_evidence,
    render_report,
)


ASSESSMENT_V7 = "llm-capability-doctor.assessment.v7"
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
        return parsed, reviews

    def _v3_tool_turn_fixture(self) -> tuple[dict, dict]:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)
        parsed["requests"] = {}
        for test_id in ("046", "047", "048", "049"):
            request_id = f"test-{test_id}-turn-1"
            parsed["requests"][request_id] = self._v3_request(
                request_id,
                self._chat_response(request_id),
            )
            parsed["tests"][test_id]["requestRefs"] = [request_id]
            reviews["tests"][test_id]["evidenceRefs"] = [f"request:{request_id}"]
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

    def test_assembly_generates_one_result_for_every_top_level_request(self) -> None:
        parsed, reviews = self._fixture()

        assessment = assemble_assessment(parsed, reviews)
        expected = analyze_protocol_conformance(parsed)

        self.assertIn("protocolConformance", assessment)
        self.assertEqual(expected, assessment["protocolConformance"])
        self.assertEqual(
            list(parsed["requests"]),
            [item["requestId"] for item in assessment["protocolConformance"]["results"]],
        )
        self.assertEqual(
            2,
            assessment["protocolConformance"]["summary"]["totalRequests"],
        )
        self.assertEqual(
            2,
            assessment["protocolConformance"]["summary"]["checkedRequests"],
        )
        expected_top_level_fields = {
            "schemaVersion",
            "generatedAt",
            "source",
            "run",
            "tokenTotals",
            "warnings",
            "summary",
            "capabilitySummary",
            "protocolConformance",
            "categories",
            "tests",
        }
        self.assertEqual(expected_top_level_fields, set(assessment))
        self.assertEqual(1, list(assessment).count("protocolConformance"))

    def test_protocol_differences_do_not_change_capability_results(self) -> None:
        consistent_parsed, consistent_reviews = self._fixture(
            second_request_is_different=False
        )
        different_parsed, different_reviews = self._fixture(
            second_request_is_different=True
        )

        consistent = assemble_assessment(consistent_parsed, consistent_reviews)
        different = assemble_assessment(different_parsed, different_reviews)

        self.assertIn("protocolConformance", consistent)
        self.assertIn("protocolConformance", different)
        self.assertEqual(
            2,
            consistent["protocolConformance"]["summary"]["consistentRequests"],
        )
        self.assertGreater(
            different["protocolConformance"]["summary"]["differentRequests"],
            0,
        )
        for field in ("summary", "categories", "capabilitySummary", "tests"):
            with self.subTest(field=field):
                self.assertEqual(consistent[field], different[field])
        self.assertEqual({"PASS": 1, "FAIL": 0}, different["summary"]["counts"])
        verdict = different["capabilitySummary"]["generalVerdict"]
        self.assertEqual(1, verdict["collectedTests"])
        self.assertEqual(1, verdict["passedTests"])

    def test_validate_assessment_requires_one_closed_top_level_result(self) -> None:
        parsed, reviews = self._fixture()
        assessment = assemble_assessment(parsed, reviews)

        for field in assessment:
            with self.subTest(missing=field):
                missing = deepcopy(assessment)
                missing.pop(field)
                self.assertTrue(
                    any(field in error for error in validate_assessment(missing)),
                    validate_assessment(missing),
                )

        extra = deepcopy(assessment)
        extra["protocolConformanceCopy"] = analyze_protocol_conformance(parsed)
        self.assertTrue(
            any(
                "protocolConformanceCopy" in error
                for error in validate_assessment(extra)
            )
        )

    def test_validate_assessment_rejects_tampered_conformance_invariants(self) -> None:
        parsed, reviews = self._fixture()
        assessment = assemble_assessment(parsed, reviews)
        self.assertIn("protocolConformance", assessment)
        self.assertEqual([], validate_assessment(assessment))

        def mutate_summary_count(value: dict) -> None:
            value["protocolConformance"]["summary"]["totalRequests"] += 1

        def duplicate_request_id(value: dict) -> None:
            results = value["protocolConformance"]["results"]
            results[1]["requestId"] = results[0]["requestId"]

        def contradict_status(value: dict) -> None:
            value["protocolConformance"]["results"][0]["status"] = "DIFFERENT"

        def use_unknown_difference_kind(value: dict) -> None:
            differences = value["protocolConformance"]["results"][1]["differences"]
            differences[0]["differenceKind"] = "UNKNOWN_DIFFERENCE"

        def alter_baseline(value: dict) -> None:
            value["protocolConformance"]["baselines"][0]["officialVersion"] = (
                "unfrozen-version"
            )

        def add_summary_field(value: dict) -> None:
            value["protocolConformance"]["summary"]["extra"] = 1

        def add_result_field(value: dict) -> None:
            value["protocolConformance"]["results"][0]["extra"] = True

        def add_difference_field(value: dict) -> None:
            value["protocolConformance"]["results"][1]["differences"][0][
                "extra"
            ] = True

        def remove_summary_field(value: dict) -> None:
            del value["protocolConformance"]["summary"]["checkedRequests"]

        def remove_result_field(value: dict) -> None:
            del value["protocolConformance"]["results"][0]["status"]

        def remove_difference_field(value: dict) -> None:
            del value["protocolConformance"]["results"][1]["differences"][0][
                "actual"
            ]

        mutations = {
            "summary count": mutate_summary_count,
            "duplicate request ID": duplicate_request_id,
            "status without differences": contradict_status,
            "unknown difference kind": use_unknown_difference_kind,
            "unpinned baseline": alter_baseline,
            "extra summary field": add_summary_field,
            "extra request result field": add_result_field,
            "extra difference field": add_difference_field,
            "missing summary field": remove_summary_field,
            "missing request result field": remove_result_field,
            "missing difference field": remove_difference_field,
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name):
                tampered = deepcopy(assessment)
                mutate(tampered)
                self.assertTrue(validate_assessment(tampered), name)

    def test_assessment_schema_closes_conformance_nested_objects_and_enums(self) -> None:
        schema = self._schema()

        self.assertEqual(ASSESSMENT_V7, schema["$id"])
        self.assertEqual(
            ASSESSMENT_V7,
            schema["properties"]["schemaVersion"]["const"],
        )
        self.assertEqual(1, schema["required"].count("protocolConformance"))
        self.assertIn("protocolConformance", schema["properties"])
        self.assertFalse(schema["additionalProperties"])

        conformance = self._resolve_ref(
            schema, schema["properties"]["protocolConformance"]
        )
        conformance_fields = {
            "ruleSetVersion",
            "baselineDate",
            "baselines",
            "summary",
            "results",
        }
        self._assert_closed_object(conformance, conformance_fields)
        self.assertEqual(
            RULE_SET_VERSION,
            conformance["properties"]["ruleSetVersion"]["const"],
        )
        self.assertEqual(
            BASELINE_DATE,
            conformance["properties"]["baselineDate"]["const"],
        )

        baselines = conformance["properties"]["baselines"]
        self.assertEqual(5, baselines["minItems"])
        self.assertEqual(5, baselines["maxItems"])
        baseline = self._resolve_ref(schema, baselines["items"])
        self._assert_closed_object(
            baseline,
            {
                "protocol",
                "officialVersion",
                "referenceDate",
                "sourceUrl",
                "supportingReferences",
            },
        )
        self.assertEqual(
            set(SUPPORTED_PROTOCOLS),
            set(baseline["properties"]["protocol"]["enum"]),
        )
        self.assertIn("allOf", baseline)
        frozen_branches = baseline["allOf"][0]["oneOf"]
        expected_baselines = analyze_protocol_conformance({})["baselines"]
        self.assertEqual(len(expected_baselines), len(frozen_branches))
        for expected, branch in zip(expected_baselines, frozen_branches):
            with self.subTest(frozen=expected["protocol"]):
                properties = branch["properties"]
                self.assertEqual(
                    set(expected),
                    set(branch["required"]),
                )
                for field, value in expected.items():
                    self.assertEqual(value, properties[field]["const"])

        summary = self._resolve_ref(schema, conformance["properties"]["summary"])
        self._assert_closed_object(
            summary,
            {
                "totalRequests",
                "checkedRequests",
                "consistentRequests",
                "differentRequests",
                "byProtocol",
                "byCheck",
                "byDifferenceKind",
            },
        )
        grouping_fields = {
            "byProtocol": {
                "protocol",
                "totalRequests",
                "consistentRequests",
                "differentRequests",
            },
            "byCheck": {
                "checkId",
                "totalRequests",
                "consistentRequests",
                "differentRequests",
            },
            "byDifferenceKind": {"differenceKind", "count"},
        }
        for property_name, fields in grouping_fields.items():
            with self.subTest(group=property_name):
                grouping = self._resolve_ref(
                    schema, summary["properties"][property_name]["items"]
                )
                self._assert_closed_object(grouping, fields)

        result = self._resolve_ref(
            schema, conformance["properties"]["results"]["items"]
        )
        self._assert_closed_object(
            result,
            {
                "requestId",
                "protocol",
                "checkIds",
                "stream",
                "httpStatus",
                "status",
                "differences",
            },
        )
        self.assertEqual(
            ["CONSISTENT", "DIFFERENT"],
            result["properties"]["status"]["enum"],
        )

        difference = self._resolve_ref(
            schema, result["properties"]["differences"]["items"]
        )
        self._assert_closed_object(
            difference,
            {
                "requestId",
                "protocol",
                "location",
                "differenceKind",
                "expected",
                "actual",
                "officialReference",
            },
        )
        self.assertEqual(
            set(DIFFERENCE_KINDS),
            set(difference["properties"]["differenceKind"]["enum"]),
        )

    def test_assessment_v7_schema_allows_only_contract_total_combinations(self) -> None:
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
                self.assertLess(html.index(escape(request[field])), html.index("请求输入"))
        self.assertIn("tool_contract_errors_json", html)
        self.assertIn("tool.call:&lt;invalid&gt;", html)
        self.assertGreater(html.index("tool.call:&lt;invalid&gt;"), html.index("请求输出"))

    def test_combined_v7_v3_has_protocol_conformance_and_47_32_15_totals(self) -> None:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        verdict = assessment["capabilitySummary"]["generalVerdict"]
        self.assertEqual((47, 32, 15), (
            verdict["totalTests"],
            verdict["totalCoreTests"],
            verdict["totalEnhancedTests"],
        ))
        self.assertEqual(analyze_protocol_conformance(parsed), assessment["protocolConformance"])
        self.assertEqual([], validate_assessment(assessment))

    def test_combined_v7_v2_preserves_protocol_conformance_and_46_31_15_totals(self) -> None:
        parsed, reviews = self._complete_fixture(V2_CONTRACT)

        assessment = assemble_assessment(parsed, reviews)

        verdict = assessment["capabilitySummary"]["generalVerdict"]
        self.assertEqual((46, 31, 15), (
            verdict["totalTests"],
            verdict["totalCoreTests"],
            verdict["totalEnhancedTests"],
        ))
        self.assertEqual(analyze_protocol_conformance(parsed), assessment["protocolConformance"])
        self.assertEqual([], validate_assessment(assessment))

    def test_v3_tool_turns_appear_once_in_protocol_conformance_with_check_ids(self) -> None:
        parsed, reviews = self._v3_tool_turn_fixture()

        assessment = assemble_assessment(parsed, reviews)

        results = assessment["protocolConformance"]["results"]
        self.assertEqual(4, len(results))
        for check_id in ("046", "047", "048", "049"):
            request_id = f"test-{check_id}-turn-1"
            matches = [item for item in results if item["requestId"] == request_id]
            self.assertEqual(1, len(matches), request_id)
            self.assertEqual([check_id], matches[0]["checkIds"])

    def test_combined_v7_schema_requires_conformance_and_contract_totals(self) -> None:
        schema = self._schema()

        self.assertEqual(ASSESSMENT_V7, schema["$id"])
        self.assertEqual(ASSESSMENT_V7, schema["properties"]["schemaVersion"]["const"])
        self.assertIn("protocolConformance", schema["required"])
        summary = schema["$defs"]["capabilitySummary"]
        self.assertIn("generalVerdict", summary["required"])
        verdict = schema["$defs"]["generalVerdict"]
        self.assertEqual(2, len(verdict["oneOf"]))

    def test_combined_v7_validator_rechecks_both_invariant_families(self) -> None:
        parsed, reviews = self._complete_fixture(V3_CONTRACT)
        tampered = assemble_assessment(parsed, reviews)
        tampered["capabilitySummary"]["generalVerdict"]["totalTests"] = 46
        tampered["protocolConformance"]["summary"]["totalRequests"] += 1

        errors = validate_assessment(tampered)

        self.assertIn(
            "capabilitySummary generalVerdict does not match test statuses",
            errors,
        )
        self.assertIn("summary totalRequests does not match results", errors)

    def test_report_places_official_conformance_before_capability_table(self) -> None:
        parsed, reviews = self._fixture()
        assessment = assemble_assessment(parsed, reviews)

        report = render_report(assessment, ASSET_DIR)

        markers = (
            '<section class="final-conclusion"',
            '<section class="protocol-conformance"',
            '<table class="summary-table">',
        )
        positions = tuple(report.find(marker) for marker in markers)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)
        self.assertEqual(1, report.count('class="protocol-conformance"'))
        self.assertIn("官方协议结构一致性", report)

    def test_report_renders_counts_groupings_and_pinned_baselines(self) -> None:
        parsed, reviews = self._fixture()
        assessment = assemble_assessment(parsed, reviews)
        conformance = assessment["protocolConformance"]

        report = render_report(assessment, ASSET_DIR)

        summary = conformance["summary"]
        metrics = {
            "totalRequests": "请求总数",
            "checkedRequests": "已检查",
            "consistentRequests": "完全一致",
            "differentRequests": "存在差异",
        }
        for field, label in metrics.items():
            with self.subTest(metric=field):
                self.assertIn(f'data-metric="{field}"', report)
                self.assertIn(label, report)
                self.assertIn(f">{summary[field]}<", report)

        for group_name in ("byProtocol", "byCheck", "byDifferenceKind"):
            self.assertIn(f'data-group="{group_name}"', report)
        for baseline in conformance["baselines"]:
            with self.subTest(protocol=baseline["protocol"]):
                self.assertIn(escape(baseline["protocol"]), report)
                self.assertIn(escape(baseline["officialVersion"]), report)
                self.assertIn(escape(baseline["referenceDate"]), report)
                source = escape(baseline["sourceUrl"], quote=True)
                self.assertIn(f'href="{source}"', report)

    def test_report_renders_each_request_and_every_difference_field(self) -> None:
        parsed, reviews = self._fixture()
        assessment = assemble_assessment(parsed, reviews)

        report = render_report(assessment, ASSET_DIR)

        for result in assessment["protocolConformance"]["results"]:
            request_id = escape(result["requestId"], quote=True)
            with self.subTest(request_id=request_id):
                self.assertEqual(
                    1,
                    report.count(f'data-request-id="{request_id}"'),
                )
                for field in (
                    "requestId",
                    "protocol",
                    "checkIds",
                    "stream",
                    "httpStatus",
                    "status",
                ):
                    self.assertIn(f'data-field="{field}"', report)
                for difference in result["differences"]:
                    fragment = _protocol_difference(difference)
                    for field in ("requestId", "protocol"):
                        self.assertIn(f'data-field="{field}"', fragment)
                        self.assertIn(escape(difference[field]), fragment)
                    for field in (
                        "location",
                        "differenceKind",
                        "expected",
                        "actual",
                        "officialReference",
                    ):
                        self.assertIn(f'data-field="{field}"', report)
                        self.assertIn(escape(difference[field]), report)
                    reference = escape(
                        difference["officialReference"], quote=True
                    )
                    self.assertIn(f'href="{reference}"', report)

    def test_report_conformance_details_are_responsive_and_print_visible(self) -> None:
        parsed, reviews = self._fixture()
        report = render_report(assemble_assessment(parsed, reviews), ASSET_DIR)
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")

        self.assertNotIn("<link ", report)
        self.assertNotIn("<script src=", report)
        for selector in (
            ".protocol-conformance",
            ".protocol-conformance-counts",
            ".protocol-request",
            ".protocol-differences",
        ):
            self.assertIn(selector, css)
        mobile = css.split("@media (max-width: 640px)", 1)[1].split(
            "@media print", 1
        )[0]
        printed = css.split("@media print", 1)[1]
        self.assertIn(".protocol-conformance-counts", mobile)
        self.assertIn(".protocol-request", mobile)
        self.assertIn(".protocol-request", printed)
        self.assertIn(".protocol-differences", printed)

    def test_report_rejects_legacy_assessment_without_conformance(self) -> None:
        parsed, reviews = self._fixture()
        legacy = assemble_assessment(parsed, reviews)
        legacy["schemaVersion"] = "llm-capability-doctor.assessment.v6"
        legacy.pop("protocolConformance")

        with self.assertRaisesRegex(ValueError, "protocolConformance"):
            render_report(legacy, ASSET_DIR)

    def test_workflow_documents_official_structure_evaluation_rules(self) -> None:
        paths = (
            SKILL_DIR / "SKILL.md",
            SKILL_DIR / "references" / "evaluation-rules.md",
            SKILL_DIR.parents[1] / "README.md",
        )
        texts = [path.read_text(encoding="utf-8") for path in paths]
        combined = "\n".join(texts)

        for path, text in zip(paths, texts):
            with self.subTest(path=path.name):
                self.assertIn("llm-capability-doctor.assessment.v7", text)
                self.assertNotIn("llm-capability-doctor.assessment.v6", text)
        for rule in (
            "全部原始请求",
            "成功与错误响应",
            "流式与非流式响应",
            "仅比较官方协议数据结构",
            "可选字段可以缺失",
            "未记录的额外字段属于差异",
            "证据缺口不得判为一致",
            "explicitly recorded empty response",
            "requests not referenced by any manifest use `checkIds: []`",
            "both `CONSISTENT` and `DIFFERENT`",
            "不得归一化或修正响应",
        ):
            with self.subTest(rule=rule):
                self.assertIn(rule, combined)

    def test_runtime_contract_messages_use_assessment_v7(self) -> None:
        source = (
            SCRIPT_DIR / "model_doctor_assessment.py"
        ).read_text(encoding="utf-8")

        self.assertIn("assessment.v7 contract", source)
        self.assertNotIn("assessment.v6 contract", source)


if __name__ == "__main__":
    unittest.main()
