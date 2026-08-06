from __future__ import annotations

import sys
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_general_verdict import (  # noqa: E402
    CORE_TEST_IDS,
    ENHANCED_TEST_IDS,
    derive_general_verdict,
)
from model_doctor_log import RETAINED_TEST_IDS  # noqa: E402


class GeneralVerdictTests(unittest.TestCase):
    def setUp(self) -> None:
        self.all_pass = {test_id: "PASS" for test_id in RETAINED_TEST_IDS}

    def test_partition_covers_all_retained_tests_once(self) -> None:
        self.assertEqual(CORE_TEST_IDS & ENHANCED_TEST_IDS, frozenset())
        self.assertEqual(
            CORE_TEST_IDS | ENHANCED_TEST_IDS,
            frozenset(RETAINED_TEST_IDS),
        )
        self.assertEqual(len(CORE_TEST_IDS), 31)
        self.assertEqual(len(ENHANCED_TEST_IDS), 15)

    def test_all_checks_pass(self) -> None:
        verdict = derive_general_verdict(self.all_pass)

        self.assertEqual(verdict["level"], "PASS")
        self.assertEqual(verdict["label"], "通用能力通过")
        self.assertEqual(verdict["collectedTests"], 46)
        self.assertEqual(verdict["passedTests"], 46)
        self.assertEqual(verdict["passedCoreTests"], 31)
        self.assertEqual(verdict["passedEnhancedTests"], 15)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测全部通过，因此判定通用能力通过。",
        )

    def test_enhanced_failure_is_conditional_pass(self) -> None:
        statuses = dict(self.all_pass)
        statuses["060"] = "FAIL"

        verdict = derive_general_verdict(statuses)

        self.assertEqual(verdict["level"], "CONDITIONAL_PASS")
        self.assertEqual(verdict["label"], "通用能力有条件通过")
        self.assertEqual(verdict["passedTests"], 45)
        self.assertEqual(verdict["passedCoreTests"], 31)
        self.assertEqual(verdict["passedEnhancedTests"], 14)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测通过 45 项，31 项基础必过项全部通过；"
            "1 项增强能力存在限制，因此判定通用能力有条件通过。",
        )

    def test_core_failure_is_fail(self) -> None:
        statuses = dict(self.all_pass)
        statuses["001"] = "FAIL"

        verdict = derive_general_verdict(statuses)

        self.assertEqual(verdict["level"], "FAIL")
        self.assertEqual(verdict["label"], "通用能力未通过")
        self.assertEqual(verdict["passedTests"], 45)
        self.assertEqual(verdict["passedCoreTests"], 30)
        self.assertEqual(
            verdict["statement"],
            "本轮固定 46 项检测通过 45 项，其中 1 项基础必过能力未满足，"
            "因此判定通用能力未通过。",
        )

    def test_core_failure_takes_precedence_over_enhanced_failure(self) -> None:
        statuses = dict(self.all_pass)
        statuses["001"] = "FAIL"
        statuses["060"] = "FAIL"

        verdict = derive_general_verdict(statuses)

        self.assertEqual(verdict["level"], "FAIL")
        self.assertEqual(verdict["passedTests"], 44)
        self.assertEqual(verdict["passedCoreTests"], 30)
        self.assertEqual(verdict["passedEnhancedTests"], 14)

    def test_partial_collection_is_not_assessed(self) -> None:
        verdict = derive_general_verdict({"001": "PASS"})

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
            derive_general_verdict(statuses)


if __name__ == "__main__":
    unittest.main()
