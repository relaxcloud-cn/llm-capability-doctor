import json
import subprocess
import sys
import tempfile
import unittest
from copy import deepcopy
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
CLI = SCRIPT_DIR / "model_doctor_report.py"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

import model_doctor_assessment as assessment_module  # noqa: E402
from model_doctor_assessment import (  # noqa: E402
    ASSESSMENT_SCHEMA_VERSION,
    assemble_assessment,
    validate_assessment,
    validate_reviews,
)
from model_doctor_log import parse_log  # noqa: E402


def valid_review(test_id="001", status="PASS", gate="critical", confidence="high"):
    return {
        "testId": test_id,
        "reviewedStatus": status,
        "confidence": confidence,
        "gateLevel": gate,
        "conclusion": "可观察证据满足该检测项要求。",
        "logic": {
            "purpose": "验证目标能力。",
            "method": "检查请求、响应和运行指标。",
            "passCriteria": ["存在完整且一致的直接证据。"],
            "failCriteria": ["响应与要求冲突。"],
            "capabilityBoundary": "仅证明本次请求中的可观察行为。",
        },
        "evidenceRefs": [f"request:test-{test_id}"],
        "evidenceExcerpts": ["HTTP 200"],
        "limitations": [],
        "retestInstructions": [],
    }


class ModelDoctorAssessmentTests(unittest.TestCase):
    def setUp(self):
        self.parsed = parse_log(FIXTURES / "minimal.log")

    def test_validate_reviews_rejects_missing_test_review(self):
        errors = validate_reviews(self.parsed, {})

        self.assertTrue(any("001" in error and "missing" in error.lower() for error in errors))

    def test_validate_reviews_rejects_pass_without_evidence(self):
        review = valid_review()
        review["evidenceRefs"] = []

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("evidenceRefs" in error for error in errors))

    def test_validate_reviews_rejects_invalid_enums_and_incomplete_logic(self):
        review = valid_review()
        review["reviewedStatus"] = "MAYBE"
        review["confidence"] = "certain"
        review["gateLevel"] = "optional"
        review["logic"]["method"] = ""

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertGreaterEqual(len(errors), 4)

    def test_validate_reviews_requires_unknown_reason_and_retest(self):
        review = valid_review(status="UNDETERMINED", confidence="low")
        review["limitations"] = []
        review["retestInstructions"] = []

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("limitations" in error for error in errors))
        self.assertTrue(any("retestInstructions" in error for error in errors))

    def test_validate_reviews_rejects_low_confidence_critical_pass(self):
        review = valid_review(confidence="low")

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("low-confidence" in error for error in errors))

    def test_validate_reviews_rejects_unknown_evidence_reference(self):
        review = valid_review()
        review["evidenceRefs"] = ["request:missing"]

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("request:missing" in error for error in errors))

    def test_current_catalog_gate_mapping_covers_all_62_tests(self):
        mapping = getattr(assessment_module, "CURRENT_CATALOG_GATE_LEVELS", {})
        expected_critical = {f"{value:03d}" for value in range(1, 7)} | {
            f"{value:03d}" for value in range(40, 51)
        }
        expected_important = {
            *(f"{value:03d}" for value in range(14, 19)),
            *(f"{value:03d}" for value in range(32, 37)),
            "057",
        }
        all_ids = {f"{value:03d}" for value in range(1, 63)}
        expected_observation = all_ids - expected_critical - expected_important

        self.assertEqual(set(mapping), all_ids)
        self.assertEqual({test_id for test_id, gate in mapping.items() if gate == "critical"}, expected_critical)
        self.assertEqual({test_id for test_id, gate in mapping.items() if gate == "important"}, expected_important)
        self.assertEqual({test_id for test_id, gate in mapping.items() if gate == "observation"}, expected_observation)

    def test_historical_v0_3_gate_mapping_remains_available(self):
        mapping = getattr(assessment_module, "V0_3_CATALOG_GATE_LEVELS", {})
        expected_critical = {f"{value:03d}" for value in range(1, 7)} | {
            f"{value:03d}" for value in range(43, 54)
        }
        expected_important = {
            *(f"{value:03d}" for value in range(26, 29)),
            *(f"{value:03d}" for value in range(35, 40)),
            "060",
        }
        all_ids = {f"{value:03d}" for value in range(1, 66)}

        self.assertEqual(set(mapping), all_ids)
        self.assertEqual({test_id for test_id, gate in mapping.items() if gate == "critical"}, expected_critical)
        self.assertEqual({test_id for test_id, gate in mapping.items() if gate == "important"}, expected_important)
        self.assertEqual(
            {test_id for test_id, gate in mapping.items() if gate == "observation"},
            all_ids - expected_critical - expected_important,
        )

    def test_current_and_historical_62_item_versions_share_the_gate_mapping(self):
        mappings = getattr(assessment_module, "CATALOG_GATE_LEVELS", {})

        self.assertEqual(mappings["0.6.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
        self.assertEqual(mappings["0.5.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)
        self.assertEqual(mappings["0.4.0"], assessment_module.CURRENT_CATALOG_GATE_LEVELS)

    def test_validate_reviews_rejects_wrong_gate_for_current_catalog(self):
        self.parsed["run"]["script_version"] = "0.6.0"
        review = valid_review(gate="observation")

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("001" in error and "critical" in error for error in errors))

    def test_validate_reviews_rejects_wrong_gate_for_historical_62_item_catalogs(self):
        for version in ("0.5.0", "0.4.0"):
            with self.subTest(version=version):
                self.parsed["run"]["script_version"] = version
                review = valid_review(gate="observation")

                errors = validate_reviews(self.parsed, {"001": review})

                self.assertTrue(
                    any("001" in error and "critical" in error for error in errors)
                )

    def test_validate_reviews_rejects_wrong_gate_for_historical_v0_3_catalog(self):
        self.parsed["run"]["script_version"] = "0.3.0"
        review = valid_review(gate="observation")

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("001" in error and "critical" in error for error in errors))

    def test_concurrency_ladder_uses_detected_summary_as_raw_observation(self):
        summary = (
            "c4:success=4/4,p50_ms=100,p95_ms=140,max_ms=140,rate_limited=0;"
            "c8:success=8/8,p50_ms=120,p95_ms=190,max_ms=190,rate_limited=0;"
            "c16:success=16/16,p50_ms=150,p95_ms=240,max_ms=250,rate_limited=0;"
            "c32:success=32/32,p50_ms=210,p95_ms=390,max_ms=420,rate_limited=0"
        )
        self.parsed["run"]["script_version"] = "0.6.0"
        test = self.parsed["tests"].pop("001")
        request = self.parsed["requests"].pop("test-001")
        test.update(
            {
                "id": "057",
                "name": "4-32 并发响应时间",
                "category": "性能与稳定性",
                "detected": summary,
                "requestRefs": ["test-057-c4-1"],
            }
        )
        request["request_id"] = "test-057-c4-1"
        self.parsed["tests"]["057"] = test
        self.parsed["requests"]["test-057-c4-1"] = request
        review = valid_review(test_id="057", gate="important")
        review["evidenceRefs"] = ["request:test-057-c4-1"]

        assessment = assemble_assessment(self.parsed, {"057": review})

        self.assertEqual(assessment["tests"][0]["rawObservation"], summary)

    def test_historical_057_keeps_its_request_metrics_as_raw_observation(self):
        self.parsed["run"]["script_version"] = "0.5.0"
        test = self.parsed["tests"].pop("001")
        request = self.parsed["requests"].pop("test-001")
        test.update(
            {
                "id": "057",
                "name": "8 并发性能",
                "category": "性能与稳定性",
                "detected": "8/8",
                "requestRefs": ["test-057-c8-1"],
            }
        )
        request["request_id"] = "test-057-c8-1"
        self.parsed["tests"]["057"] = test
        self.parsed["requests"]["test-057-c8-1"] = request
        review = valid_review(test_id="057", gate="important")
        review["evidenceRefs"] = ["request:test-057-c8-1"]

        assessment = assemble_assessment(self.parsed, {"057": review})

        self.assertEqual(
            assessment["tests"][0]["rawObservation"],
            "HTTP 200 · 1.250000s · 128 bytes",
        )

    def test_assemble_assessment_uses_reviewed_status_as_only_formal_verdict(self):
        review = valid_review(status="FAIL")
        assessment = assemble_assessment(self.parsed, {"001": review})
        item = assessment["tests"][0]

        self.assertEqual(ASSESSMENT_SCHEMA_VERSION, "llm-capability-doctor.assessment.v2")
        self.assertEqual(assessment["schemaVersion"], "llm-capability-doctor.assessment.v2")
        self.assertEqual(item["reviewedStatus"], "FAIL")
        for removed in ("originalStatus", "discrepancy", "originalTest"):
            self.assertNotIn(removed, item)
        self.assertNotIn("summary", assessment)
        self.assertEqual(assessment["overall"]["counts"]["FAIL"], 1)

    def test_assessment_schema_declares_skill_only_v2_contract(self):
        schema_path = (
            ROOT
            / "skills"
            / "creating-model-doctor-reports"
            / "references"
            / "assessment-schema.json"
        )
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        test_schema = schema["properties"]["tests"]["items"]

        self.assertEqual(schema["$id"], "llm-capability-doctor.assessment.v2")
        self.assertEqual(
            schema["properties"]["schemaVersion"]["const"],
            "llm-capability-doctor.assessment.v2",
        )
        self.assertIn("reviewedStatus", test_schema["required"])
        for removed in ("originalStatus", "discrepancy", "originalTest"):
            self.assertNotIn(removed, test_schema["required"])
            self.assertNotIn(removed, test_schema.get("properties", {}))

    def test_overall_verdict_blocks_on_critical_failure(self):
        assessment = assemble_assessment(
            self.parsed,
            {"001": valid_review(status="FAIL", gate="critical")},
        )

        self.assertEqual(assessment["overall"]["verdict"], "BLOCKED")
        self.assertEqual(assessment["overall"]["blockers"][0]["testId"], "001")

    def test_overall_verdict_is_conditional_on_critical_unknown(self):
        review = valid_review(status="UNDETERMINED", gate="critical", confidence="low")
        review["limitations"] = ["响应正文缺失。"]
        review["retestInstructions"] = ["重新运行检测项 001。"]

        assessment = assemble_assessment(self.parsed, {"001": review})

        self.assertEqual(assessment["overall"]["verdict"], "CONDITIONAL")

    def test_overall_verdict_is_ready_when_all_non_observation_gates_pass(self):
        assessment = assemble_assessment(self.parsed, {"001": valid_review()})

        self.assertEqual(assessment["overall"]["verdict"], "READY")
        self.assertEqual(assessment["categories"][0]["counts"]["PASS"], 1)

    def test_validate_assessment_detects_tampered_counts(self):
        assessment = assemble_assessment(self.parsed, {"001": valid_review()})
        tampered = deepcopy(assessment)
        tampered["overall"]["counts"]["PASS"] = 99

        errors = validate_assessment(tampered)

        self.assertTrue(any("counts" in error for error in errors))


class ModelDoctorCliAssessmentTests(unittest.TestCase):
    def test_validate_command_distinguishes_valid_and_invalid_reviews(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            parsed_path = directory / "parsed.json"
            reviews_path = directory / "reviews.json"
            parsed_path.write_text(json.dumps(parse_log(FIXTURES / "minimal.log")), encoding="utf-8")
            reviews_path.write_text(json.dumps({"001": valid_review()}), encoding="utf-8")

            valid = subprocess.run(
                [sys.executable, str(CLI), "validate", str(parsed_path), str(reviews_path)],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertIn("valid", valid.stdout.lower())

            reviews_path.write_text("{}", encoding="utf-8")
            invalid = subprocess.run(
                [sys.executable, str(CLI), "validate", str(parsed_path), str(reviews_path)],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("001", invalid.stderr)


if __name__ == "__main__":
    unittest.main()
