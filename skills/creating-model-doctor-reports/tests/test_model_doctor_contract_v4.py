from __future__ import annotations

import sys
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import model_doctor_assessment as assessment  # noqa: E402
import model_doctor_contracts as contracts  # noqa: E402
import model_doctor_opencodex_compatibility as compatibility  # noqa: E402
from model_doctor_display import TEST_DISPLAY  # noqa: E402


class EvidenceV4ContractTests(unittest.TestCase):
    def test_only_v4_contract_is_supported(self) -> None:
        expected = ("llm-capability-doctor.evidence.v4", "0.12.0")

        self.assertEqual(expected, getattr(contracts, "V4_CONTRACT", None))
        self.assertEqual({expected}, set(contracts.CONTRACT_TEST_IDS))
        self.assertEqual({expected}, set(contracts.CONTRACT_VERDICT_PARTITIONS))
        for old_contract in (
            ("llm-capability-doctor.evidence.v1", "0.9.0"),
            ("llm-capability-doctor.evidence.v2", "0.10.0"),
            ("llm-capability-doctor.evidence.v3", "0.11.0"),
        ):
            with self.subTest(old_contract=old_contract), self.assertRaisesRegex(
                ValueError,
                "Unsupported log schema/version pair",
            ):
                contracts.contract_key({
                    "log_schema": old_contract[0],
                    "script_version": old_contract[1],
                })

    def test_v4_partition_has_46_checks_without_034(self) -> None:
        contract = getattr(contracts, "V4_CONTRACT", None)
        test_ids = contracts.CONTRACT_TEST_IDS.get(contract, frozenset())
        core_ids, enhanced_ids = contracts.CONTRACT_VERDICT_PARTITIONS.get(
            contract,
            (frozenset(), frozenset()),
        )

        self.assertEqual(46, len(test_ids))
        self.assertEqual(32, len(core_ids))
        self.assertEqual(14, len(enhanced_ids))
        self.assertNotIn("034", test_ids)
        self.assertEqual(test_ids, core_ids | enhanced_ids)
        self.assertFalse(core_ids & enhanced_ids)

    def test_report_contracts_advance_with_evidence_v4(self) -> None:
        self.assertEqual(
            "llm-capability-doctor.assessment.v9",
            assessment.ASSESSMENT_SCHEMA_VERSION,
        )
        self.assertEqual(
            ("llm-capability-doctor.evidence.v4", "0.12.0"),
            compatibility.EVIDENCE_CONTRACT,
        )

    def test_check_034_has_no_report_mapping(self) -> None:
        self.assertNotIn("034", TEST_DISPLAY)

    def test_check_022_rule_requires_ordered_labels_without_extras(self) -> None:
        rules = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        section = rules.split("### 022 多字段抽取", 1)[1].split("### 024", 1)[0]

        self.assertIn('["URGENT","DATABASE"]', section)
        self.assertIn("exact order", section)
        self.assertIn("no extra labels", section)
        self.assertNotIn("any order", section)

    def test_check_057_accepts_any_non_empty_protocol_response(self) -> None:
        rules = (
            SKILL_DIR / "references" / "evaluation-rules.md"
        ).read_text(encoding="utf-8")
        section = rules.split("### 057 并发响应时间", 1)[1].split(
            "## 13. Security Business Language", 1
        )[0]

        self.assertIn("non-empty model-visible content", section)
        self.assertIn("HTTP 2xx", section)
        self.assertIn("Do not require an exact wave marker", section)
        self.assertNotIn("has the exact wave marker", section)
        self.assertNotIn("wrong content", section)


if __name__ == "__main__":
    unittest.main()
