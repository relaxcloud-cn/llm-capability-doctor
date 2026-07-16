#!/usr/bin/env python3
"""Validate semantic reviews and assemble the Model Doctor assessment."""

from __future__ import annotations

from collections import OrderedDict
from datetime import datetime, timezone
from typing import Dict, List, Set


ASSESSMENT_SCHEMA_VERSION = "llm-capability-doctor.assessment.v2"
STATUSES = {"PASS", "FAIL", "UNSUPPORTED", "UNDETERMINED", "SKIPPED", "ERROR"}
CONFIDENCES = {"high", "medium", "low"}
GATE_LEVELS = {"critical", "important", "observation"}
LOGIC_FIELDS = {"purpose", "method", "passCriteria", "failCriteria", "capabilityBoundary"}


def _catalog_gate_levels(
    catalog_size: int, critical: Set[str], important: Set[str]
) -> Dict[str, str]:
    all_ids = {f"{test_id:03d}" for test_id in range(1, catalog_size + 1)}
    observation = all_ids - critical - important
    return {
        **{test_id: "critical" for test_id in critical},
        **{test_id: "important" for test_id in important},
        **{test_id: "observation" for test_id in observation},
    }


V0_3_CATALOG_GATE_LEVELS = _catalog_gate_levels(
    65,
    {f"{test_id:03d}" for test_id in (*range(1, 7), *range(43, 54))},
    {f"{test_id:03d}" for test_id in (*range(26, 29), *range(35, 40), 60)},
)
CURRENT_CATALOG_GATE_LEVELS = _catalog_gate_levels(
    62,
    {f"{test_id:03d}" for test_id in (*range(1, 7), *range(40, 51))},
    {f"{test_id:03d}" for test_id in (*range(14, 19), *range(32, 37), 57)},
)
CATALOG_GATE_LEVELS = {
    "0.3.0": V0_3_CATALOG_GATE_LEVELS,
    "0.4.0": CURRENT_CATALOG_GATE_LEVELS,
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
    if reference.startswith("test:") and reference.endswith(":raw"):
        parts = reference.split(":")
        return len(parts) == 3 and parts[1] in parsed.get("tests", {})
    return False


def validate_reviews(parsed: dict, reviews: dict) -> List[str]:
    """Return human-readable errors for Codex-authored semantic reviews."""

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
        script_version = parsed.get("run", {}).get("script_version")
        expected_gate = CATALOG_GATE_LEVELS.get(script_version, {}).get(test_id)
        if expected_gate and gate != expected_gate:
            errors.append(
                f"Test {test_id} gateLevel must be {expected_gate} for script {script_version}"
            )
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
                    errors.append(f"Test {test_id} evidence reference does not exist: {reference}")

        if not _string_list(review.get("evidenceExcerpts"), require_item=True):
            errors.append(f"Test {test_id} evidenceExcerpts must contain observable text")
        if not _string_list(review.get("limitations")):
            errors.append(f"Test {test_id} limitations must be a string array")
        if not _string_list(review.get("retestInstructions")):
            errors.append(f"Test {test_id} retestInstructions must be a string array")

        if status == "UNDETERMINED":
            if not _string_list(review.get("limitations"), require_item=True):
                errors.append(f"Test {test_id} UNDETERMINED requires limitations")
            if not _string_list(review.get("retestInstructions"), require_item=True):
                errors.append(f"Test {test_id} UNDETERMINED requires retestInstructions")
        if status == "PASS" and gate == "critical" and confidence == "low":
            errors.append(f"Test {test_id} low-confidence critical PASS is not allowed")

    return errors


def _raw_observation(test: dict, requests: List[dict]) -> str:
    if requests:
        metrics = requests[-1].get("metrics", {})
        parts = []
        if metrics.get("http_status"):
            parts.append(f"HTTP {metrics['http_status']}")
        if metrics.get("time_total"):
            parts.append(f"{metrics['time_total']}s")
        if metrics.get("size_download"):
            parts.append(f"{metrics['size_download']} bytes")
        if parts:
            return " · ".join(parts)
    return str(test.get("detected") or test.get("conclusion") or "No direct observation")


def _status_counts(items: List[dict]) -> Dict[str, int]:
    return {status: sum(1 for item in items if item["reviewedStatus"] == status) for status in sorted(STATUSES)}


def _category_status(items: List[dict]) -> str:
    statuses = {item["reviewedStatus"] for item in items}
    if statuses & {"FAIL", "ERROR"}:
        return "FAIL"
    if statuses & {"UNSUPPORTED", "UNDETERMINED", "SKIPPED"}:
        return "CONDITIONAL"
    return "PASS"


def _overall(items: List[dict]) -> dict:
    blockers = [
        {"testId": item["testId"], "name": item["name"], "status": item["reviewedStatus"], "conclusion": item["conclusion"]}
        for item in items
        if item["gateLevel"] == "critical" and item["reviewedStatus"] in {"FAIL", "ERROR"}
    ]
    conditions = [
        {"testId": item["testId"], "name": item["name"], "status": item["reviewedStatus"], "conclusion": item["conclusion"]}
        for item in items
        if item["gateLevel"] in {"critical", "important"} and item["reviewedStatus"] != "PASS"
        and not (item["gateLevel"] == "critical" and item["reviewedStatus"] in {"FAIL", "ERROR"})
    ]
    if blockers:
        verdict = "BLOCKED"
    elif conditions:
        verdict = "CONDITIONAL"
    else:
        verdict = "READY"
    return {
        "verdict": verdict,
        "counts": _status_counts(items),
        "blockers": blockers,
        "conditions": conditions,
    }


def assemble_assessment(parsed: dict, reviews: dict) -> dict:
    """Merge parsed evidence with validated semantic reviews."""

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
                "rawObservation": _raw_observation(test, requests),
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
            "criticalFailures": [
                item["testId"]
                for item in group
                if item["gateLevel"] == "critical" and item["reviewedStatus"] in {"FAIL", "ERROR"}
            ],
            "unknowns": [
                item["testId"]
                for item in group
                if item["reviewedStatus"] in {"UNSUPPORTED", "UNDETERMINED", "SKIPPED"}
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
    """Validate cross-field consistency after assessment assembly."""

    errors: List[str] = []
    if assessment.get("schemaVersion") != ASSESSMENT_SCHEMA_VERSION:
        errors.append("schemaVersion is invalid")
    items = assessment.get("tests")
    if not isinstance(items, list):
        return errors + ["tests must be an array"]
    expected_counts = _status_counts(items)
    actual_counts = assessment.get("overall", {}).get("counts")
    if actual_counts != expected_counts:
        errors.append("overall counts do not match test results")
    expected_overall = _overall(items)
    if assessment.get("overall", {}).get("verdict") != expected_overall["verdict"]:
        errors.append("overall verdict does not match gate results")
    if any("path" in assessment.get("source", {}) for _ in [0]):
        errors.append("source must not expose an absolute path")
    return errors
