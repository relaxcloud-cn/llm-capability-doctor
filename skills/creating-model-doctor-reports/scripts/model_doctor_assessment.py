#!/usr/bin/env python3
"""Validate binary semantic reviews and assemble a Model Doctor assessment."""

from __future__ import annotations

from collections import OrderedDict
from copy import deepcopy
from datetime import datetime, timezone
import re
from typing import Dict, List

from model_doctor_contracts import V3_CONTRACT, contract_key
from model_doctor_general_verdict import derive_general_verdict
from model_doctor_opencodex_compatibility import derive_opencodex_compatibility
from model_doctor_tool_loop_conformance import (
    V3_TOOL_TEST_IDS,
    _validate_v3_tool_pass_requests,
    _validate_v3_tool_pass_review,
)
from model_doctor_verified_facts import validate_verified_facts


REVIEW_SCHEMA_VERSION = "llm-capability-doctor.reviews.v2"
ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v8"
STATUSES = {"PASS", "FAIL"}
FAILURE_KINDS = {
    "DIRECT",
    "CONTRACT_FACET",
    "MEASUREMENT_UNAVAILABLE",
    "EVIDENCE_GAP",
}
EVIDENCE_SUFFICIENCY = {"SUFFICIENT", "LIMITED", "INSUFFICIENT"}
CAPABILITY_SCOPE_BOUNDARY = (
    "本节仅总结本轮可观察能力，不构成项目 READY/BLOCKED 判定。"
)
READINESS_DECISION_PATTERN = re.compile(
    r"(?<![A-Za-z])(?:READY|BLOCKED)(?![A-Za-z])|可上线|不可上线",
    re.IGNORECASE,
)
REVIEW_FIELDS = {"schemaVersion", "tests", "capabilitySummary"}
REVIEW_TEST_FIELDS = {
    "testId",
    "reviewedStatus",
    "conclusion",
    "logic",
    "evidenceRefs",
    "evidenceExcerpts",
    "limitations",
    "retestInstructions",
    "failureAnalysis",
}
FAILURE_ANALYSIS_FIELDS = {
    "failureKind",
    "evidenceSufficiency",
    "supportedClaim",
    "unsupportedClaims",
    "dependsOnTestIds",
    "evidenceRefs",
}
REVIEW_CAPABILITY_SUMMARY_FIELDS = {
    "headline",
    "verifiedFacts",
    "issues",
    "scopeBoundary",
}
ASSESSMENT_CAPABILITY_SUMMARY_FIELDS = REVIEW_CAPABILITY_SUMMARY_FIELDS | {
    "generalVerdict",
    "openCodexCompatibility",
}
GENERAL_VERDICT_COUNT_FIELDS = (
    "collectedTests",
    "passedTests",
    "totalTests",
    "passedCoreTests",
    "totalCoreTests",
    "passedEnhancedTests",
    "totalEnhancedTests",
)
CAPABILITY_ISSUE_FIELDS = {
    "title",
    "statement",
    "testRefs",
    "evidenceRefs",
    "boundary",
}
LOGIC_FIELDS = {
    "purpose",
    "method",
    "passCriteria",
    "failCriteria",
    "capabilityBoundary",
}
ASSESSMENT_FIELDS = {
    "schemaVersion",
    "generatedAt",
    "source",
    "run",
    "tokenTotals",
    "warnings",
    "summary",
    "capabilitySummary",
    "categories",
    "tests",
}
TEST_FIELDS = {
    "testId",
    "category",
    "name",
    "reviewedStatus",
    "conclusion",
    "logic",
    "rawObservation",
    "metrics",
    "evidenceRefs",
    "evidenceExcerpts",
    "limitations",
    "retestInstructions",
    "requests",
    "failureAnalysis",
}


