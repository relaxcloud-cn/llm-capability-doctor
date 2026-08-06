from __future__ import annotations

import sys
import unittest
import json
import io
import re
import tempfile
from contextlib import redirect_stderr, redirect_stdout
from html.parser import HTMLParser
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
ASSET_DIR = SKILL_DIR / "assets"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_assessment import (  # noqa: E402
    assemble_assessment,
    validate_assessment,
    validate_reviews,
)
from model_doctor_html import render_report  # noqa: E402
from model_doctor_log import (  # noqa: E402
    RETAINED_TEST_IDS,
    _credential_is_masked,
    parse_log,
    redact_text,
)
from model_doctor_report import main  # noqa: E402


def _full_v2_log() -> str:
    manifests = "".join(
        "========== TEST-{0} BEGIN ==========\n"
        "name: fixture-{0}\n"
        "category: fixture\n"
        "request_refs: \n"
        "========== TEST-{0} END ==========\n".format(test_id)
        for test_id in sorted(RETAINED_TEST_IDS)
    )
    return (
        "========== MODEL DOCTOR RUN ==========\n"
        "script_version: 0.10.0\n"
        "section_encoding: base64\n"
        "log_schema: llm-capability-doctor.evidence.v2\n"
        "selected_test_count: 46\n"
        + manifests
        + "========== RUN SUMMARY ==========\n"
        "request_count: 0\n"
        "test_manifest_count: 46\n"
        "========== END ==========\n"
    )


class _MainChildParser(HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self.stack = []
        self.children = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str]]) -> None:
        if self.stack and self.stack[-1] == "main":
            attributes = dict(attrs)
            self.children.append((tag, attributes.get("class", "")))
        self.stack.append(tag)

    def handle_endtag(self, tag: str) -> None:
        if self.stack and self.stack[-1] == tag:
            self.stack.pop()


