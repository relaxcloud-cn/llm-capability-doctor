from __future__ import annotations

import sys
import unittest
import json
import io
import re
import tempfile
from contextlib import redirect_stderr, redirect_stdout
from html import escape
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
from model_doctor_html import (  # noqa: E402
    _opencodex_compatibility,
    _verified_facts,
    render_report,
)
from model_doctor_log import (  # noqa: E402
    RETAINED_TEST_IDS,
    _credential_is_masked,
    parse_log,
    redact_text,
)
from model_doctor_report import main  # noqa: E402
from model_doctor_verified_facts import validate_verified_facts  # noqa: E402


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
    VOID_ELEMENTS = {
        "area",
        "base",
        "br",
        "col",
        "embed",
        "hr",
        "img",
        "input",
        "link",
        "meta",
        "source",
        "track",
        "wbr",
    }

    def __init__(self) -> None:
        super().__init__()
        self.stack = []
        self.children = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str]]) -> None:
        if self.stack and self.stack[-1] == "main":
            attributes = dict(attrs)
            self.children.append((tag, attributes.get("class", "")))
        if tag not in self.VOID_ELEMENTS:
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
                "log_schema": "llm-capability-doctor.evidence.v1",
                "script_version": "0.9.0",
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

    def _complete_fact_fixture(self) -> tuple[dict, dict]:
        def completion_response(
            request_id: str,
            content: str,
            prompt_tokens: int,
        ) -> str:
            return json.dumps(
                {
                    "id": f"chatcmpl-{request_id}",
                    "object": "chat.completion",
                    "created": 1785427200,
                    "model": "fixture-model",
                    "choices": [
                        {
                            "index": 0,
                            "message": {"role": "assistant", "content": content},
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {
                        "prompt_tokens": prompt_tokens,
                        "completion_tokens": 1,
                        "total_tokens": prompt_tokens + 1,
                    },
                },
                ensure_ascii=False,
            )

        wave_request_ids = [
            f"concurrency-32-{index}" for index in range(1, 33)
        ]
        concurrency_requests = {}
        for index, request_id in enumerate(wave_request_ids, start=1):
            marker = f"WAVE-32-{index:02d}"
            request = self._request(
                request_id,
                completion_response(request_id, marker, 12),
            )
            request["requestBody"] = json.dumps(
                {
                    "model": "fixture-model",
                    "messages": [
                        {
                            "role": "system",
                            "content": "Return only the exact marker requested by the user.",
                        },
                        {
                            "role": "user",
                            "content": f"Return exactly this wave marker: {marker}",
                        },
                    ],
                    "temperature": 0,
                    "max_tokens": 16,
                },
                ensure_ascii=False,
            )
            concurrency_requests[request_id] = request

        requests = {
            "protocol-openai": self._request(
                "protocol-openai",
                json.dumps(
                    {
                        "id": "chatcmpl-fixture",
                        "object": "chat.completion",
                        "choices": [
                            {
                                "index": 0,
                                "message": {"role": "assistant", "content": "pong"},
                                "finish_reason": "stop",
                            }
                        ],
                    },
                    ensure_ascii=False,
                ),
            ),
            "context-pass": self._request(
                "context-pass",
                completion_response("context-pass", "EXPECTED_SENTINEL", 80175),
            ),
            "context-fail": self._request(
                "context-fail",
                completion_response("context-fail", "NOT_FOUND", 131072),
            ),
        }
        requests.update(concurrency_requests)
        requests["protocol-openai"]["requestBody"] = json.dumps(
            {
                "model": "fixture-model",
                "messages": [{"role": "user", "content": "ping"}],
            },
            ensure_ascii=False,
        )
        for request_id, tier in (
            ("context-pass", "32K Token 近似档"),
            ("context-fail", "64K Token 近似档"),
        ):
            requests[request_id]["requestBody"] = json.dumps(
                {
                    "model": "fixture-model",
                    "messages": [
                        {
                            "role": "system",
                            "content": "Return exactly the required target sentinel and no other text.",
                        },
                        {
                            "role": "user",
                            "content": (
                                f"Context-tier intent: {tier}\n"
                                "Required target sentinel: EXPECTED_SENTINEL"
                            ),
                        },
                    ],
                    "temperature": 0,
                    "max_tokens": 16,
                },
                ensure_ascii=False,
            )
        requests["context-pass"]["metrics"].update(
            context_tier="32K Token 近似档",
        )
        requests["context-fail"]["metrics"].update(
            context_tier="64K Token 近似档",
        )
        context_prompt_tokens = {
            request_id: json.loads(requests[request_id]["responseBody"])["usage"][
                "prompt_tokens"
            ]
            for request_id in ("context-pass", "context-fail")
        }

        parsed = {
            "schemaVersion": "llm-capability-doctor.parsed-evidence.v1",
            "source": {
                "fileName": "complete-facts.log",
                "size": 4096,
                "sha256": "1" * 64,
            },
            "run": {
                "url": "https://example.invalid/v1/chat/completions",
                "model": "fixture-model",
                "api_key": "[MASKED]",
                "log_schema": "llm-capability-doctor.evidence.v1",
                "script_version": "0.9.0",
            },
            "tokenTotals": {},
            "warnings": [],
            "tests": {
                "002": {
                    "category": "接口与协议",
                    "name": "OpenAI Chat Completions 协议结构",
                    "requestRefs": ["protocol-openai"],
                },
                "016": {
                    "category": "上下文能力",
                    "name": "32K Token 近似档",
                    "requestRefs": ["context-pass"],
                },
                "017": {
                    "category": "上下文能力",
                    "name": "64K Token 近似档",
                    "requestRefs": ["context-fail"],
                },
                "057": {
                    "category": "性能与稳定性",
                    "name": "32 路短时并发",
                    "requestRefs": wave_request_ids,
                },
            },
            "requests": requests,
        }

        def review(test_id: str, status: str, request_id: str, excerpt: str) -> dict:
            value = {
                "testId": test_id,
                "reviewedStatus": status,
                "conclusion": (
                    "响应满足本项可观察契约，因此判定通过。"
                    if status == "PASS"
                    else "响应未返回目标哨兵，因此判定未通过。"
                ),
                "logic": self._logic(),
                "evidenceRefs": [f"request:{request_id}"],
                "evidenceExcerpts": [excerpt],
                "limitations": [],
                "retestInstructions": [],
            }
            if status == "FAIL":
                value["failureAnalysis"] = {
                    "failureKind": "DIRECT",
                    "evidenceSufficiency": "SUFFICIENT",
                    "supportedClaim": "usage 记录 131072 输入 Token，但响应未返回目标哨兵。",
                    "unsupportedClaims": [
                        "不能据此声称服务拒绝该输入或推断精确硬上限。"
                    ],
                    "dependsOnTestIds": [],
                    "evidenceRefs": [f"request:{request_id}"],
                }
            return value

        verified = {
            "interfaceProtocol": {
                "evidenceState": "VERIFIED",
                "family": "OPENAI_CHAT_COMPLETIONS",
                "requestFormat": "messages 数组与 role/content 消息。",
                "responseFormat": "choices[].message 与 chat.completion 结构。",
                "statement": "请求和响应采用 OpenAI Chat Completions 格式，不是 Anthropic 或自定义格式。",
                "evidenceRefs": ["request:protocol-openai"],
                "boundary": "协议结构不证明底层商业模型身份。",
            },
            "contextWindow": {
                "evidenceState": "VERIFIED",
                "highestVerifiedTier": "32K Token 近似档",
                "highestVerifiedInputTokens": context_prompt_tokens["context-pass"],
                "firstFailedTier": "64K Token 近似档",
                "firstFailedInputTokens": context_prompt_tokens["context-fail"],
                "statement": "最高已验证 80175 输入 Token，131072 输入 Token 的更高档首次失败。",
                "evidenceRefs": ["request:context-pass", "request:context-fail"],
                "boundary": "最高已验证值不是硬上限，真实上限未测试。",
            },
            "concurrency": {
                "evidenceState": "VERIFIED",
                "highestVerifiedConcurrentRequests": 32,
                "statement": "短时并发最高已验证到 32 个同时请求。",
                "evidenceRefs": [
                    f"request:{request_id}" for request_id in wave_request_ids
                ],
                "boundary": "32 是最高已验证波次，不代表服务硬上限或持续负载能力。",
            },
        }
        reviews = {
            "schemaVersion": "llm-capability-doctor.reviews.v2",
            "tests": {
                "002": review(
                    "002",
                    "PASS",
                    "protocol-openai",
                    "messages 请求对应 chat.completion 与 choices[].message 响应。",
                ),
                "016": review(
                    "016", "PASS", "context-pass", "80175 输入 Token 请求成功。"
                ),
                "017": review(
                    "017",
                    "FAIL",
                    "context-fail",
                    "usage.prompt_tokens=131072，响应内容为 NOT_FOUND。",
                ),
                "057": review(
                    "057",
                    "PASS",
                    "concurrency-32-1",
                    "32 路短时并发波次全部成功。",
                ),
            },
            "capabilitySummary": {
                "headline": "协议与短时并发证据通过，更高上下文档位语义验证失败。",
                "verifiedFacts": verified,
                "issues": [
                    {
                        "title": "更高上下文档位语义验证失败",
                        "statement": "131072 输入 Token 被计入 usage，但未返回目标哨兵。",
                        "testRefs": ["017"],
                        "evidenceRefs": ["request:context-fail"],
                        "boundary": "只证明本轮该档语义验证失败，不代表服务拒绝输入或达到硬上限。",
                    }
                ],
                "scopeBoundary": "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。",
            },
        }
        reviews["tests"]["057"]["evidenceRefs"] = [
            f"request:{request_id}" for request_id in wave_request_ids
        ]
        return parsed, reviews

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

    def test_not_collected_rejects_each_collected_fact_domain(self) -> None:
        facts = self._verified_facts()
        facts["interfaceProtocol"].update(
            evidenceState="NOT_COLLECTED",
            family="UNKNOWN",
            evidenceRefs=[],
        )
        domains = {
            "interfaceProtocol": {"request:req-002"},
            "contextWindow": {"request:req-014"},
            "concurrency": {"request:req-057"},
        }

        errors = validate_verified_facts(facts, domains)

        for fact_name in domains:
            self.assertIn(
                f"{fact_name} NOT_COLLECTED is invalid when its evidence domain "
                "contains collected requests",
                errors,
            )
        self.assertEqual(
            [],
            validate_verified_facts(
                facts,
                {fact_name: set() for fact_name in domains},
            ),
        )

    def test_reviews_reject_not_collected_for_collected_protocol(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"].update(
            evidenceState="NOT_COLLECTED",
            family="UNKNOWN",
            evidenceRefs=[],
        )

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn(
            "interfaceProtocol NOT_COLLECTED is invalid when its evidence domain "
            "contains collected requests",
            errors,
        )

    def test_custom_protocol_requires_verified_state(self) -> None:
        for state, rejected in (("INCONCLUSIVE", True), ("VERIFIED", False)):
            with self.subTest(state=state):
                reviews = self._reviews()
                protocol = reviews["capabilitySummary"]["verifiedFacts"][
                    "interfaceProtocol"
                ]
                protocol.update(evidenceState=state, family="CUSTOM")

                errors = validate_reviews(self._parsed(), reviews)

                expected = "interfaceProtocol CUSTOM requires evidenceState VERIFIED"
                if rejected:
                    self.assertIn(expected, errors)
                else:
                    self.assertNotIn(expected, errors)

    def test_bounded_hard_limit_phrase_is_allowed(self) -> None:
        parsed = self._parsed()
        parsed["tests"]["057"] = {
            "category": "性能与稳定性",
            "name": "并发响应时间",
            "requestRefs": ["req-pass"],
        }
        reviews = self._reviews()
        reviews["tests"]["057"] = self._test_review("057", "PASS")
        concurrency = reviews["capabilitySummary"]["verifiedFacts"]["concurrency"]
        concurrency.update(
            evidenceState="VERIFIED",
            highestVerifiedConcurrentRequests=32,
            statement="本轮最高已验证 32 路并发，不构成硬上限。",
            evidenceRefs=["request:req-pass"],
        )

        self.assertEqual([], validate_reviews(parsed, reviews))
        concurrency["statement"] = "本轮 32 路并发是真实硬上限。"
        self.assertIn(
            "concurrency statement contains an unbounded maximum claim",
            validate_reviews(parsed, reviews),
        )

    def test_verified_facts_validation_matrix(self) -> None:
        cases = (
            (
                "exact child shape",
                "interfaceProtocol",
                "unexpected",
                True,
                "interfaceProtocol field unexpected is not allowed",
            ),
            (
                "evidenceState list",
                "interfaceProtocol",
                "evidenceState",
                [],
                "interfaceProtocol evidenceState is invalid",
            ),
            (
                "family dict",
                "interfaceProtocol",
                "family",
                {},
                "interfaceProtocol family is invalid",
            ),
            (
                "context bool",
                "contextWindow",
                "highestVerifiedInputTokens",
                True,
                "contextWindow highestVerifiedInputTokens must be a non-negative integer or null",
            ),
            (
                "context negative",
                "contextWindow",
                "highestVerifiedInputTokens",
                -1,
                "contextWindow highestVerifiedInputTokens must be a non-negative integer or null",
            ),
            (
                "concurrency bool",
                "concurrency",
                "highestVerifiedConcurrentRequests",
                True,
                "concurrency highestVerifiedConcurrentRequests must be a positive integer or null",
            ),
            (
                "concurrency zero",
                "concurrency",
                "highestVerifiedConcurrentRequests",
                0,
                "concurrency highestVerifiedConcurrentRequests must be a positive integer or null",
            ),
            (
                "wrong evidence domain",
                "interfaceProtocol",
                "evidenceRefs",
                ["request:req-pass"],
                "interfaceProtocol evidence reference is outside its allowed domain: request:req-pass",
            ),
        )
        for name, fact_name, field, value, expected in cases:
            with self.subTest(name=name):
                reviews = self._reviews()
                reviews["capabilitySummary"]["verifiedFacts"][fact_name][field] = value
                try:
                    errors = validate_reviews(self._parsed(), reviews)
                except TypeError as error:
                    self.fail(f"verified-facts validation crashed: {error}")
                self.assertIn(expected, errors)

        with self.subTest(name="successful VERIFIED combination"):
            parsed = self._parsed()
            reviews = self._reviews()
            for test_id in ("014", "057"):
                parsed["tests"][test_id] = {
                    "category": "能力边界",
                    "name": f"验证样例 {test_id}",
                    "requestRefs": ["req-pass"],
                }
                reviews["tests"][test_id] = self._test_review(test_id, "PASS")
            facts = reviews["capabilitySummary"]["verifiedFacts"]
            facts["interfaceProtocol"].update(
                evidenceState="VERIFIED",
                family="OPENAI_CHAT_COMPLETIONS",
                requestFormat="已验证 OpenAI Chat Completions 请求格式。",
                responseFormat="已验证 OpenAI Chat Completions 响应格式。",
                statement="本轮已验证接口协议格式。",
            )
            facts["contextWindow"].update(
                evidenceState="VERIFIED",
                highestVerifiedTier="8K Token 近似档",
                highestVerifiedInputTokens=8120,
                statement="本轮至少支持 8K Token 近似档输入。",
                evidenceRefs=["request:req-pass"],
            )
            facts["concurrency"].update(
                evidenceState="VERIFIED",
                highestVerifiedConcurrentRequests=4,
                statement="本轮最高已验证 4 路短时并发请求成功。",
                evidenceRefs=["request:req-pass"],
            )
            self.assertEqual([], validate_reviews(parsed, reviews))

    def test_parsed_structure_rejects_dangling_request_ref(self) -> None:
        parsed = self._parsed()
        parsed["tests"]["002"]["requestRefs"].append("req-missing")
        reviews = self._reviews()
        reviews["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"][
            "evidenceRefs"
        ] = ["request:req-missing"]

        errors = validate_reviews(parsed, reviews)

        self.assertIn(
            "Parsed test 002 requestRef does not exist: req-missing",
            errors,
        )

    def test_v6_requires_failure_analysis_for_every_fail(self) -> None:
        reviews = self._reviews()
        del reviews["tests"]["002"]["failureAnalysis"]

        errors = validate_reviews(self._parsed(), reviews)

        self.assertIn("Test 002 failureAnalysis is required for FAIL", errors)

    def test_v6_rejects_failure_analysis_on_pass(self) -> None:
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

    def test_v6_rejects_duplicate_contract_references(self) -> None:
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

    def test_assembly_emits_assessment_v7_with_verified_facts(self) -> None:
        reviews = self._reviews()
        expected_facts = self._verified_facts()
        try:
            assessment = assemble_assessment(self._parsed(), reviews)
        except KeyError as error:
            self.fail(f"assembly rejected the reviews v2 envelope: {error}")

        self.assertEqual(
            "llm-capability-doctor.assessment.v8",
            assessment["schemaVersion"],
        )
        self.assertNotIn("failureAnalysis", assessment["tests"][0])
        self.assertEqual(
            "SUFFICIENT",
            assessment["tests"][1]["failureAnalysis"]["evidenceSufficiency"],
        )
        self.assertEqual(
            expected_facts,
            assessment["capabilitySummary"]["verifiedFacts"],
        )
        self.assertIsNot(
            reviews["capabilitySummary"]["verifiedFacts"],
            assessment["capabilitySummary"]["verifiedFacts"],
        )
        self.assertEqual([], validate_assessment(assessment))

    def test_complete_verified_facts_survive_validate_assemble_and_render(
        self,
    ) -> None:
        parsed, reviews = self._complete_fact_fixture()
        native_prompt_tokens = {
            request_id: json.loads(parsed["requests"][request_id]["responseBody"])
            .get("usage", {})
            .get("prompt_tokens")
            for request_id in ("context-pass", "context-fail")
        }
        self.assertEqual(
            {"context-pass": 80175, "context-fail": 131072},
            native_prompt_tokens,
        )
        context_contracts = {
            "context-pass": ("32K Token 近似档", "EXPECTED_SENTINEL", True),
            "context-fail": ("64K Token 近似档", "EXPECTED_SENTINEL", False),
        }
        for request_id, (tier, target, should_pass) in context_contracts.items():
            request_body = json.loads(parsed["requests"][request_id]["requestBody"])
            messages = request_body.get("messages")
            self.assertIsInstance(messages, list)
            self.assertGreaterEqual(len(messages), 2)
            contract_text = "\n".join(message["content"] for message in messages)
            self.assertIn("Return exactly the required target sentinel", contract_text)
            self.assertIn(f"Context-tier intent: {tier}", contract_text)
            self.assertIn(f"Required target sentinel: {target}", contract_text)
            response_body = json.loads(
                parsed["requests"][request_id]["responseBody"]
            )
            returned = response_body["choices"][0]["message"]["content"]
            if should_pass:
                self.assertEqual(target, returned)
            else:
                self.assertEqual("NOT_FOUND", returned)
                self.assertNotEqual(target, returned)

            self.assertNotIn(
                "input_tokens",
                parsed["requests"][request_id]["metrics"],
            )
            self.assertEqual(
                "200",
                parsed["requests"][request_id]["metrics"]["http_status"],
            )
        wave_request_ids = [
            f"concurrency-32-{index}" for index in range(1, 33)
        ]
        self.assertEqual(
            wave_request_ids,
            parsed["tests"]["057"]["requestRefs"],
        )
        self.assertEqual(
            [f"request:{request_id}" for request_id in wave_request_ids],
            reviews["capabilitySummary"]["verifiedFacts"]["concurrency"][
                "evidenceRefs"
            ],
        )
        self.assertEqual(
            [f"request:{request_id}" for request_id in wave_request_ids],
            reviews["tests"]["057"]["evidenceRefs"],
        )
        self.assertEqual(
            wave_request_ids,
            [
                request_id
                for request_id in parsed["requests"]
                if request_id.startswith("concurrency-32-")
            ],
        )
        wave_samples = {
            request_id: parsed["requests"][request_id]
            for request_id in wave_request_ids
            if request_id in parsed["requests"]
        }
        self.assertEqual(32, len(wave_samples))
        for request_id, sample in wave_samples.items():
            with self.subTest(request_id=request_id):
                request_body = json.loads(sample["requestBody"])
                user_content = request_body["messages"][-1]["content"]
                marker_prefix = "Return exactly this wave marker: "
                self.assertTrue(user_content.startswith(marker_prefix))
                requested_marker = user_content[len(marker_prefix) :]
                response_body = json.loads(sample["responseBody"])
                returned_marker = response_body["choices"][0]["message"]["content"]
                self.assertEqual(requested_marker, returned_marker)

                metrics = sample["metrics"]
                self.assertEqual(
                    {
                        "curl_exit_code",
                        "http_status",
                        "time_total",
                        "time_starttransfer",
                        "size_download",
                    },
                    set(metrics),
                )
                self.assertEqual("0", metrics["curl_exit_code"])
                self.assertEqual("200", metrics["http_status"])
                self.assertGreater(float(metrics["time_total"]), 0.0)
                response_text = sample["responseBody"].lower()
                self.assertNotIn("429", response_text)
                self.assertNotIn("rate limit", response_text)
                self.assertNotIn("too many requests", response_text)
        self.assertEqual([], validate_reviews(parsed, reviews))
        assessment = assemble_assessment(parsed, reviews)
        self.assertEqual([], validate_assessment(assessment))
        html = render_report(assessment, ASSET_DIR)
        self.assertEqual(
            [f"request:{request_id}" for request_id in wave_request_ids],
            assessment["capabilitySummary"]["verifiedFacts"]["concurrency"][
                "evidenceRefs"
            ],
        )
        self.assertEqual(
            native_prompt_tokens["context-pass"],
            assessment["capabilitySummary"]["verifiedFacts"]["contextWindow"][
                "highestVerifiedInputTokens"
            ],
        )
        self.assertEqual(
            native_prompt_tokens["context-fail"],
            assessment["capabilitySummary"]["verifiedFacts"]["contextWindow"][
                "firstFailedInputTokens"
            ],
        )
        self.assertEqual(
            32,
            assessment["capabilitySummary"]["verifiedFacts"]["concurrency"][
                "highestVerifiedConcurrentRequests"
            ],
        )
        self.assertIn("OpenAI Chat Completions", html)
        self.assertIn("真实上限未测试", html)

    def test_assessment_rejects_verified_fact_evidence_outside_item_domain(
        self,
    ) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())
        assessment["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"][
            "evidenceRefs"
        ] = ["request:req-pass"]

        errors = validate_assessment(assessment)

        self.assertIn(
            "interfaceProtocol evidence reference is outside its allowed domain: "
            "request:req-pass",
            errors,
        )

    def test_assessment_fact_validation_skips_malformed_request_data(self) -> None:
        malformed_requests = (
            "not-an-array",
            ["not-an-object", None, {"request_id": ""}],
        )
        for requests in malformed_requests:
            with self.subTest(requests=requests):
                assessment = assemble_assessment(self._parsed(), self._reviews())
                assessment["tests"][1]["requests"] = requests

                try:
                    errors = validate_assessment(assessment)
                except (AttributeError, TypeError) as error:
                    self.fail(f"malformed assessment requests caused crash: {error}")

                self.assertIn(
                    "interfaceProtocol evidence reference is outside its allowed "
                    "domain: request:req-fail",
                    errors,
                )

    def test_assessment_duplicate_test_id_cannot_hide_fail_or_supply_fact_evidence(
        self,
    ) -> None:
        for duplicate_position in ("before", "after"):
            with self.subTest(duplicate_position=duplicate_position):
                assessment = assemble_assessment(
                    self._parsed(include_dependent_fail=True),
                    self._reviews(include_dependent_fail=True),
                )
                duplicate = dict(assessment["tests"][0])
                duplicate["testId"] = "002"
                insert_at = 1 if duplicate_position == "before" else 2
                assessment["tests"].insert(insert_at, duplicate)
                assessment["summary"]["counts"] = {"PASS": 2, "FAIL": 2}
                assessment["categories"][0]["counts"]["PASS"] = 2
                assessment["capabilitySummary"]["issues"] = [
                    {
                        "title": "派生失败",
                        "statement": "依赖样本不足，指标不可测量。",
                        "testRefs": ["003"],
                        "evidenceRefs": ["request:req-dependent"],
                        "boundary": "不判定性能优劣。",
                    }
                ]
                assessment["capabilitySummary"]["verifiedFacts"][
                    "interfaceProtocol"
                ]["evidenceRefs"] = ["request:req-pass"]

                errors = validate_assessment(assessment)

                self.assertIn("tests[2] testId is duplicated: 002", errors)
                self.assertIn(
                    "Test 003 dependency must reference another FAIL test: 002",
                    errors,
                )
                self.assertIn(
                    "interfaceProtocol evidence reference is outside its allowed "
                    "domain: request:req-pass",
                    errors,
                )
                self.assertNotIn(
                    "capabilitySummary does not cover FAIL tests: 002",
                    errors,
                )

    def test_assessment_rejects_invalid_test_ids_without_crashing(self) -> None:
        invalid_ids = (None, "", "   ", [], {}, 2, True)
        for test_id in invalid_ids:
            with self.subTest(test_id=test_id):
                assessment = assemble_assessment(self._parsed(), self._reviews())
                assessment["tests"][0]["testId"] = test_id

                try:
                    errors = validate_assessment(assessment)
                except (AttributeError, TypeError) as error:
                    self.fail(f"invalid assessment testId caused crash: {error}")

                self.assertIn(
                    "tests[0] testId must be a non-empty string",
                    errors,
                )

    def test_assessment_rejects_malformed_containers_without_crashing(self) -> None:
        cases = (
            ("assessment", None, [], "Assessment must be an object"),
            ("summary", "summary", [], "summary must be an object"),
            ("source", "source", None, "source must be an object"),
        )
        for name, field, value, expected in cases:
            with self.subTest(name=name):
                assessment = assemble_assessment(self._parsed(), self._reviews())
                if field is None:
                    assessment = value
                else:
                    assessment[field] = value

                try:
                    errors = validate_assessment(assessment)
                except (AttributeError, TypeError) as error:
                    self.fail(f"malformed assessment container caused crash: {error}")

                self.assertIn(expected, errors)

    def test_renderer_rejects_incompatible_containers_with_value_error(self) -> None:
        cases = (
            (
                "run",
                lambda assessment: assessment.__setitem__("run", []),
                "run must be an object",
            ),
            (
                "logic",
                lambda assessment: assessment["tests"][0].__setitem__(
                    "logic", []
                ),
                "Test 001 logic must be an object",
            ),
            (
                "requests",
                lambda assessment: assessment["tests"][0].__setitem__(
                    "requests", {"unexpected": {}}
                ),
                "Test 001 requests must be an array",
            ),
            (
                "request",
                lambda assessment: assessment["tests"][0].__setitem__(
                    "requests", [[]]
                ),
                "Test 001 requests[0] must be an object",
            ),
            (
                "request metrics",
                lambda assessment: assessment["tests"][0]["requests"][
                    0
                ].__setitem__("metrics", []),
                "Test 001 requests[0] metrics must be an object",
            ),
        )
        for name, mutate, expected in cases:
            with self.subTest(name=name):
                assessment = assemble_assessment(self._parsed(), self._reviews())
                mutate(assessment)

                try:
                    render_report(assessment, ASSET_DIR)
                except AttributeError as error:
                    self.fail(f"renderer leaked AttributeError: {error}")
                except ValueError as error:
                    self.assertIn(expected, str(error))
                else:
                    self.fail("renderer accepted an incompatible assessment container")

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

    def test_assessment_schema_requires_verified_facts(self) -> None:
        schema_path = SKILL_DIR / "references" / "assessment-schema.json"
        schema = json.loads(schema_path.read_text(encoding="utf-8"))

        self.assertEqual(
            "llm-capability-doctor.assessment.v8",
            schema["$id"],
        )
        self.assertEqual(
            "llm-capability-doctor.assessment.v8",
            schema["properties"]["schemaVersion"]["const"],
        )
        self.assertIn("capabilitySummary", schema["required"])
        self.assertIn(
            "verifiedFacts",
            schema["$defs"]["capabilitySummary"]["required"],
        )
        self.assertIn("verifiedFacts", schema["$defs"])
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

    def test_renderer_places_sections_in_approved_reading_order(self) -> None:
        try:
            assessment = assemble_assessment(self._parsed(), self._reviews())
            html = render_report(assessment, ASSET_DIR)
        except (KeyError, ValueError) as error:
            self.fail(f"renderer rejected the desired v7 contract: {error}")

        expected_order = (
            '<section class="report-section" id="conclusion"',
            '<section class="report-section" id="issues"',
            '<section class="report-section" id="run-info"',
            '<section class="report-section" id="capabilities"',
        )
        positions = tuple(html.find(marker) for marker in expected_order)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)
        self.assertNotIn('id="protocol"', html)
        parser = _MainChildParser()
        parser.feed(html)
        self.assertEqual(
            [
                ("header", "report-header"),
                ("section", "report-section"),
                ("section", "report-section"),
                ("section", "report-section"),
                ("section", "report-section"),
            ],
            parser.children[:5],
        )

    def test_renderer_omits_sidebar_actions_and_result_scope(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())
        html = render_report(assessment, ASSET_DIR)
        script = (ASSET_DIR / "report.js").read_text(encoding="utf-8")
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")

        retained = (
            'id="conclusion"',
            'id="issues"',
            'id="run-info"',
            'id="capabilities"',
        )
        positions = tuple(html.find(marker) for marker in retained)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)
        for deleted in (
            'id="expand-all"',
            'id="collapse-all"',
            'id="print-report"',
            'href="#scope"',
            'value="scope"',
            'id="scope"',
            "展开全部",
            "收起全部",
            "打印报告",
            "结果适用范围",
        ):
            with self.subTest(deleted=deleted):
                self.assertNotIn(deleted, html)
        for deleted in ('"expand-all"', '"collapse-all"', '"print-report"'):
            with self.subTest(script=deleted):
                self.assertNotIn(deleted, script)
        for deleted in (".sidebar-actions", ".action-button", ".scope-list"):
            with self.subTest(css=deleted):
                self.assertNotIn(deleted, css)

    def test_opencodex_compatibility_renders_all_levels(self) -> None:
        for level, value in (
            ("PASS", "兼容"),
            ("FAIL", "不兼容"),
            ("NOT_ASSESSED", "未评定"),
        ):
            with self.subTest(level=level):
                html = _opencodex_compatibility(
                    {
                        "profile": "opencodex-2.7.42-data-format",
                        "level": level,
                        "label": f"OpenCodex 数据格式{value}",
                        "protocolFamily": "OPENAI_CHAT_COMPLETIONS",
                        "requiredTestIds": ["002", "004"],
                        "failedTestIds": [],
                        "statement": "固定兼容性结论。",
                        "scopeBoundary": "固定范围边界。",
                    }
                )

                tone = {"PASS": "pass", "FAIL": "fail"}.get(level, "neutral")
                self.assertIn(f'class="verdict-block {tone}"', html)
                self.assertIn(
                    '<p class="verdict-label">能否直接接入 OpenCodex（数据格式）</p>',
                    html,
                )
                self.assertIn(f'<p class="verdict-value">{value}</p>', html)
                self.assertIn('<p class="verdict-statement">固定兼容性结论。</p>', html)
                self.assertIn('<p class="verdict-boundary">固定范围边界。</p>', html)

    def test_opencodex_compatibility_escapes_all_dynamic_fields(self) -> None:
        payloads = {
            "level": '<level data-x="2">level</level>',
            "statement": '<statement data-x="7">statement</statement>',
            "scopeBoundary": '<scope data-x="8">scope</scope>',
        }

        html = _opencodex_compatibility(payloads)

        for payload in (
            payloads["statement"],
            payloads["scopeBoundary"],
        ):
            with self.subTest(payload=payload):
                self.assertNotIn(payload, html)
                self.assertIn(escape(payload, quote=True), html)
        self.assertIn('class="verdict-block neutral"', html)
        self.assertNotIn('class="<level', html)
        self.assertNotIn("<level data-x", html)

    def test_opencodex_compatibility_css_is_responsive_and_print_safe(self) -> None:
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")
        mobile_css = css.split("@media (max-width: 680px)", 1)[1].split(
            "@media print", 1
        )[0]
        print_css = css.split("@media print", 1)[1]

        self.assertIn(".verdict-block", css)
        self.assertRegex(
            css,
            r"\.verdict-boundary\s*\{[^}]*"
            r"overflow-wrap:\s*anywhere;",
        )
        self.assertRegex(
            mobile_css,
            r"\.result-columns[^{]*\{[^}]*grid-template-columns:\s*1fr;",
        )
        self.assertRegex(
            print_css,
            r"\.verdict-block[^{]*\{[^}]*break-inside:\s*avoid;",
        )
        for variable, value in {
            "--pass": "#167453",
            "--fail": "#b42318",
            "--neutral": "#4b5563",
        }.items():
            with self.subTest(variable=variable):
                self.assertIn(f"{variable}: {value};", css)

    def test_final_conclusion_renders_all_three_fact_rows(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        for expected in (
            "接口格式（依据检测 002）",
            "长文本处理（依据检测 014-018）",
            "同时请求（依据检测 057）",
            "本轮证据不足以确认接口协议格式。",
            "本轮未采集上下文档位证据。",
            "本轮未采集并发波次证据。",
        ):
            self.assertIn(expected, html)
        expected_order = (
            'id="conclusion"',
            '<dl class="verified-facts">',
            '<section class="report-section" id="issues"',
        )
        positions = tuple(html.find(marker) for marker in expected_order)
        self.assertTrue(all(position >= 0 for position in positions), positions)
        self.assertEqual(tuple(sorted(positions)), positions)

    def test_verified_facts_escape_dynamic_text(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["verifiedFacts"]["interfaceProtocol"][
            "statement"
        ] = "<script>alert(1)</script>"
        assessment = assemble_assessment(self._parsed(), reviews)

        html = render_report(assessment, ASSET_DIR)

        self.assertNotIn("<script>alert(1)</script>", html)
        self.assertIn("&lt;script&gt;alert(1)&lt;/script&gt;", html)

    def test_verified_facts_helper_escapes_every_dynamic_field(self) -> None:
        payloads = {
            "interface statement": "<script>interface-statement</script>",
            "request format": "<request-format>request</request-format>",
            "response format": "<response-format>response</response-format>",
            "protocol family": "<unknown-family>family</unknown-family>",
            "interface boundary": "<interface-boundary>boundary</interface-boundary>",
            "context statement": "<context-statement>statement</context-statement>",
            "highest tier": "<highest-tier>tier</highest-tier>",
            "highest tokens": "<highest-tokens>tokens</highest-tokens>",
            "failed tier": "<failed-tier>tier</failed-tier>",
            "failed tokens": "<failed-tokens>tokens</failed-tokens>",
            "context boundary": "<context-boundary>boundary</context-boundary>",
            "concurrency statement": (
                "<concurrency-statement>statement</concurrency-statement>"
            ),
            "concurrency value": "<concurrency-value>value</concurrency-value>",
            "concurrency boundary": (
                "<concurrency-boundary>boundary</concurrency-boundary>"
            ),
        }
        facts = {
            "interfaceProtocol": {
                "statement": payloads["interface statement"],
                "requestFormat": payloads["request format"],
                "responseFormat": payloads["response format"],
                "family": payloads["protocol family"],
                "boundary": payloads["interface boundary"],
            },
            "contextWindow": {
                "statement": payloads["context statement"],
                "highestVerifiedTier": payloads["highest tier"],
                "highestVerifiedInputTokens": payloads["highest tokens"],
                "firstFailedTier": payloads["failed tier"],
                "firstFailedInputTokens": payloads["failed tokens"],
                "boundary": payloads["context boundary"],
            },
            "concurrency": {
                "statement": payloads["concurrency statement"],
                "highestVerifiedConcurrentRequests": payloads[
                    "concurrency value"
                ],
                "boundary": payloads["concurrency boundary"],
            },
        }

        html = _verified_facts(facts)

        for name, payload in payloads.items():
            with self.subTest(name=name):
                self.assertNotIn(payload, html)
                self.assertIn(escape(payload, quote=True), html)

    def test_only_fail_rows_render_evidence_review(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        self.assertEqual(1, html.count("为什么没有通过"))
        self.assertIn("现有记录足以确认本次结果", html)
        self.assertIn("响应直接违反了本项核心契约。", html)
        self.assertIn("不能据此推断后端模型身份。", html)

    def test_renderer_shows_decisive_evidence_refs_kind_and_metrics(self) -> None:
        assessment = assemble_assessment(self._parsed(), self._reviews())

        html = render_report(assessment, ASSET_DIR)

        self.assertIn("关键证据摘录", html)
        self.assertIn("记录了可观察响应。", html)
        self.assertIn("为什么判为未通过", html)
        self.assertIn("实际结果没有完成要求", html)
        self.assertNotIn("<dd>DIRECT</dd>", html)
        self.assertIn("对应的原始记录编号", html)
        self.assertIn("request:req-fail", html)
        self.assertIn("curl exit 0", html)
        self.assertIn("首次返回 0.050000 秒", html)

    def test_print_css_reveals_evidence_and_removes_code_clipping(self) -> None:
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")
        print_css = css.split("@media print", 1)[1]

        self.assertNotRegex(
            print_css,
            r"\.verified-facts(?:\s*>\s*div)?\s*\{[^}]*display:\s*none;",
        )
        self.assertRegex(
            print_css,
            r"\.test-detail[^{]*\{[^}]*display:\s*block\s*!important;",
        )
        self.assertRegex(print_css, r"pre\s*\{[^}]*max-height:\s*none;")
        self.assertRegex(print_css, r"pre\s*\{[^}]*overflow:\s*visible;")

    def test_final_conclusion_omits_freeform_summary_content(self) -> None:
        reviews = self._reviews()
        reviews["capabilitySummary"]["headline"] = "HTML_HEADLINE_MUST_NOT_RENDER"
        reviews["capabilitySummary"]["issues"][0].update(
            title="HTML_ISSUE_TITLE_MUST_NOT_RENDER",
            statement="HTML_ISSUE_STATEMENT_MUST_NOT_RENDER",
            boundary="HTML_ISSUE_BOUNDARY_MUST_NOT_RENDER",
        )
        assessment = assemble_assessment(self._parsed(), reviews)

        html = render_report(assessment, ASSET_DIR)

        self.assertEqual(
            "HTML_HEADLINE_MUST_NOT_RENDER",
            assessment["capabilitySummary"]["headline"],
        )
        for omitted in (
            "HTML_HEADLINE_MUST_NOT_RENDER",
            "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。",
            'class="final-conclusion-lead"',
            'class="final-conclusion-list"',
            'class="final-conclusion-scope"',
        ):
            self.assertNotIn(omitted, html)
        self.assertIn("当前主要有 1 类问题：HTML_ISSUE_TITLE_MUST_NOT_RENDER。", html)
        self.assertIn("HTML_ISSUE_STATEMENT_MUST_NOT_RENDER", html)
        self.assertIn("HTML_ISSUE_BOUNDARY_MUST_NOT_RENDER", html)
        self.assertIn("关联检测项", html)
        self.assertIn('class="verdict-block ', html)
        self.assertIn('<dl class="verified-facts">', html)

    def test_final_conclusion_css_omits_removed_freeform_styles(self) -> None:
        css = (ASSET_DIR / "report.css").read_text(encoding="utf-8")

        for selector in (
            ".final-conclusion-lead",
            ".final-conclusion-list",
            ".final-conclusion-boundary",
            ".final-conclusion-refs",
            ".final-conclusion-scope",
        ):
            self.assertNotIn(selector, css)

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

        self.assertNotIn("本轮所有已执行检测项均通过。", html)
        self.assertNotIn('<ol class="final-conclusion-list">', html)
        self.assertIn("本轮没有需要处理的问题", html)
        self.assertIn("本轮检查没有未通过项。", html)

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

    def test_cli_render_writes_v7_and_final_conclusion(self) -> None:
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
            "llm-capability-doctor.assessment.v8",
            assessment["schemaVersion"],
        )
        self.assertIn('<section class="report-section" id="conclusion"', html)
        self.assertIn("大模型能力诊断报告", html)
        self.assertEqual(1, html.count("为什么没有通过"))

    def test_skill_instructions_require_the_three_stage_flow(self) -> None:
        skill_text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("failureAnalysis", skill_text)
        self.assertIn("capabilitySummary", skill_text)
        self.assertIn("检测信息", skill_text)
        self.assertIn("不得手工修改渲染后的 HTML", skill_text)
        self.assertNotIn("assessment.v4", skill_text)
        self.assertNotIn("结果适用范围", skill_text)
        self.assertNotIn("展开全部", skill_text)
        self.assertNotIn("收起全部", skill_text)
        self.assertNotIn("打印报告", skill_text)

    def test_skill_requires_verified_protocol_context_and_concurrency_facts(self) -> None:
        skill_text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")
        workflow = skill_text.split("## Workflow", 1)[1].split(
            "Write `$TMP/reviews.json`", 1
        )[0]

        for marker in (
            "llm-capability-doctor.reviews.v2",
            "llm-capability-doctor.assessment.v8",
            "interfaceProtocol",
            "contextWindow",
            "highestVerifiedInputTokens",
            "firstFailedInputTokens",
            "highestVerifiedConcurrentRequests",
        ):
            self.assertIn(marker, workflow)
        self.assertLess(
            workflow.index("capabilitySummary.verifiedFacts"),
            workflow.index("capabilitySummary.issues"),
        )

    def test_skill_documents_opencodex_contract_and_program_ownership(self) -> None:
        skill_text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")

        for marker in (
            "collector v0.11.0",
            "llm-capability-doctor.evidence.v3",
            "compatibility_profile: opencodex-2.7.42-data-format",
            "llm-capability-doctor.assessment.v8",
            "openCodexCompatibility",
            "002、004、005、006、040、041、043、047",
            "仅判断本轮模型端数据格式，不覆盖鉴权、网络、部署或 ClawOps 运行环境。",
        ):
            self.assertIn(marker, skill_text)
        self.assertIn("不得填写 `generalVerdict` 或 `openCodexCompatibility`", skill_text)
        self.assertIn("v1/v2", skill_text)
        self.assertIn("NOT_ASSESSED", skill_text)

    def test_evaluation_rules_define_opencodex_v3_format_contract(self) -> None:
        rules = (SKILL_DIR / "references" / "evaluation-rules.md").read_text(
            encoding="utf-8"
        )

        def section(current: str, following: str) -> str:
            return rules.split(current, 1)[1].split(following, 1)[0]

        for marker in (
            "collector v0.11.0",
            "llm-capability-doctor.evidence.v3",
            "compatibility_profile: opencodex-2.7.42-data-format",
            "assessment.v8.capabilitySummary.openCodexCompatibility",
            "002、004、005、006、040、041、043、047",
            "OPENAI_CHAT_COMPLETIONS",
            "OPENAI_RESPONSES",
            "ANTHROPIC_MESSAGES",
            "GEMINI_GENERATE_CONTENT",
            "OLLAMA_CHAT",
            "CUSTOM",
            "UNKNOWN",
            "doctor/get_weather",
            "doctor__get_weather",
            "response.completed",
            "message_stop",
            "finishReason",
            "usageMetadata",
            "tool_call_id",
            "call_id",
            "tool_use_id",
            "Google 本地调用 ID",
        ):
            self.assertIn(marker, rules)
        self.assertIn("045 仍是增强能力项", rules)
        self.assertIn("v1/v2", rules)
        self.assertIn("NOT_ASSESSED", rules)

        protocol_rules = section("### 002 协议识别", "### 003 鉴权与模型接受")
        self.assertIn("non-empty root `id`", protocol_rules)
        self.assertIn('`type:"output_text"`', protocol_rules)
        self.assertIn("`candidates[0].content.parts[].text`", protocol_rules)
        self.assertNotIn('`type:"response"`', protocol_rules)
        self.assertNotIn('`status:"completed"`', protocol_rules)

        sync_rules = section("### 004 同步生成", "### 005 流式生成")
        self.assertIn("does not require root `type` or `status`", sync_rules)
        self.assertIn("nested lookalike", sync_rules)
        self.assertNotIn("incomplete state", sync_rules)

        stream_rules = section("### 005 流式生成", "### 006 流结束完整性")
        self.assertIn(
            "OpenAI Responses, Anthropic, and Google accept `data:` with an "
            "optional following space",
            stream_rules,
        )
        self.assertIn(
            "Anthropic drops every `data:` frame whose JSON cannot be parsed",
            stream_rules,
        )
        self.assertIn(
            "malformed JSON `data:` frames for OpenAI Chat, OpenAI Responses, "
            "and Google record a stream error",
            stream_rules,
        )
        self.assertIn(
            "OpenAI Chat reads only `choices[0]` and Google reads only "
            "`candidates[0]`",
            stream_rules,
        )
        self.assertIn(
            "uses exactly `?alt=sse` and discards any existing query",
            stream_rules,
        )
        self.assertIn("Custom paths remain unchanged", stream_rules)

        terminal_rules = section("### 006 流结束完整性", "### 007 Token usage")
        self.assertIn("stops parsing at an immediate terminal", terminal_rules)
        self.assertIn("stops reading later network chunks", terminal_rules)
        self.assertIn(
            "may still exist in the raw response evidence but do not participate "
            "in the stream inspector verdict",
            terminal_rules,
        )
        self.assertNotIn("outside the collected evidence", terminal_rules)
        self.assertNotIn("content after an immediate terminal", terminal_rules)
        for marker in (
            "Chat `length` or `content_filter`",
            "Anthropic `max_tokens` or `content_filter`",
            "Google `MAX_TOKENS`, `SAFETY`, `RECITATION`, `BLOCKLIST`, "
            "`PROHIBITED_CONTENT`, or `SPII`",
        ):
            self.assertIn(marker, terminal_rules)

        single_tool_rules = section("### 040 单工具调用", "### 041 工具选择")
        for marker in (
            "non-empty `tool_calls[].id`",
            "non-empty `call_id`",
            "non-empty `tool_use.id`",
            "upstream `functionCall.id` is optional",
        ):
            self.assertIn(marker, single_tool_rules)
        self.assertNotIn("Call ID integrity is not judged", single_tool_rules)

        serial_tool_rules = section("### 047 串行工具调用", "### 048 工具结果忠实性")
        self.assertIn(
            "OpenAI Responses must use `doctor/get_weather`",
            serial_tool_rules,
        )
        self.assertIn(
            "OpenAI Chat, Anthropic, and Google must use `doctor__get_weather`",
            serial_tool_rules,
        )
        self.assertIn("second tool remains bare `get_time`", serial_tool_rules)

        concurrency_rules = section("### 057 并发响应时间", "## 13. Security Business Language")
        self.assertIn("in evidence v2 and v3", concurrency_rules)
        self.assertNotIn("in evidence v2;", concurrency_rules)

    def test_readme_explains_bounded_opencodex_compatibility_result(self) -> None:
        readme = (SKILL_DIR.parents[1] / "README.md").read_text(encoding="utf-8")

        for marker in (
            "llm-capability-doctor.assessment.v8",
            "002、004、005、006、040、041、043、047",
            "OpenAI Chat Completions",
            "OpenAI Responses",
            "Anthropic Messages",
            "Gemini GenerateContent",
            "首个 choice/candidate",
            "`response.incomplete`",
            "`?alt=sse`",
            "仅判断本轮模型端数据格式，不覆盖鉴权、网络、部署或 ClawOps 运行环境。",
            "python3 -m unittest discover",
        ):
            self.assertIn(marker, readme)
        self.assertIn("47 个固定检测项", readme)
        self.assertIn("v1/v2", readme)
        self.assertIn("NOT_ASSESSED", readme)

    def test_skill_reviews_example_is_valid_and_evidence_bounded(self) -> None:
        skill_text = (SKILL_DIR / "SKILL.md").read_text(encoding="utf-8")
        match = re.search(
            r"Write `\$TMP/reviews\.json` in this envelope:\s*"
            r"```json\s*(\{.*?\})\s*```",
            skill_text,
            re.DOTALL,
        )
        self.assertIsNotNone(match)
        reviews = json.loads(match.group(1))
        parsed = self._parsed()
        parsed["tests"] = {
            "008": {
                "category": "接口与协议",
                "name": "错误可观测性",
                "requestRefs": ["test-008"],
            }
        }
        parsed["requests"] = {
            "test-008": self._request("test-008", "UNSTRUCTURED_ERROR")
        }

        self.assertEqual([], validate_reviews(parsed, reviews))
        self.assertNotIn(
            "基础能力可用",
            reviews["capabilitySummary"]["headline"],
        )

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

    def test_rules_forbid_claiming_tested_values_as_hard_limits(self) -> None:
        rules_text = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        facts_section = rules_text.split(
            "## 4. Verified Capability Facts", 1
        )[1].split("## 5. Failure Evidence Audit", 1)[0]

        for marker in ("最高已验证", "不能写成真实硬上限", "002", "014-018", "057"):
            self.assertIn(marker, facts_section)
        for custom_rule in (
            "complete, coherent request/response contract",
            "matches none of the five known families",
            "UNKNOWN/INCONCLUSIVE",
        ):
            self.assertIn(custom_rule, facts_section)

    def test_context_tiers_measure_capacity_without_scoring_answer_accuracy(self) -> None:
        rules_text = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        context_section = rules_text.split(
            "## 10. Context, Instruction, and Reasoning", 1
        )[1].split("### 019", 1)[0]

        for marker in (
            "context capacity only",
            "HTTP 2xx",
            "protocol-valid response",
            "non-empty model-visible assistant content",
            "Do not score response accuracy",
            "highest tested passing tier",
        ):
            self.assertIn(marker, context_section)
        self.assertNotIn("all six requested JSON fields", context_section)
        self.assertNotIn("every requested value is exact", context_section)


if __name__ == "__main__":
    unittest.main()
