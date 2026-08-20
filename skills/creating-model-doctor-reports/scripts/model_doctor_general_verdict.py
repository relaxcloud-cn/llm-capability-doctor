#!/usr/bin/env python3
"""Derive a fixed customer-facing verdict from Model Doctor check statuses."""

from __future__ import annotations

from typing import Mapping

from model_doctor_contracts import (
    CONTRACT_TEST_IDS,
    CONTRACT_VERDICT_PARTITIONS,
    V4_CONTRACT,
)

CORE_TEST_IDS, ENHANCED_TEST_IDS = CONTRACT_VERDICT_PARTITIONS[V4_CONTRACT]
ALL_VERDICT_TEST_IDS = CONTRACT_TEST_IDS[V4_CONTRACT]

LABELS = {
    "PASS": "通用能力通过",
    "CONDITIONAL_PASS": "通用能力有条件通过",
    "FAIL": "通用能力未通过",
    "NOT_ASSESSED": "通用能力未评定",
}


def derive_general_verdict(
    statuses: Mapping[str, str],
    contract: tuple[str, str],
) -> dict:
    """Return the complete deterministic generalVerdict object."""

    try:
        all_test_ids = CONTRACT_TEST_IDS[contract]
        core_test_ids, enhanced_test_ids = CONTRACT_VERDICT_PARTITIONS[contract]
    except (KeyError, TypeError) as error:
        raise ValueError(f"Unsupported evidence contract: {contract!r}") from error

    unknown_ids = set(statuses) - all_test_ids
    if unknown_ids:
        raise ValueError(f"Unknown test ID: {sorted(unknown_ids)[0]}")
    for test_id, status in statuses.items():
        if status not in {"PASS", "FAIL"}:
            raise ValueError(f"Invalid status for test {test_id}: {status}")

    collected = len(statuses)
    passed = sum(status == "PASS" for status in statuses.values())
    passed_core = sum(
        statuses.get(test_id) == "PASS" for test_id in core_test_ids
    )
    passed_enhanced = sum(
        statuses.get(test_id) == "PASS" for test_id in enhanced_test_ids
    )
    total_tests = len(all_test_ids)
    total_core = len(core_test_ids)
    total_enhanced = len(enhanced_test_ids)

    if set(statuses) != all_test_ids:
        level = "NOT_ASSESSED"
        statement = (
            f"本轮仅采集 {collected}/{total_tests} 项，证据不足以生成通用能力等级，"
            "因此本轮通用能力未评定。"
        )
    elif passed_core < total_core:
        level = "FAIL"
        failed_core = total_core - passed_core
        statement = (
            f"本轮固定 {total_tests} 项检测通过 {passed} 项，其中 {failed_core} 项"
            "基础必过能力未满足，因此判定通用能力未通过。"
        )
    elif passed_enhanced < total_enhanced:
        level = "CONDITIONAL_PASS"
        failed_enhanced = total_enhanced - passed_enhanced
        statement = (
            f"本轮固定 {total_tests} 项检测通过 {passed} 项，"
            f"{total_core} 项基础必过项全部通过；"
            f"{failed_enhanced} 项增强能力存在限制，因此判定通用能力有条件通过。"
        )
    else:
        level = "PASS"
        statement = (
            f"本轮固定 {total_tests} 项检测全部通过，因此判定通用能力通过。"
        )

    return {
        "level": level,
        "label": LABELS[level],
        "collectedTests": collected,
        "passedTests": passed,
        "totalTests": total_tests,
        "passedCoreTests": passed_core,
        "totalCoreTests": total_core,
        "passedEnhancedTests": passed_enhanced,
        "totalEnhancedTests": total_enhanced,
        "statement": statement,
    }
