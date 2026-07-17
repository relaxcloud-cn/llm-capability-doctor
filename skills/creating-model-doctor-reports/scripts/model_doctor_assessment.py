#!/usr/bin/env python3
"""Validate binary semantic reviews and assemble a Model Doctor assessment."""

from __future__ import annotations

from collections import OrderedDict
from datetime import datetime, timezone
from typing import Dict, List, Set


ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v3"
STATUSES = {"PASS", "FAIL"}
CONFIDENCES = {"high", "medium", "low"}
GATE_LEVELS = {"critical", "important", "observation"}
LOGIC_FIELDS = {
    "purpose",
    "method",
    "passCriteria",
    "failCriteria",
    "capabilityBoundary",
}


def _catalog_gate_levels(
    catalog_size: int,
    critical: Set[str],
    important: Set[str],
) -> Dict[str, str]:
    all_ids = {f"{test_id:03d}" for test_id in range(1, catalog_size + 1)}
    observation = all_ids - critical - important
    return {
        **{test_id: "critical" for test_id in critical},
        **{test_id: "important" for test_id in important},
        **{test_id: "observation" for test_id in observation},
    }


CURRENT_CATALOG_GATE_LEVELS = _catalog_gate_levels(
    62,
    {f"{test_id:03d}" for test_id in (*range(1, 7), *range(40, 51))},
    {f"{test_id:03d}" for test_id in (*range(14, 19), *range(32, 37), 57)},
)


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
        confidence = review.get("confidence")
        gate = review.get("gateLevel")
        if status not in STATUSES:
            errors.append(f"Test {test_id} reviewedStatus is invalid")
        if confidence not in CONFIDENCES:
            errors.append(f"Test {test_id} confidence is invalid")
        if gate not in GATE_LEVELS:
            errors.append(f"Test {test_id} gateLevel is invalid")
        expected_gate = CURRENT_CATALOG_GATE_LEVELS.get(test_id)
        if expected_gate and gate != expected_gate:
            errors.append(f"Test {test_id} gateLevel must be {expected_gate}")
        if not _non_empty_string(review.get("conclusion")):
            errors.append(f"Test {test_id} conclusion is required")

        logic = review.get("logic")
        if not isinstance(logic, dict):
            errors.append(f"Test {test_id} logic must be an object")
        else:
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
        if status == "PASS" and gate == "critical" and confidence == "low":
            errors.append(
                f"Test {test_id} low-confidence critical PASS is not allowed"
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


def _category_status(items: List[dict]) -> str:
    return "PASS" if all(item["reviewedStatus"] == "PASS" for item in items) else "FAIL"


def _summary(item: dict) -> dict:
    return {
        "testId": item["testId"],
        "name": item["name"],
        "status": item["reviewedStatus"],
        "conclusion": item["conclusion"],
    }


def _overall(items: List[dict]) -> dict:
    blockers = [
        _summary(item)
        for item in items
        if item["gateLevel"] in {"critical", "important"}
        and item["reviewedStatus"] == "FAIL"
    ]
    return {
        "verdict": "BLOCKED" if blockers else "READY",
        "counts": _status_counts(items),
        "blockers": blockers,
    }


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
                "gateLevel": review["gateLevel"],
                "reviewedStatus": review["reviewedStatus"],
                "confidence": review["confidence"],
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

    category_items: "OrderedDict[str, List[dict]]" = OrderedDict()
    for item in items:
        category_items.setdefault(item["category"], []).append(item)
    categories = [
        {
            "name": name,
            "status": _category_status(group),
            "counts": _status_counts(group),
            "failures": [
                item["testId"]
                for item in group
                if item["reviewedStatus"] == "FAIL"
            ],
        }
        for name, group in category_items.items()
    ]

    return {
        "schemaVersion": ASSESSMENT_SCHEMA_VERSION,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "source": parsed.get("source", {}),
        "run": parsed.get("run", {}),
        "tokenTotals": parsed.get("tokenTotals", {}),
        "warnings": parsed.get("warnings", []),
        "overall": _overall(items),
        "categories": categories,
        "tests": items,
    }


def validate_assessment(assessment: dict) -> List[str]:
    """Validate binary cross-field consistency after assessment assembly."""

    errors: List[str] = []
    if assessment.get("schemaVersion") != ASSESSMENT_SCHEMA_VERSION:
        errors.append("schemaVersion is invalid")
    items = assessment.get("tests")
    if not isinstance(items, list):
        return errors + ["tests must be an array"]
    if any(item.get("reviewedStatus") not in STATUSES for item in items):
        errors.append("tests contain a non-binary reviewedStatus")
    expected_overall = _overall(items)
    actual_overall = assessment.get("overall", {})
    if actual_overall.get("counts") != expected_overall["counts"]:
        errors.append("overall counts do not match test results")
    if actual_overall.get("verdict") != expected_overall["verdict"]:
        errors.append("overall verdict does not match gate results")
    if actual_overall.get("blockers") != expected_overall["blockers"]:
        errors.append("overall blockers do not match gate results")
    if "conditions" in actual_overall:
        errors.append("overall must not contain conditions")
    if "path" in assessment.get("source", {}):
        errors.append("source must not expose an absolute path")
    return errors