def _non_empty_string(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _string_list(value: object, require_item: bool = False) -> bool:
    if not isinstance(value, list):
        return False
    if require_item and not value:
        return False
    return all(_non_empty_string(item) for item in value)


def _has_duplicate_strings(value: object) -> bool:
    return isinstance(value, list) and len(value) != len(set(value))


def _contains_readiness_decision(value: object) -> bool:
    return isinstance(value, str) and bool(READINESS_DECISION_PATTERN.search(value))


def _validate_issue_membership(
    dependencies_by_test: Dict[str, List[str]],
    issue_indexes_by_test: Dict[str, List[int]],
) -> List[str]:
    errors: List[str] = []
    for test_id, indexes in sorted(issue_indexes_by_test.items()):
        if len(indexes) != 1:
            errors.append(
                "FAIL test must appear in exactly one capabilitySummary issue: "
                f"{test_id}"
            )
    for test_id, dependencies in sorted(dependencies_by_test.items()):
        for dependency in dependencies:
            source_indexes = issue_indexes_by_test.get(test_id, [])
            dependency_indexes = issue_indexes_by_test.get(dependency, [])
            if (
                len(source_indexes) != 1
                or len(dependency_indexes) != 1
                or source_indexes[0] != dependency_indexes[0]
            ):
                errors.append(
                    "Dependent FAIL tests must share one capabilitySummary issue: "
                    f"{test_id} -> {dependency}"
                )
    return errors


def _evidence_ref_exists(parsed: dict, reference: str) -> bool:
    if reference.startswith("request:"):
        return reference.split(":", 1)[1] in parsed.get("requests", {})
    if reference.startswith("test:") and reference.endswith(":manifest"):
        parts = reference.split(":")
        return len(parts) == 3 and parts[1] in parsed.get("tests", {})
    return False


def _evidence_ref_belongs_to_test(
    parsed: dict,
    test_id: str,
    reference: str,
) -> bool:
    request_refs = parsed.get("tests", {}).get(test_id, {}).get("requestRefs", [])
    if reference.startswith("request:"):
        return reference.split(":", 1)[1] in request_refs
    return reference == f"test:{test_id}:manifest" and not request_refs


def _validate_parsed_structure(parsed: object) -> List[str]:
    if not isinstance(parsed, dict):
        return ["Parsed evidence must be an object"]
    run = parsed.get("run")
    if not isinstance(run, dict):
        return ["Parsed run must be an object"]
    tests = parsed.get("tests")
    if not isinstance(tests, dict):
        return ["Parsed tests must be an object keyed by test ID"]
    requests = parsed.get("requests")
    if not isinstance(requests, dict):
        return ["Parsed requests must be an object keyed by request ID"]

    errors: List[str] = []
    for test_id, test in tests.items():
        if not isinstance(test, dict):
            errors.append(f"Parsed test {test_id} must be an object")
            continue
        for field in ("name", "category"):
            if not _non_empty_string(test.get(field)):
                errors.append(f"Parsed test {test_id} {field} must be non-empty")
        refs = test.get("requestRefs")
        if not _string_list(refs):
            errors.append(f"Parsed test {test_id} requestRefs must be a string array")
        else:
            for request_id in refs:
                if request_id not in requests:
                    errors.append(
                        f"Parsed test {test_id} requestRef does not exist: "
                        f"{request_id}"
                    )
    for request_id, request in requests.items():
        if not isinstance(request, dict):
            errors.append(f"Parsed request {request_id} must be an object")
    return errors


def _parsed_fact_evidence_domains(parsed: dict) -> Dict[str, set[str]]:
    tests = parsed.get("tests", {})
    if not isinstance(tests, dict):
        tests = {}

    def request_refs(test_ids: set[str]) -> set[str]:
        references = set()
        for test_id in test_ids:
            test = tests.get(test_id)
            if not isinstance(test, dict):
                continue
            request_ids = test.get("requestRefs", [])
            if not isinstance(request_ids, list):
                continue
            references.update(
                f"request:{request_id}"
                for request_id in request_ids
                if _non_empty_string(request_id)
            )
        return references

    return {
        "interfaceProtocol": request_refs({"002"}),
        "contextWindow": request_refs({"014", "015", "016", "017", "018"}),
        "concurrency": request_refs({"057"}),
    }


def validate_reviews(parsed: dict, reviews: dict) -> List[str]:
    """Return human-readable errors for Skill-authored binary reviews."""

    errors = _validate_parsed_structure(parsed)
    if errors:
        return errors
    parsed_tests = parsed.get("tests", {})
    if not isinstance(reviews, dict):
        return [f"Reviews must use {REVIEW_SCHEMA_VERSION}"]

    if reviews.get("schemaVersion") != REVIEW_SCHEMA_VERSION:
        errors.append(f"Reviews must use {REVIEW_SCHEMA_VERSION}")
    for field in sorted(set(reviews) - REVIEW_FIELDS):
        errors.append(f"Reviews field {field} is not allowed")

    test_reviews = reviews.get("tests")
    if not isinstance(test_reviews, dict):
        return errors + ["Reviews tests must be an object keyed by test ID"]

    for test_id in parsed_tests:
        if test_id not in test_reviews:
            errors.append(f"Test {test_id} review is missing")

    for test_id in test_reviews:
        if test_id not in parsed_tests:
            errors.append(f"Review contains unknown test ID {test_id}")

    statuses = {
        test_id: review.get("reviewedStatus")
        for test_id, review in test_reviews.items()
        if test_id in parsed_tests and isinstance(review, dict)
    }
    dependencies_by_test: Dict[str, List[str]] = {}

    for test_id, review in test_reviews.items():
        if test_id not in parsed_tests or not isinstance(review, dict):
            if not isinstance(review, dict):
                errors.append(f"Test {test_id} review must be an object")
            continue

        for field in sorted(set(review) - REVIEW_TEST_FIELDS):
            errors.append(f"Test {test_id} review field {field} is not allowed")
        if review.get("testId") != test_id:
            errors.append(f"Test {test_id} testId must match its object key")
        status = review.get("reviewedStatus")
        if status not in STATUSES:
            errors.append(f"Test {test_id} reviewedStatus is invalid")
        errors.extend(_validate_v3_tool_pass_review(parsed, test_id, review))
        for obsolete_field in ("confidence", "gateLevel"):
            if obsolete_field in review:
                errors.append(
                    f"Test {test_id} {obsolete_field} is not part of the "
                    "reviews.v2 contract"
                )
        if not _non_empty_string(review.get("conclusion")):
            errors.append(f"Test {test_id} conclusion is required")

        logic = review.get("logic")
        if not isinstance(logic, dict):
            errors.append(f"Test {test_id} logic must be an object")
        else:
            for field in sorted(set(logic) - LOGIC_FIELDS):
                errors.append(f"Test {test_id} logic.{field} is not allowed")
            for field in LOGIC_FIELDS:
                value = logic.get(field)
                if field in {"passCriteria", "failCriteria"}:
                    if not _string_list(value, require_item=True):
                        errors.append(f"Test {test_id} logic.{field} is required")
                elif not _non_empty_string(value):
                    errors.append(f"Test {test_id} logic.{field} is required")

        evidence_refs = review.get("evidenceRefs")
        if not _string_list(evidence_refs, require_item=True):
            errors.append(f"Test {test_id} evidenceRefs must contain evidence")
        else:
            for reference in evidence_refs:
                if not _evidence_ref_exists(parsed, reference):
                    errors.append(
                        f"Test {test_id} evidence reference does not exist: {reference}"
                    )
                elif not _evidence_ref_belongs_to_test(parsed, test_id, reference):
                    errors.append(
                        f"Test {test_id} evidence reference is not referenced by "
                        f"TEST-{test_id}: {reference}"
                    )

        if not _string_list(review.get("evidenceExcerpts"), require_item=True):
            errors.append(
                f"Test {test_id} evidenceExcerpts must contain observable text"
            )
        if not _string_list(review.get("limitations")):
            errors.append(f"Test {test_id} limitations must be a string array")
        if not _string_list(review.get("retestInstructions")):
            errors.append(
                f"Test {test_id} retestInstructions must be a string array"
            )

        failure_analysis = review.get("failureAnalysis")
        if status == "FAIL" and failure_analysis is None:
            errors.append(f"Test {test_id} failureAnalysis is required for FAIL")
        if status == "PASS" and failure_analysis is not None:
            errors.append(f"Test {test_id} failureAnalysis is only allowed for FAIL")
        if failure_analysis is None:
            continue
        if not isinstance(failure_analysis, dict):
            errors.append(f"Test {test_id} failureAnalysis must be an object")
            continue

        for field in sorted(set(failure_analysis) - FAILURE_ANALYSIS_FIELDS):
            errors.append(
                f"Test {test_id} failureAnalysis field {field} is not allowed"
            )
        if failure_analysis.get("failureKind") not in FAILURE_KINDS:
            errors.append(f"Test {test_id} failureKind is invalid")
        sufficiency = failure_analysis.get("evidenceSufficiency")
        if sufficiency not in EVIDENCE_SUFFICIENCY:
            errors.append(f"Test {test_id} evidenceSufficiency is invalid")
        if not _non_empty_string(failure_analysis.get("supportedClaim")):
            errors.append(f"Test {test_id} supportedClaim is required")
        unsupported = failure_analysis.get("unsupportedClaims")
        if not _string_list(unsupported):
            errors.append(f"Test {test_id} unsupportedClaims must be a string array")
        elif sufficiency in {"LIMITED", "INSUFFICIENT"} and not unsupported:
            errors.append(
                f"Test {test_id} {sufficiency} failureAnalysis requires "
                "unsupportedClaims"
            )

        dependencies = failure_analysis.get("dependsOnTestIds")
        if not _string_list(dependencies):
            errors.append(f"Test {test_id} dependsOnTestIds must be a string array")
        else:
            if _has_duplicate_strings(dependencies):
                errors.append(
                    f"Test {test_id} dependsOnTestIds must contain unique values"
                )
            valid_dependencies = []
            for dependency in dict.fromkeys(dependencies):
                if dependency == test_id or statuses.get(dependency) != "FAIL":
                    errors.append(
                        f"Test {test_id} dependency must reference another FAIL test: "
                        f"{dependency}"
                    )
                else:
                    valid_dependencies.append(dependency)
            if status == "FAIL":
                dependencies_by_test[test_id] = valid_dependencies

        failure_refs = failure_analysis.get("evidenceRefs")
        if not _string_list(failure_refs, require_item=True):
            errors.append(
                f"Test {test_id} failureAnalysis evidenceRefs must contain evidence"
            )
        else:
            if _has_duplicate_strings(failure_refs):
                errors.append(
                    f"Test {test_id} failureAnalysis evidenceRefs must contain "
                    "unique values"
                )
            for reference in dict.fromkeys(failure_refs):
                if not _evidence_ref_exists(parsed, reference):
                    errors.append(
                        f"Test {test_id} failureAnalysis evidence does not exist: "
                        f"{reference}"
                    )
                elif not _evidence_ref_belongs_to_test(parsed, test_id, reference):
                    errors.append(
                        f"Test {test_id} failureAnalysis evidence is not referenced "
                        f"by TEST-{test_id}: {reference}"
                    )

    summary = reviews.get("capabilitySummary")
    if not isinstance(summary, dict):
        return errors + ["capabilitySummary must be an object"]
    for field in sorted(set(summary) - REVIEW_CAPABILITY_SUMMARY_FIELDS):
        errors.append(f"capabilitySummary field {field} is not allowed")
    if not _non_empty_string(summary.get("headline")):
        errors.append("capabilitySummary headline is required")
    elif _contains_readiness_decision(summary.get("headline")):
        errors.append(
            "capabilitySummary headline must not contain project readiness decisions"
        )
    if summary.get("scopeBoundary") != CAPABILITY_SCOPE_BOUNDARY:
        errors.append("capabilitySummary scopeBoundary is invalid")
    errors.extend(
        validate_verified_facts(
            summary.get("verifiedFacts"),
            _parsed_fact_evidence_domains(parsed),
        )
    )

    issues = summary.get("issues")
    if not isinstance(issues, list):
        return errors + ["capabilitySummary issues must be an array"]
    fail_ids = {test_id for test_id, status in statuses.items() if status == "FAIL"}
    if fail_ids and not 1 <= len(issues) <= 5:
        errors.append(
            "capabilitySummary must contain 1 to 5 issues when FAIL tests exist"
        )
    if not fail_ids and issues:
        errors.append("capabilitySummary issues must be empty when all tests PASS")

    issue_indexes_by_test: Dict[str, List[int]] = {}
    for index, issue in enumerate(issues, start=1):
        if not isinstance(issue, dict):
            errors.append(f"capabilitySummary issue {index} must be an object")
            continue
        for field in sorted(set(issue) - CAPABILITY_ISSUE_FIELDS):
            errors.append(
                f"capabilitySummary issue {index} field {field} is not allowed"
            )
        for field in ("title", "statement", "boundary"):
            if not _non_empty_string(issue.get(field)):
                errors.append(
                    f"capabilitySummary issue {index} {field} is required"
                )
            elif _contains_readiness_decision(issue.get(field)):
                errors.append(
                    f"capabilitySummary issue {index} {field} must not contain "
                    "project readiness decisions"
                )

        test_refs = issue.get("testRefs")
        valid_test_refs = []
        if not _string_list(test_refs, require_item=True):
            errors.append(
                f"capabilitySummary issue {index} testRefs must contain FAIL tests"
            )
        else:
            if _has_duplicate_strings(test_refs):
                errors.append(
                    f"capabilitySummary issue {index} testRefs must contain "
                    "unique values"
                )
            for test_ref in dict.fromkeys(test_refs):
                if statuses.get(test_ref) != "FAIL":
                    errors.append(
                        f"capabilitySummary issue {index} must reference a FAIL test: "
                        f"{test_ref}"
                    )
                else:
                    valid_test_refs.append(test_ref)
                    issue_indexes_by_test.setdefault(test_ref, []).append(index)

        issue_refs = issue.get("evidenceRefs")
        if not _string_list(issue_refs, require_item=True):
            errors.append(
                f"capabilitySummary issue {index} evidenceRefs must contain evidence"
            )
        else:
            if _has_duplicate_strings(issue_refs):
                errors.append(
                    f"capabilitySummary issue {index} evidenceRefs must contain "
                    "unique values"
                )
            for reference in dict.fromkeys(issue_refs):
                if not any(
                    _evidence_ref_belongs_to_test(parsed, test_ref, reference)
                    for test_ref in valid_test_refs
                ):
                    errors.append(
                        f"capabilitySummary issue {index} evidence does not belong "
                        f"to its testRefs: {reference}"
                    )

    missing_fail_ids = sorted(fail_ids - set(issue_indexes_by_test))
    if missing_fail_ids:
        errors.append(
            "capabilitySummary does not cover FAIL tests: "
            + ", ".join(missing_fail_ids)
        )
    errors.extend(
        _validate_issue_membership(
            dependencies_by_test,
            issue_indexes_by_test,
        )
    )
    return errors


def _raw_observation(requests: List[dict]) -> str:
    if not requests:
        return "Manifest references no request evidence"
    metrics = requests[-1].get("metrics", {})
    parts = []
    if metrics.get("http_status"):
        parts.append(f"HTTP {metrics['http_status']}")
    if metrics.get("time_total"):
        parts.append(f"{metrics['time_total']}s")
    if metrics.get("size_download"):
        parts.append(f"{metrics['size_download']} bytes")
    return " · ".join(parts) if parts else "Request evidence recorded"


def _status_counts(items: List[dict]) -> Dict[str, int]:
    return {
        status: sum(
            1 for item in items if item.get("reviewedStatus") == status
        )
        for status in ("PASS", "FAIL")
    }


def _categories(items: List[dict]) -> List[dict]:
    category_items: "OrderedDict[str, List[dict]]" = OrderedDict()
    for item in items:
        category = str(item.get("category") or "未分类")
        category_items.setdefault(category, []).append(item)
    return [
        {
            "name": name,
            "counts": _status_counts(group),
        }
        for name, group in category_items.items()
    ]


def assemble_assessment(parsed: dict, reviews: dict) -> dict:
    """Merge strict parsed evidence with validated binary reviews."""

    errors = validate_reviews(parsed, reviews)
    if errors:
        raise ValueError("Invalid reviews:\n" + "\n".join(errors))
    try:
        contract = contract_key(parsed.get("run"))
    except ValueError as error:
        raise ValueError(f"Invalid parsed run contract: {error}") from error

    test_reviews = reviews["tests"]
    items: List[dict] = []
    for test_id, test in parsed.get("tests", {}).items():
        review = test_reviews[test_id]
        request_refs = test.get("requestRefs", [])
        requests = [parsed["requests"][request_id] for request_id in request_refs]
        item = {
            "testId": test_id,
            "category": test.get("category", "Unclassified"),
            "name": test.get("name", f"Test {test_id}"),
            "reviewedStatus": review["reviewedStatus"],
            "conclusion": review["conclusion"],
            "logic": review["logic"],
            "rawObservation": _raw_observation(requests),
            "metrics": [request.get("metrics", {}) for request in requests],
            "evidenceRefs": review["evidenceRefs"],
            "evidenceExcerpts": review["evidenceExcerpts"],
            "limitations": review["limitations"],
            "retestInstructions": review["retestInstructions"],
            "requests": requests,
        }
        if review["reviewedStatus"] == "FAIL":
            item["failureAnalysis"] = deepcopy(review["failureAnalysis"])
        items.append(item)

    capability_summary = deepcopy(reviews["capabilitySummary"])
    statuses = {
        item["testId"]: item["reviewedStatus"] for item in items
    }
    capability_summary["generalVerdict"] = derive_general_verdict(
        statuses,
        contract,
    )
    protocol_family = capability_summary["verifiedFacts"]["interfaceProtocol"][
        "family"
    ]
    capability_summary["openCodexCompatibility"] = derive_opencodex_compatibility(
        parsed.get("run", {}),
        statuses,
        protocol_family,
    )

    return {
        "schemaVersion": ASSESSMENT_SCHEMA_VERSION,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "source": parsed.get("source", {}),
        "run": parsed.get("run", {}),
        "tokenTotals": parsed.get("tokenTotals", {}),
        "warnings": parsed.get("warnings", []),
        "summary": {"counts": _status_counts(items)},
        "capabilitySummary": capability_summary,
        "categories": _categories(items),
        "tests": items,
    }


def _assessment_evidence_ref_belongs_to_item(item: dict, reference: str) -> bool:
    if reference.startswith("request:"):
        request_id = reference.split(":", 1)[1]
        requests = item.get("requests", [])
        if not isinstance(requests, list):
            return False
        return any(
            request.get("request_id") == request_id
            for request in requests
            if isinstance(request, dict)
        )
    return (
        reference == f"test:{item.get('testId')}:manifest"
        and not item.get("requests")
    )


def _assessment_test_id_counts(items: List[dict]) -> Dict[str, int]:
    counts: Dict[str, int] = {}
    for item in items:
        if not isinstance(item, dict):
            continue
        test_id = item.get("testId")
        if _non_empty_string(test_id):
            counts[test_id] = counts.get(test_id, 0) + 1
    return counts


def _assessment_items_by_id(items: List[dict]) -> Dict[str, dict]:
    test_id_counts = _assessment_test_id_counts(items)
    items_by_id: Dict[str, dict] = {}
    for item in items:
        if not isinstance(item, dict):
            continue
        test_id = item.get("testId")
        if not _non_empty_string(test_id) or test_id_counts.get(test_id) != 1:
            continue
        items_by_id[test_id] = item
    return items_by_id


def _assessment_fact_evidence_domains(items: List[dict]) -> Dict[str, set[str]]:
    items_by_id = _assessment_items_by_id(items)

    def request_refs(test_ids: set[str]) -> set[str]:
        references = set()
        for test_id in test_ids:
            item = items_by_id.get(test_id)
            if not isinstance(item, dict):
                continue
            requests = item.get("requests", [])
            if not isinstance(requests, list):
                continue
            for request in requests:
                if not isinstance(request, dict):
                    continue
                request_id = request.get("request_id")
                if _non_empty_string(request_id):
                    references.add(f"request:{request_id}")
        return references

    return {
        "interfaceProtocol": request_refs({"002"}),
        "contextWindow": request_refs({"014", "015", "016", "017", "018"}),
        "concurrency": request_refs({"057"}),
    }


def _validate_assessment_failure_analysis(
    item: dict,
    statuses: Dict[str, str],
) -> List[str]:
    errors: List[str] = []
    test_id = item.get("testId", "unknown")
    status = item.get("reviewedStatus")
    analysis = item.get("failureAnalysis")
    if status == "FAIL" and analysis is None:
        return [f"Test {test_id} failureAnalysis is required for FAIL"]
    if status == "PASS" and analysis is not None:
        return [f"Test {test_id} failureAnalysis is only allowed for FAIL"]
    if analysis is None:
        return errors
    if not isinstance(analysis, dict):
        return [f"Test {test_id} failureAnalysis must be an object"]

    for field in sorted(set(analysis) - FAILURE_ANALYSIS_FIELDS):
        errors.append(f"Test {test_id} failureAnalysis field {field} is not allowed")
    if analysis.get("failureKind") not in FAILURE_KINDS:
        errors.append(f"Test {test_id} failureKind is invalid")
    sufficiency = analysis.get("evidenceSufficiency")
    if sufficiency not in EVIDENCE_SUFFICIENCY:
        errors.append(f"Test {test_id} evidenceSufficiency is invalid")
    if not _non_empty_string(analysis.get("supportedClaim")):
        errors.append(f"Test {test_id} supportedClaim is required")
    unsupported = analysis.get("unsupportedClaims")
    if not _string_list(unsupported):
        errors.append(f"Test {test_id} unsupportedClaims must be a string array")
    elif sufficiency in {"LIMITED", "INSUFFICIENT"} and not unsupported:
        errors.append(
            f"Test {test_id} {sufficiency} failureAnalysis requires unsupportedClaims"
        )

    dependencies = analysis.get("dependsOnTestIds")
    if not _string_list(dependencies):
        errors.append(f"Test {test_id} dependsOnTestIds must be a string array")
    else:
        if _has_duplicate_strings(dependencies):
            errors.append(
                f"Test {test_id} dependsOnTestIds must contain unique values"
            )
        for dependency in dict.fromkeys(dependencies):
            if dependency == test_id or statuses.get(dependency) != "FAIL":
                errors.append(
                    f"Test {test_id} dependency must reference another FAIL test: "
                    f"{dependency}"
                )

    evidence_refs = analysis.get("evidenceRefs")
    if not _string_list(evidence_refs, require_item=True):
        errors.append(
            f"Test {test_id} failureAnalysis evidenceRefs must contain evidence"
        )
    else:
        if _has_duplicate_strings(evidence_refs):
            errors.append(
                f"Test {test_id} failureAnalysis evidenceRefs must contain unique values"
            )
        for reference in dict.fromkeys(evidence_refs):
            if not _assessment_evidence_ref_belongs_to_item(item, reference):
                errors.append(
                    f"Test {test_id} failureAnalysis evidence does not belong "
                    f"to the test: {reference}"
                )
    return errors


def _validate_assessment_summary(
    items: List[dict],
    summary: object,
    contract: tuple[str, str],
    run: object,
) -> List[str]:
    errors: List[str] = []
    if not isinstance(summary, dict):
        return ["capabilitySummary must be an object"]
    for field in sorted(set(summary) - ASSESSMENT_CAPABILITY_SUMMARY_FIELDS):
        errors.append(f"capabilitySummary field {field} is not allowed")
    general_verdict = summary.get("generalVerdict")
    if isinstance(general_verdict, dict):
        for field in GENERAL_VERDICT_COUNT_FIELDS:
            if type(general_verdict.get(field)) is not int:
                errors.append(
                    f"capabilitySummary generalVerdict {field} must be an integer"
                )
    statuses = {
        item.get("testId"): item.get("reviewedStatus")
        for item in items
        if isinstance(item, dict)
    }
    try:
        expected_verdict = derive_general_verdict(statuses, contract)
    except ValueError:
        errors.append(
            "capabilitySummary generalVerdict cannot be derived from test statuses"
        )
    else:
        if summary.get("generalVerdict") != expected_verdict:
            errors.append(
                "capabilitySummary generalVerdict does not match test statuses"
            )
    verified_facts = summary.get("verifiedFacts")
    interface_protocol = (
        verified_facts.get("interfaceProtocol")
        if isinstance(verified_facts, dict)
        else None
    )
    protocol_family = (
        interface_protocol.get("family")
        if isinstance(interface_protocol, dict)
        and isinstance(interface_protocol.get("family"), str)
        else "UNKNOWN"
    )
    expected_compatibility = derive_opencodex_compatibility(
        run if isinstance(run, dict) else {},
        statuses,
        protocol_family,
    )
    if summary.get("openCodexCompatibility") != expected_compatibility:
        errors.append(
            "capabilitySummary openCodexCompatibility does not match run "
            "metadata, protocol family, and test statuses"
        )
    if not _non_empty_string(summary.get("headline")):
        errors.append("capabilitySummary headline is required")
    elif _contains_readiness_decision(summary.get("headline")):
        errors.append(
            "capabilitySummary headline must not contain project readiness decisions"
        )
    if summary.get("scopeBoundary") != CAPABILITY_SCOPE_BOUNDARY:
        errors.append("capabilitySummary scopeBoundary is invalid")
    errors.extend(
        validate_verified_facts(
            summary.get("verifiedFacts"),
            _assessment_fact_evidence_domains(items),
        )
    )

    issues = summary.get("issues")
    if not isinstance(issues, list):
        return errors + ["capabilitySummary issues must be an array"]
    items_by_id = _assessment_items_by_id(items)
    fail_ids = {
        test_id
        for test_id, item in items_by_id.items()
        if item.get("reviewedStatus") == "FAIL"
    }
    dependencies_by_test = {
        test_id: [
            dependency
            for dependency in dict.fromkeys(
                item.get("failureAnalysis", {}).get("dependsOnTestIds", [])
            )
            if dependency in fail_ids and dependency != test_id
        ]
        for test_id, item in items_by_id.items()
        if test_id in fail_ids
        and isinstance(item.get("failureAnalysis"), dict)
        and _string_list(
            item.get("failureAnalysis", {}).get("dependsOnTestIds")
        )
    }
    if fail_ids and not 1 <= len(issues) <= 5:
        errors.append(
            "capabilitySummary must contain 1 to 5 issues when FAIL tests exist"
        )
    if not fail_ids and issues:
        errors.append("capabilitySummary issues must be empty when all tests PASS")

    issue_indexes_by_test: Dict[str, List[int]] = {}
    for index, issue in enumerate(issues, start=1):
        if not isinstance(issue, dict):
            errors.append(f"capabilitySummary issue {index} must be an object")
            continue
        for field in sorted(set(issue) - CAPABILITY_ISSUE_FIELDS):
            errors.append(
                f"capabilitySummary issue {index} field {field} is not allowed"
            )
        for field in ("title", "statement", "boundary"):
            if not _non_empty_string(issue.get(field)):
                errors.append(f"capabilitySummary issue {index} {field} is required")
            elif _contains_readiness_decision(issue.get(field)):
                errors.append(
                    f"capabilitySummary issue {index} {field} must not contain "
                    "project readiness decisions"
                )

        valid_items = []
        test_refs = issue.get("testRefs")
        if not _string_list(test_refs, require_item=True):
            errors.append(
                f"capabilitySummary issue {index} testRefs must contain FAIL tests"
            )
        else:
            if _has_duplicate_strings(test_refs):
                errors.append(
                    f"capabilitySummary issue {index} testRefs must contain "
                    "unique values"
                )
            for test_ref in dict.fromkeys(test_refs):
                item = items_by_id.get(test_ref)
                if not item or item.get("reviewedStatus") != "FAIL":
                    errors.append(
                        f"capabilitySummary issue {index} must reference a FAIL test: "
                        f"{test_ref}"
                    )
                else:
                    valid_items.append(item)
                    issue_indexes_by_test.setdefault(test_ref, []).append(index)

        evidence_refs = issue.get("evidenceRefs")
        if not _string_list(evidence_refs, require_item=True):
            errors.append(
                f"capabilitySummary issue {index} evidenceRefs must contain evidence"
            )
        else:
            if _has_duplicate_strings(evidence_refs):
                errors.append(
                    f"capabilitySummary issue {index} evidenceRefs must contain "
                    "unique values"
                )
            for reference in dict.fromkeys(evidence_refs):
                if not any(
                    _assessment_evidence_ref_belongs_to_item(item, reference)
                    for item in valid_items
                ):
                    errors.append(
                        f"capabilitySummary issue {index} evidence does not belong "
                        f"to its testRefs: {reference}"
                    )

    missing_fail_ids = sorted(fail_ids - set(issue_indexes_by_test))
    if missing_fail_ids:
        errors.append(
            "capabilitySummary does not cover FAIL tests: "
            + ", ".join(missing_fail_ids)
        )
    errors.extend(
        _validate_issue_membership(
            dependencies_by_test,
            issue_indexes_by_test,
        )
    )
    return errors


def validate_assessment(assessment: object) -> List[str]:
    """Validate binary cross-field consistency after assessment assembly."""

    if not isinstance(assessment, dict):
        return ["Assessment must be an object"]

    errors: List[str] = []
    if assessment.get("schemaVersion") != ASSESSMENT_SCHEMA_VERSION:
        errors.append("schemaVersion is invalid")
    for field in sorted(ASSESSMENT_FIELDS - set(assessment)):
        errors.append(f"Assessment field {field} is required")
    for field in sorted(set(assessment) - ASSESSMENT_FIELDS):
        errors.append(f"Assessment field {field} is not allowed")

    summary = assessment.get("summary")
    if not isinstance(summary, dict):
        errors.append("summary must be an object")
    source = assessment.get("source")
    if not isinstance(source, dict):
        errors.append("source must be an object")
    run = assessment.get("run")
    if not isinstance(run, dict):
        errors.append("run must be an object")
    try:
        contract = contract_key(run)
    except ValueError:
        errors.append("run contract is invalid")
        contract = ("", "")

    items = assessment.get("tests")
    if not isinstance(items, list):
        return errors + ["tests must be an array"]
    object_items = [item for item in items if isinstance(item, dict)]
    if len(object_items) != len(items):
        errors.append("tests contain a non-object item")
    if any(item.get("reviewedStatus") not in STATUSES for item in object_items):
        errors.append("tests contain a non-binary reviewedStatus")

    test_id_counts = _assessment_test_id_counts(object_items)
    duplicate_test_ids = {
        test_id for test_id, count in test_id_counts.items() if count > 1
    }
    valid_items = []
    seen_test_ids = set()
    for index, item in enumerate(items):
        if not isinstance(item, dict):
            continue
        test_id = item.get("testId")
        if not _non_empty_string(test_id):
            errors.append(f"tests[{index}] testId must be a non-empty string")
            continue
        if test_id in seen_test_ids:
            errors.append(f"tests[{index}] testId is duplicated: {test_id}")
        seen_test_ids.add(test_id)
        if test_id in duplicate_test_ids:
            continue
        valid_items.append(item)

    statuses = {
        item.get("testId"): item.get("reviewedStatus")
        for item in valid_items
    }
    for item in object_items:
        test_id = item.get("testId", "unknown")
        for field in sorted(set(item) - TEST_FIELDS):
            errors.append(f"Test {test_id} field {field} is not allowed")
        for field in ("category", "name", "conclusion", "rawObservation"):
            if not _non_empty_string(item.get(field)):
                errors.append(f"Test {test_id} {field} is required")
        for obsolete_field in ("confidence", "gateLevel"):
            if obsolete_field in item:
                errors.append(
                    f"Test {test_id} {obsolete_field} is not part of "
                    "the per-test assessment.v8 contract"
                )
        logic = item.get("logic")
        if not isinstance(logic, dict):
            errors.append(f"Test {test_id} logic must be an object")
        else:
            for field in sorted(set(logic) - LOGIC_FIELDS):
                errors.append(
                    f"Test {test_id} logic.{field} is not allowed"
                )
        requests = item.get("requests")
        if not isinstance(requests, list):
            errors.append(f"Test {test_id} requests must be an array")
        else:
            for request_index, request in enumerate(requests):
                if not isinstance(request, dict):
                    errors.append(
                        f"Test {test_id} requests[{request_index}] must be an object"
                    )
                    continue
                if not isinstance(request.get("metrics"), dict):
                    errors.append(
                        f"Test {test_id} requests[{request_index}] metrics must be "
                        "an object"
                    )
        errors.extend(_validate_assessment_failure_analysis(item, statuses))
    if contract == V3_CONTRACT:
        for item in valid_items:
            test_id = item.get("testId")
            if (
                test_id not in V3_TOOL_TEST_IDS
                or item.get("reviewedStatus") != "PASS"
            ):
                continue
            errors.extend(
                _validate_v3_tool_pass_requests(test_id, item.get("requests"))
            )

    expected_counts = _status_counts(object_items)
    if isinstance(summary, dict) and summary.get("counts") != expected_counts:
        errors.append("summary counts do not match test results")
    if assessment.get("categories") != _categories(object_items):
        errors.append("categories do not match test results")
    errors.extend(
        _validate_assessment_summary(
            valid_items,
            assessment.get("capabilitySummary"),
            contract,
            run,
        )
    )
    if "overall" in assessment:
        errors.append("overall is not part of the assessment.v8 contract")
    if isinstance(source, dict) and "path" in source:
        errors.append("source must not expose an absolute path")
    return errors
