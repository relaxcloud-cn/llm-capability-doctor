#!/usr/bin/env python3
"""Validate binary semantic reviews and assemble a Model Doctor assessment."""

from __future__ import annotations

from collections import OrderedDict
from datetime import datetime, timezone
from typing import Dict, List


ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v4"
STATUSES = {"PASS", "FAIL"}
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
}


def _non_empty_string(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _string_list(value: object, require_item: bool = False) -> bool:
    if not isinstance(value, list):
        return False
    if require_item and not value:
        return False
    return all(_non_empty_string(item) for item in value)


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


def validate_reviews(parsed: dict, reviews: dict) -> List[str]:
    """Return human-readable errors for Skill-authored binary reviews."""

    errors: List[str] = []
    parsed_tests = parsed.get("tests", {})
    if not isinstance(reviews, dict):
        return ["reviews must be a JSON object keyed by test ID"]

    for test_id in parsed_tests:
        if test_id not in reviews:
            errors.append(f"Test {test_id} review is missing")

    for test_id in reviews:
        if test_id not in parsed_tests:
            errors.append(f"Review contains unknown test ID {test_id}")

    for test_id, review in reviews.items():
        if test_id not in parsed_tests or not isinstance(review, dict):
            if not isinstance(review, dict):
                errors.append(f"Test {test_id} review must be an object")
            continue

        if review.get("testId") != test_id:
            errors.append(f"Test {test_id} testId must match its object key")
        status = review.get("reviewedStatus")
        if status not in STATUSES:
            errors.append(f"Test {test_id} reviewedStatus is invalid")
        for obsolete_field in ("confidence", "gateLevel"):
            if obsolete_field in review:
                errors.append(
                    f"Test {test_id} {obsolete_field} is not part of the v4 review contract"
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
            1 for item in items if item["reviewedStatus"] == status
        )
        for status in ("PASS", "FAIL")
    }


def _categories(items: List[dict]) -> List[dict]:
    category_items: "OrderedDict[str, List[dict]]" = OrderedDict()
    for item in items:
        category_items.setdefault(item["category"], []).append(item)
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

    items: List[dict] = []
    for test_id, test in parsed.get("tests", {}).items():
        review = reviews[test_id]
        request_refs = test.get("requestRefs", [])
        requests = [parsed["requests"][request_id] for request_id in request_refs]
        items.append(
            {
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
        )

    return {
        "schemaVersion": ASSESSMENT_SCHEMA_VERSION,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "source": parsed.get("source", {}),
        "run": parsed.get("run", {}),
        "tokenTotals": parsed.get("tokenTotals", {}),
        "warnings": parsed.get("warnings", []),
        "summary": {"counts": _status_counts(items)},
        "categories": _categories(items),
        "tests": items,
    }


def validate_assessment(assessment: dict) -> List[str]:
    """Validate binary cross-field consistency after assessment assembly."""

    errors: List[str] = []
    if assessment.get("schemaVersion") != ASSESSMENT_SCHEMA_VERSION:
        errors.append("schemaVersion is invalid")
    for field in sorted(set(assessment) - ASSESSMENT_FIELDS):
        errors.append(f"Assessment field {field} is not allowed")
    items = assessment.get("tests")
    if not isinstance(items, list):
        return errors + ["tests must be an array"]
    if any(item.get("reviewedStatus") not in STATUSES for item in items):
        errors.append("tests contain a non-binary reviewedStatus")
    for item in items:
        test_id = item.get("testId", "unknown")
        for field in sorted(set(item) - TEST_FIELDS):
            errors.append(f"Test {test_id} field {field} is not allowed")
        for obsolete_field in ("confidence", "gateLevel"):
            if obsolete_field in item:
                errors.append(
                    f"Test {test_id} {obsolete_field} is not part of "
                    "the per-test v4 contract"
                )
        logic = item.get("logic")
        if isinstance(logic, dict):
            for field in sorted(set(logic) - LOGIC_FIELDS):
                errors.append(
                    f"Test {test_id} logic.{field} is not allowed"
                )
    expected_counts = _status_counts(items)
    if assessment.get("summary", {}).get("counts") != expected_counts:
        errors.append("summary counts do not match test results")
    if assessment.get("categories") != _categories(items):
        errors.append("categories do not match test results")
    if "overall" in assessment:
        errors.append("overall is not part of the per-test v4 contract")
    if "path" in assessment.get("source", {}):
        errors.append("source must not expose an absolute path")
    return errors
