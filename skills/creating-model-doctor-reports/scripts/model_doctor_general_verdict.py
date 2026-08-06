#!/usr/bin/env python3
"""Derive a fixed customer-facing verdict from Model Doctor check statuses."""

from __future__ import annotations

from typing import Mapping


CORE_TEST_IDS = frozenset(
    {
        "001",
        "002",
        "003",
        "004",
        "005",
        "006",
        "007",
        "009",
        "010",
        "011",
        "012",
        "013",
        "014",
        "019",
        "022",
        "031",
        "038",
        "040",
        "041",
        "042",
        "043",
        "044",
        "047",
        "048",
        "049",
        "050",
        "052",
        "053",
        "054",
        "055",
        "057",
    }
)
ENHANCED_TEST_IDS = frozenset(
    {
        "008",
        "015",
        "016",
        "017",
        "018",
        "020",
        "024",
        "033",
        "034",
        "035",
        "036",
        "045",
        "056",
        "059",
        "060",
    }
)
ALL_VERDICT_TEST_IDS = CORE_TEST_IDS | ENHANCED_TEST_IDS

LABELS = {
    "PASS": "通用能力通过",
    "CONDITIONAL_PASS": "通用能力有条件通过",
    "FAIL": "通用能力未通过",
    "NOT_ASSESSED": "通用能力未评定",
}


def derive_general_verdict(statuses: Mapping[str, str]) -> dict:
    """Return the complete deterministic generalVerdict object."""

    unknown_ids = set(statuses) - ALL_VERDICT_TEST_IDS
    if unknown_ids:
        raise ValueError(f"Unknown test ID: {sorted(unknown_ids)[0]}")
    for test_id, status in statuses.items():
        if status not in {"PASS", "FAIL"}:
            raise ValueError(f"Invalid status for test {test_id}: {status}")

    collected = len(statuses)
    passed = sum(status == "PASS" for status in statuses.values())
    passed_core = sum(statuses.get(test_id) == "PASS" for test_id in CORE_TEST_IDS)
    passed_enhanced = sum(
        statuses.get(test_id) == "PASS" for test_id in ENHANCED_TEST_IDS
    )

    if set(statuses) != ALL_VERDICT_TEST_IDS:
        level = "NOT_ASSESSED"
        statement = (
            f"本轮仅采集 {collected}/46 项，证据不足以生成通用能力等级，"
            "因此本轮通用能力未评定。"
        )
    elif passed_core < len(CORE_TEST_IDS):
        level = "FAIL"
        failed_core = len(CORE_TEST_IDS) - passed_core
        statement = (
            f"本轮固定 46 项检测通过 {passed} 项，其中 {failed_core} 项"
            "基础必过能力未满足，因此判定通用能力未通过。"
        )
    elif passed_enhanced < len(ENHANCED_TEST_IDS):
        level = "CONDITIONAL_PASS"
        failed_enhanced = len(ENHANCED_TEST_IDS) - passed_enhanced
        statement = (
            f"本轮固定 46 项检测通过 {passed} 项，31 项基础必过项全部通过；"
            f"{failed_enhanced} 项增强能力存在限制，因此判定通用能力有条件通过。"
        )
    else:
        level = "PASS"
        statement = "本轮固定 46 项检测全部通过，因此判定通用能力通过。"

    return {
        "level": level,
        "label": LABELS[level],
        "collectedTests": collected,
        "passedTests": passed,
        "totalTests": len(ALL_VERDICT_TEST_IDS),
        "passedCoreTests": passed_core,
        "totalCoreTests": len(CORE_TEST_IDS),
        "passedEnhancedTests": passed_enhanced,
        "totalEnhancedTests": len(ENHANCED_TEST_IDS),
        "statement": statement,
    }