class ModelDoctorV6Tests(unittest.TestCase):
    def _parse_text_log(self, value: str, name: str) -> dict:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / name
        path.write_text(value, encoding="utf-8")
        return parse_log(path)

    def _request(self, request_id: str, content: str) -> dict:
        return {
            "request_id": request_id,
            "started_at": "2026-07-28T00:00:00+0800",
            "completed_at": "2026-07-28T00:00:01+0800",
            "protocol": "openai_chat",
            "auth_mode": "bearer",
            "stream": "0",
            "curlCommand": "curl --header 'Authorization: Bearer ${MODEL_API_KEY}'",
            "requestBody": '{"model":"fixture"}',
            "metrics": {
                "curl_exit_code": "0",
                "http_status": "200",
                "time_total": "0.100000",
                "time_starttransfer": "0.050000",
                "size_download": "64",
            },
            "responseHeaders": "content-type: application/json",
            "stderr": "",
            "responseBody": content,
        }

    def _parsed(self, include_dependent_fail: bool = False) -> dict:
        parsed = {
            "schemaVersion": "llm-capability-doctor.parsed-evidence.v1",
            "source": {
                "fileName": "fixture.log",
                "size": 128,
                "sha256": "0" * 64,
            },
            "run": {
                "url": "https://example.invalid/v1/chat/completions",
                "model": "fixture-model",
                "api_key": "[MASKED]",
            },
            "tokenTotals": {},
            "warnings": [],
            "tests": {
                "001": {
                    "category": "接口与协议",
                    "name": "通过样例",
                    "requestRefs": ["req-pass"],
                },
                "002": {
                    "category": "性能与稳定性",
                    "name": "失败样例",
                    "requestRefs": ["req-fail"],
                },
            },
            "requests": {
                "req-pass": self._request("req-pass", "EXPECTED"),
                "req-fail": self._request("req-fail", "UNEXPECTED"),
            },
        }
        if include_dependent_fail:
            parsed["tests"]["003"] = {
                "category": "性能与稳定性",
                "name": "依赖失败样例",
                "requestRefs": ["req-dependent"],
            }
            parsed["requests"]["req-dependent"] = self._request(
                "req-dependent", "NO_VALID_MEASUREMENT"
            )
        return parsed

    def _logic(self) -> dict:
        return {
            "purpose": "验证目标行为。",
            "method": "检查响应语义。",
            "passCriteria": ["响应满足目标契约。"],
            "failCriteria": ["响应不满足目标契约。"],
            "capabilityBoundary": "只证明本次固定输入。",
        }

    def _failure_analysis(self) -> dict:
        return {
            "failureKind": "DIRECT",
            "evidenceSufficiency": "SUFFICIENT",
            "supportedClaim": "响应直接违反了本项核心契约。",
            "unsupportedClaims": ["不能据此推断后端模型身份。"],
            "dependsOnTestIds": [],
            "evidenceRefs": ["request:req-fail"],
        }

    def _test_review(self, test_id: str, status: str) -> dict:
        request_id = "req-pass" if status == "PASS" else "req-fail"
        review = {
            "testId": test_id,
            "reviewedStatus": status,
            "conclusion": (
                "响应满足目标契约，因此判定本项能力通过。"
                if status == "PASS"
                else "响应违反目标契约，因此判定本项能力未通过。"
            ),
            "logic": self._logic(),
            "evidenceRefs": [f"request:{request_id}"],
            "evidenceExcerpts": ["记录了可观察响应。"],
            "limitations": [],
            "retestInstructions": [],
        }
        if status == "FAIL":
            review["failureAnalysis"] = self._failure_analysis()
        return review

    def _verified_facts(self) -> dict:
        return {
            "interfaceProtocol": {
                "evidenceState": "INCONCLUSIVE",
                "family": "UNKNOWN",
                "requestFormat": "已采集请求，但未形成已知协议结论。",
                "responseFormat": "已采集响应，但未形成已知协议结论。",
                "statement": "本轮证据不足以确认接口协议格式。",
                "evidenceRefs": ["request:req-fail"],
                "boundary": "不能从 URL 或模型名称猜测协议格式。",
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

    def _summary(self, include_dependent_fail: bool = False) -> dict:
        test_refs = ["002", "003"] if include_dependent_fail else ["002"]
        evidence_refs = (
            ["request:req-fail", "request:req-dependent"]
            if include_dependent_fail
            else ["request:req-fail"]
        )
        return {
            "headline": "基础能力可用，但存在一项有直接证据的问题。",
            "verifiedFacts": self._verified_facts(),
            "issues": [
                {
                    "title": "响应未满足契约",
                    "statement": "失败响应与要求不一致。",
                    "testRefs": test_refs,
                    "evidenceRefs": evidence_refs,
                    "boundary": "本次证据不支持扩大归因。",
                }
            ],
            "scopeBoundary": "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。",
        }

    def _reviews(self, include_dependent_fail: bool = False) -> dict:
        reviews = {
            "schemaVersion": "llm-capability-doctor.reviews.v2",
            "tests": {
                "001": self._test_review("001", "PASS"),
                "002": self._test_review("002", "FAIL"),
            },
            "capabilitySummary": self._summary(include_dependent_fail),
        }
        if include_dependent_fail:
            dependent = self._test_review("003", "FAIL")
            dependent["evidenceRefs"] = ["request:req-dependent"]
            dependent["failureAnalysis"] = {
                "failureKind": "MEASUREMENT_UNAVAILABLE",
                "evidenceSufficiency": "LIMITED",
                "supportedClaim": "依赖样本失败导致指标不可测量。",
                "unsupportedClaims": ["不能据此判定延迟性能差。"],
                "dependsOnTestIds": ["002"],
                "evidenceRefs": ["request:req-dependent"],
            }
            reviews["tests"]["003"] = dependent
        return reviews

    def test_reviews_v2_requires_verified_facts(self) -> None:
        reviews = self._reviews()
        del reviews["capabilitySummary"]["verifiedFacts"]
        self.assertIn(
            "capabilitySummary verifiedFacts must be an object",
            validate_reviews(self._parsed(), reviews),
        )

    def test_verified_context_rejects_unbounded_max_claim(self) -> None:
        reviews = self._reviews()
        context = reviews["capabilitySummary"]["verifiedFacts"]["contextWindow"]
        context.update(
            evidenceState="VERIFIED",
            highestVerifiedTier="32K Token 近似档",
            highestVerifiedInputTokens=80175,
            statement="最大上下文是 80175 Token。",
            evidenceRefs=["request:req-fail"],
        )
        errors = validate_reviews(self._parsed(), reviews)
        self.assertTrue(any("unbounded maximum claim" in error for error in errors))

    def test_not_collected_rejects_values_and_evidence(self) -> None:
        reviews = self._reviews()
        concurrency = reviews["capabilitySummary"]["verifiedFacts"]["concurrency"]
        concurrency["highestVerifiedConcurrentRequests"] = 32
        concurrency["evidenceRefs"] = ["request:req-fail"]
        errors = validate_reviews(self._parsed(), reviews)
        self.assertTrue(any("NOT_COLLECTED" in error for error in errors))

    def test_v5_requires_failure_analysis_for_every_fail(self) -> None:
        reviews = self._reviews()
        del reviews["tests"]["002"]["failureAnalysis"]

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn("Test 002 failureAnalysis is required for FAIL", errors)

    def test_v5_rejects_failure_analysis_on_pass(self) -> None:
        reviews = self._reviews()
        reviews["tests"]["001"]["failureAnalysis"] = self._failure_analysis()

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn("Test 001 failureAnalysis is only allowed for FAIL", errors)

    def test_limited_failure_requires_unsupported_claim(self) -> None:
        reviews = self._reviews()
        analysis = reviews["tests"]["002"]["failureAnalysis"]
        analysis["evidenceSufficiency"] = "LIMITED"
        analysis["unsupportedClaims"] = []

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "Test 002 LIMITED failureAnalysis requires unsupportedClaims",
            errors,
        )

    def test_summary_must_cover_every_fail(self) -> None:
        parsed = self._parsed(include_dependent_fail=True)
        reviews = self._reviews(include_dependent_fail=True)
        reviews["capabilitySummary"]["issues"][0]["testRefs"] = ["002"]
        reviews["capabilitySummary"]["issues"][0]["evidenceRefs"] = [
            "request:req-fail"
        ]

        errors = validate_reviews(parsed, reviews)

        self.assertIn("capabilitySummary does not cover FAIL tests: 003", errors)

    def test_summary_evidence_must_belong_to_referenced_fail(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["issues"][0]["evidenceRefs"] = [
            "request:req-pass"
        ]

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "capabilitySummary issue 1 evidence does not belong to its testRefs: "
            "request:req-pass",
            errors,
        )

    def test_dependency_must_reference_another_fail(self) -> None:
        reviews = self._reviews()
        reviews["tests"]["002"]["failureAnalysis"]["dependsOnTestIds"] = ["001"]

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "Test 002 dependency must reference another FAIL test: 001",
            errors,
        )

    def test_dependent_failures_must_share_one_summary_issue(self) -> None:
        parsed = self._parsed(include_dependent_fail=True)
        reviews = self._reviews(include_dependent_fail=True)
        reviews["capabilitySummary"]["issues"] = [
            {
                "title": "直接失败",
                "statement": "基础请求未满足契约。",
                "testRefs": ["002"],
                "evidenceRefs": ["request:req-fail"],
                "boundary": "不扩大归因。",
            },
            {
                "title": "派生失败",
                "statement": "依赖样本不足，指标不可测量。",
                "testRefs": ["003"],
                "evidenceRefs": ["request:req-dependent"],
                "boundary": "不判定性能优劣。",
            },
        ]

        errors = validate_reviews(parsed, reviews)

        self.assertIn(
            "Dependent FAIL tests must share one capabilitySummary issue: 003 -> 002",
            errors,
        )

    def test_summary_rejects_project_readiness_decisions(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["headline"] = "BLOCKED：不可上线。"
        reviews["capabilitySummary"]["issues"][0]["statement"] = (
            "本模型可上线。"
        )

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "capabilitySummary headline must not contain project readiness decisions",
            errors,
        )
        self.assertIn(
            "capabilitySummary issue 1 statement must not contain project readiness decisions",
            errors,
        )

        reviews = self._reviews()
        reviews["capabilitySummary"]["headline"] = "模型READY状态未判定。"
        self.assertIn(
            "capabilitySummary headline must not contain project readiness decisions",
            validate_reviews(self._parsed(), reviews),
        )

        reviews["capabilitySummary"]["headline"] = "Evidence was already recorded."
        self.assertNotIn(
            "capabilitySummary headline must not contain project readiness decisions",
            validate_reviews(self._parsed(), reviews),
        )

    def test_v5_rejects_duplicate_contract_references(self) -> None:
        parsed = self._parsed(include_dependent_fail=True)
        reviews = self._reviews(include_dependent_fail=True)
        reviews["tests"]["003"]["failureAnalysis"]["dependsOnTestIds"] = [
            "002",
            "002",
        ]
        reviews["capabilitySummary"]["issues"][0]["testRefs"] = [
            "002",
            "003",
            "003",
        ]

        errors = validate_reviews(parsed, reviews)

        self.assertIn("Test 003 dependsOnTestIds must contain unique values", errors)
        self.assertIn(
            "capabilitySummary issue 1 testRefs must contain unique values",
            errors,
        )

    def test_fail_cannot_be_counted_in_multiple_summary_issues(self) -> None:
        reviews = self._reviews()
        duplicate_issue = dict(reviews["capabilitySummary"]["issues"][0])
        duplicate_issue["title"] = "重复问题"
        reviews["capabilitySummary"]["issues"].append(duplicate_issue)

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "FAIL test must appear in exactly one capabilitySummary issue: 002",
            errors,
        )

    def test_assessment_rechecks_dependency_grouping_and_unique_refs(self) -> None:
        assessment = assemble_assessment(
            self._parsed(include_dependent_fail=True),
            self._reviews(include_dependent_fail=True),
        )
        assessment["capabilitySummary"]["issues"] = [
            {
                "title": "直接失败",
                "statement": "基础请求未满足契约。",
                "testRefs": ["002"],
                "evidenceRefs": ["request:req-fail"],
                "boundary": "不扩大归因。",
            },
            {
                "title": "派生失败",
                "statement": "依赖样本不足，指标不可测量。",
                "testRefs": ["003", "003"],
                "evidenceRefs": ["request:req-dependent"],
                "boundary": "不判定性能优劣。",
            },
        ]

        errors = validate_assessment(assessment)

        self.assertIn(
            "capabilitySummary issue 2 testRefs must contain unique values",
            errors,
        )
        self.assertIn(
            "Dependent FAIL tests must share one capabilitySummary issue: 003 -> 002",
            errors,
        )

    def test_assembly_carries_failure_analysis_and_capability_summary(self) -> None:
        try:
            assessment = assemble_assessment(self._parsed(), self._reviews())
        except KeyError as error:
            self.fail(f"assembly rejected the reviews v2 envelope: {error}")

        self.assertEqual(
            "llm-capability-doctor.assessment.v6",
            assessment["schemaVersion"],
        )
        self.assertNotIn("failureAnalysis", assessment["tests"][0])
        self.assertEqual(
            "SUFFICIENT",
            assessment["tests"][1]["failureAnalysis"]["evidenceSufficiency"],
        )
        self.assertEqual(self._summary(), assessment["capabilitySummary"])
        self.assertEqual([], validate_assessment(assessment))

    def test_assessment_rejects_uncovered_fail(self) -> None:
        try:
            assessment = assemble_assessment(self._parsed(), self._reviews())
        except KeyError as error:
            self.fail(f"assembly rejected the reviews v2 envelope: {error}")
        assessment["capabilitySummary"]["issues"] = []

        errors = validate_assessment(assessment)

        self.assertIn(
            "capabilitySummary must contain 1 to 5 issues when FAIL tests exist",
            errors,
        )

    def test_assessment_rejects_non_object_test_without_crashing(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())
        assessment["tests"][0] = "not-an-object"

        try:
            errors = validate_assessment(assessment)
        except AttributeError as error:
            self.fail(f"malformed assessment caused validator crash: {error}")

        self.assertIn("tests contain a non-object item", errors)

    def test_review_validation_rejects_malformed_parsed_test_without_crashing(
        self,
    ) -> None:
        parsed = self._parsed()
        parsed["tests"]["001"] = "not-an-object"

        try:
            errors = validate_reviews(parsed, self._reviews())
        except AttributeError as error:
            self.fail(f"malformed parsed evidence caused validator crash: {error}")

        self.assertIn("Parsed test 001 must be an object", errors)

    def test_assessment_missing_status_returns_error_without_crashing(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())
        del assessment["tests"][0]["reviewedStatus"]

        try:
            errors = validate_assessment(assessment)
        except KeyError as error:
            self.fail(f"malformed assessment caused validator crash: {error}")

        self.assertIn("tests contain a non-binary reviewedStatus", errors)

    def test_parser_rejects_empty_manifest_category(self) -> None:
        log = """========== MODEL DOCTOR RUN ==========
log_schema: llm-capability-doctor.evidence.v1
script_version: 0.9.0
section_encoding: base64
collection_profile: custom
selected_test_count: 1
url: https://example.invalid/v1/chat/completions
model: fixture
api_key: [MASKED]
========== TEST-002 BEGIN ==========
name: 协议识别
category:
request_refs:
========== TEST-002 END ==========
========== RUN SUMMARY ==========
request_count: 0
test_manifest_count: 1
========== END ==========
"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty-category.log"
            path.write_text(log, encoding="utf-8")

            with self.assertRaisesRegex(
                ValueError,
                "TEST-002 category must be non-empty",
            ):
                parse_log(path)

    def test_parser_accepts_complete_profile_free_v2(self) -> None:
        parsed = self._parse_text_log(_full_v2_log(), "full-v2.log")

        self.assertEqual(
            "llm-capability-doctor.evidence.v2",
            parsed["run"]["log_schema"],
        )
        self.assertNotIn("collection_profile", parsed["run"])
        self.assertEqual(46, len(parsed["tests"]))

    def test_parser_rejects_invalid_v2_contract_variants(self) -> None:
        complete = _full_v2_log()
        variants = {
            "profile": complete.replace(
                "section_encoding: base64\n",
                "section_encoding: base64\ncollection_profile: full\n",
            ),
            "mixed": complete.replace(
                "script_version: 0.10.0",
                "script_version: 0.9.0",
            ),
            "incomplete": re.sub(
                r"^========== TEST-060 BEGIN ==========\n.*?"
                r"^========== TEST-060 END ==========\n",
                "",
                complete.replace("selected_test_count: 46", "selected_test_count: 45")
                .replace("test_manifest_count: 46", "test_manifest_count: 45"),
                count=1,
                flags=re.MULTILINE | re.DOTALL,
            ),
        }
        messages = {
            "profile": "must not contain collection_profile",
            "mixed": "schema/version pair",
            "incomplete": "must contain all 46 retained tests",
        }

        for name, value in variants.items():
            with self.subTest(name=name), self.assertRaisesRegex(
                ValueError,
                messages[name],
            ):
                self._parse_text_log(value, f"{name}.log")

    def test_redaction_handles_curl_headers_and_json_cookies(self) -> None:
        secret = "sk-live-secret-123456"
        text = (
            "curl -H 'Authorization: Bearer "
            + secret
            + "' https://example.invalid\n"
            + json.dumps({"cookie": "session=" + secret})
        )

        redacted = redact_text(text)

        self.assertNotIn(secret, redacted)
        self.assertIn("Authorization: [REDACTED]", redacted)
        self.assertIn('"cookie": "[REDACTED]"', redacted)

    def test_redaction_does_not_replace_common_scalar_echoes(self) -> None:
        text = '{"token":"true","enabled":true,"answer":"true"}'

        redacted = redact_text(text)

        self.assertEqual(
            '{"token":"[REDACTED]","enabled":true,"answer":"true"}',
            redacted,
        )
        self.assertTrue(json.loads(redacted)["enabled"])

    def test_partial_asterisks_are_not_an_approved_api_key_mask(self) -> None:
        self.assertTrue(_credential_is_masked("********"))
        self.assertFalse(_credential_is_masked("sk-****-still-secret"))

    def test_assessment_schema_declares_v5_summary_contract(self) -> None:
        schema_path = SKILL_DIR / "references" / "assessment-schema.json"
        schema = json.loads(schema_path.read_text(encoding="utf-8"))

        self.assertEqual(
            "llm-capability-doctor.assessment.v5",
            schema["properties"]["schemaVersion"]["const"],
        )
        self.assertIn("capabilitySummary", schema["required"])
        test_schema = schema["properties"]["tests"]["items"]
        self.assertIn("failureAnalysis", test_schema["properties"])
        self.assertEqual(
            1,
            test_schema["properties"]["category"]["minLength"],
        )
        self.assertIn(
            "pattern",
            schema["$defs"]["capabilitySummary"]["properties"]["headline"],
        )
        pattern = schema["$defs"]["capabilitySummary"]["properties"][
            "headline"
        ]["pattern"]
        self.assertIsNone(re.fullmatch(pattern, "模型READY状态未判定。"))
        self.assertIsNotNone(re.fullmatch(pattern, "Evidence was already recorded."))

    def test_renderer_places_final_conclusion_after_detection_information(self) -> None:
        try:
            assessment = assemble_assessment(self._parsed(), self._reviews())
            html = render_report(assessment, ASSET_DIR)
        except (KeyError, ValueError) as error:
            self.fail(f"renderer rejected the desired v5 contract: {error}")

        expected_order = (
            '<section class="run-information"',
            '<section class="final-conclusion"',
            '<table class="summary-table">',
        )
        positions = tuple(html.find(marker) for marker in expected_order)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)
        self.assertEqual(1, html.count('<section class="final-conclusion"'))
        parser = _MainChildParser()
        parser.feed(html)
        self.assertEqual(
            [
                ("section", "run-information"),
                ("section", "final-conclusion"),
                ("table", "summary-table"),
            ],
            parser.children[:3],
        )

    def test_only_fail_rows_render_evidence_review(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        self.assertEqual(1, html.count("未通过项证据复核"))
        self.assertIn("证据充分", html)
        self.assertIn("响应直接违反了本项核心契约。", html)
        self.assertIn("不能据此推断后端模型身份。", html)

    def test_renderer_shows_decisive_evidence_refs_kind_and_metrics(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        self.assertIn("关键证据摘录", html)
        self.assertIn("记录了可观察响应。", html)
        self.assertIn("失败类型", html)
        self.assertIn("DIRECT", html)
        self.assertIn("证据引用", html)
        self.assertIn("request:req-fail", html)
        self.assertIn("curl exit 0", html)
        self.assertIn("TTFT 0.050000s", html)

    def test_print_css_reveals_evidence_and_removes_code_clipping(self) -> None:
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")
        print_css = css.split("@media print", 1)[1]

        self.assertRegex(
            print_css,
            r"\.evidence-row\[hidden\]\s*\{\s*display:\s*table-row;",
        )
        self.assertRegex(print_css, r"pre\s*\{[^}]*max-height:\s*none;")
        self.assertRegex(print_css, r"pre\s*\{[^}]*overflow:\s*visible;")

    def test_summary_renders_issue_references_and_boundary(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        self.assertIn("响应未满足契约", html)
        self.assertIn("关联检测项：002", html)
        self.assertIn("本次证据不支持扩大归因。", html)
        self.assertIn(
            "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。",
            html,
        )

    def test_summary_and_failure_analysis_escape_dynamic_text(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["issues"][0]["statement"] = (
            "<script>alert(1)</script>"
        )
        reviews["tests"]["002"]["failureAnalysis"]["supportedClaim"] = (
            "<b>bad</b>"
        )
        assessment = assemble_assessment(self._parsed(), reviews)

        html = render_report(assessment, ASSET_DIR)

        self.assertNotIn("<script>alert(1)</script>", html)
        self.assertIn("&lt;script&gt;alert(1)&lt;/script&gt;", html)
        self.assertIn("&lt;b&gt;bad&lt;/b&gt;", html)

    def test_all_pass_summary_renders_without_empty_issue_list(self) -> None:
        parsed = self._parsed()
        del parsed["tests"]["002"]
        reviews = self._reviews()
        del reviews["tests"]["002"]
        verified_facts = self._verified_facts()
        verified_facts["interfaceProtocol"].update(
            evidenceState="NOT_COLLECTED",
            requestFormat="本轮未采集可确认的请求格式。",
            responseFormat="本轮未采集可确认的响应格式。",
            statement="本轮未采集接口协议证据。",
            evidenceRefs=[],
        )
        reviews["capabilitySummary"] = {
            "headline": "本轮所有已执行检测项均通过。",
            "verifiedFacts": verified_facts,
            "issues": [],
            "scopeBoundary": (
                "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。"
            ),
        }
        assessment = assemble_assessment(parsed, reviews)

        html = render_report(assessment, ASSET_DIR)

        self.assertIn("本轮所有已执行检测项均通过。", html)
        self.assertNotIn('<ol class="final-conclusion-list">', html)

    def test_cli_rejects_non_object_reviews_with_v2_message(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            directory_path = Path(directory)
            parsed_path = directory_path / "parsed.json"
            reviews_path = directory_path / "reviews.json"
            parsed_path.write_text(
                json.dumps(self._parsed(), ensure_ascii=False),
                encoding="utf-8",
            )
            reviews_path.write_text("[]", encoding="utf-8")
            stdout = io.StringIO()
            stderr = io.StringIO()

            with redirect_stdout(stdout), redirect_stderr(stderr):
                exit_code = main(["validate", str(parsed_path), str(reviews_path)])

        self.assertEqual(2, exit_code)
        self.assertIn(
            "Reviews must use llm-capability-doctor.reviews.v2",
            stderr.getvalue(),
        )

    def test_cli_render_writes_v6_and_final_conclusion(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            directory_path = Path(directory)
            parsed_path = directory_path / "parsed.json"
            reviews_path = directory_path / "reviews.json"
            assessment_path = directory_path / "assessment.json"
            html_path = directory_path / "report.html"
            parsed_path.write_text(
                json.dumps(self._parsed(), ensure_ascii=False),
                encoding="utf-8",
            )
            reviews_path.write_text(
                json.dumps(self._reviews(), ensure_ascii=False),
                encoding="utf-8",
            )
            stdout = io.StringIO()
            stderr = io.StringIO()

            with redirect_stdout(stdout), redirect_stderr(stderr):
                exit_code = main(
                    [
                        "render",
                        str(parsed_path),
                        str(reviews_path),
                        "--assessment",
                        str(assessment_path),
                        "--html",
                        str(html_path),
                    ]
                )

            assessment = json.loads(assessment_path.read_text(encoding="utf-8"))
            html = html_path.read_text(encoding="utf-8")

        self.assertEqual(0, exit_code, stderr.getvalue())
        self.assertEqual(
            "llm-capability-doctor.assessment.v6",
            assessment["schemaVersion"],
        )
        self.assertIn('<section class="final-conclusion"', html)
        self.assertEqual(1, html.count("未通过项证据复核"))

    def test_skill_instructions_require_the_three_stage_flow(self) -> None:
        skill_text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("failureAnalysis", skill_text)
        self.assertIn("capabilitySummary", skill_text)
        self.assertIn("检测信息", skill_text)
        self.assertIn("不得手工修改渲染后的 HTML", skill_text)
        self.assertNotIn("assessment.v4", skill_text)

    def test_evaluation_rules_define_evidence_sufficiency_and_grouping(self) -> None:
        rules_text = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")

        for marker in (
            "SUFFICIENT",
            "LIMITED",
            "INSUFFICIENT",
            "CONTRACT_FACET",
            "dependsOnTestIds",
            "最多五项",
        ):
            self.assertIn(marker, rules_text)


if __name__ == "__main__":
    unittest.main()
