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
            "method": "检查 manifest 引用的请求、响应和运行指标。",
            "passCriteria": ["存在完整且一致的直接证据。"],
            "failCriteria": ["证据缺失、错误或与要求冲突。"],
            "capabilityBoundary": "仅证明本次请求中的可观察行为。",
        },
        "evidenceRefs": [f"request:test-{test_id}"],
        "evidenceExcerpts": ["HTTP 200"],
        "limitations": [],
        "retestInstructions": [],
    }


def parsed_with_tests(*test_ids):
    parsed = parse_log(FIXTURES / "minimal.log")
    template_test = parsed["tests"]["001"]
    template_request = parsed["requests"]["test-001"]
    categories = {
        "001": "接口与协议",
        "051": "性能与稳定性",
        "057": "性能与稳定性",
    }
    parsed["tests"] = {}
    parsed["requests"] = {}
    for test_id in test_ids:
        request_id = f"test-{test_id}"
        request = deepcopy(template_request)
        request["request_id"] = request_id
        test = deepcopy(template_test)
        test.update(
            {
                "id": test_id,
                "name": f"Test {test_id}",
                "category": categories.get(test_id, "测试分类"),
                "requestRefs": [request_id],
            }
        )
        parsed["requests"][request_id] = request
        parsed["tests"][test_id] = test
    return parsed


class ModelDoctorAssessmentTests(unittest.TestCase):
    def setUp(self):
        self.parsed = parse_log(FIXTURES / "minimal.log")

    def test_validate_reviews_rejects_missing_test_review(self):
        errors = validate_reviews(self.parsed, {})

        self.assertTrue(any("001" in error and "missing" in error.lower() for error in errors))

    def test_only_pass_and_fail_are_valid_review_statuses(self):
        for status in ("UNSUPPORTED", "UNDETERMINED", "SKIPPED", "ERROR"):
            with self.subTest(status=status):
                review = valid_review(status=status)
                self.assertTrue(validate_reviews(self.parsed, {"001": review}))
        for status in ("PASS", "FAIL"):
            with self.subTest(status=status):
                review = valid_review(status=status)
                self.assertEqual(validate_reviews(self.parsed, {"001": review}), [])

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
        mapping = assessment_module.CURRENT_CATALOG_GATE_LEVELS
        expected_critical = {f"{value:03d}" for value in range(1, 7)} | {
            f"{value:03d}" for value in range(40, 51)
        }
        expected_important = {
            *(f"{value:03d}" for value in range(14, 19)),
            *(f"{value:03d}" for value in range(32, 37)),
            "057",
        }
        all_ids = {f"{value:03d}" for value in range(1, 63)}

        self.assertEqual(set(mapping), all_ids)
        self.assertEqual(
            {test_id for test_id, gate in mapping.items() if gate == "critical"},
            expected_critical,
        )
        self.assertEqual(
            {test_id for test_id, gate in mapping.items() if gate == "important"},
            expected_important,
        )
        self.assertFalse(hasattr(assessment_module, "V0_3_CATALOG_GATE_LEVELS"))
        self.assertFalse(hasattr(assessment_module, "CATALOG_GATE_LEVELS"))

    def test_validate_reviews_rejects_wrong_gate_for_current_catalog(self):
        review = valid_review(gate="observation")

        errors = validate_reviews(self.parsed, {"001": review})

        self.assertTrue(any("001" in error and "critical" in error for error in errors))

    def test_assemble_assessment_uses_binary_v3_contract(self):
        review = valid_review(status="FAIL")
        assessment = assemble_assessment(self.parsed, {"001": review})
        item = assessment["tests"][0]

        self.assertEqual(
            ASSESSMENT_SCHEMA_VERSION,
            "llm-capability-doctor.assessment.v3",
        )
        self.assertEqual(
            assessment["schemaVersion"],
            "llm-capability-doctor.assessment.v3",
        )
        self.assertEqual(item["reviewedStatus"], "FAIL")
        self.assertEqual(set(assessment["overall"]["counts"]), {"PASS", "FAIL"})
        self.assertNotIn("conditions", assessment["overall"])

    def test_category_and_overall_are_binary(self):
        parsed = parsed_with_tests("001", "051")
        assessment = assemble_assessment(
            parsed,
            {
                "001": valid_review(status="PASS", gate="critical"),
                "051": valid_review(
                    test_id="051",
                    status="FAIL",
                    gate="observation",
                ),
            },
        )

        self.assertEqual(
            {item["status"] for item in assessment["categories"]},
            {"PASS", "FAIL"},
        )
        self.assertEqual(assessment["overall"]["verdict"], "READY")
        failed_category = next(
            item for item in assessment["categories"] if item["status"] == "FAIL"
        )
        self.assertEqual(failed_category["failures"], ["051"])
        self.assertNotIn("unknowns", failed_category)

    def test_important_failure_blocks_overall_readiness(self):
        parsed = parsed_with_tests("057")
        assessment = assemble_assessment(
            parsed,
            {
                "057": valid_review(
                    test_id="057",
                    status="FAIL",
                    gate="important",
                )
            },
        )

        self.assertEqual(assessment["overall"]["verdict"], "BLOCKED")
        self.assertEqual(assessment["overall"]["blockers"][0]["testId"], "057")

    def test_assessment_schema_declares_binary_v3_contract(self):
        schema_path = (
            ROOT
            / "skills"
            / "creating-model-doctor-reports"
            / "references"
            / "assessment-schema.json"
        )
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        overall = schema["properties"]["overall"]
        test_schema = schema["properties"]["tests"]["items"]

        self.assertEqual(schema["$id"], "llm-capability-doctor.assessment.v3")
        self.assertEqual(
            schema["properties"]["schemaVersion"]["const"],
            "llm-capability-doctor.assessment.v3",
        )
        self.assertEqual(test_schema["properties"]["reviewedStatus"]["enum"], ["PASS", "FAIL"])
        self.assertEqual(overall["properties"]["verdict"]["enum"], ["READY", "BLOCKED"])
        self.assertIn("blockers", overall["required"])
        self.assertNotIn("conditions", overall["required"])
        self.assertNotIn("conditions", overall.get("properties", {}))

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
            parsed_path.write_text(
                json.dumps(parse_log(FIXTURES / "minimal.log")),
                encoding="utf-8",
            )
            reviews_path.write_text(
                json.dumps({"001": valid_review()}),
                encoding="utf-8",
            )

            valid = subprocess.run(
                [
                    sys.executable,
                    str(CLI),
                    "validate",
                    str(parsed_path),
                    str(reviews_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertIn("valid", valid.stdout.lower())

            reviews_path.write_text("{}", encoding="utf-8")
            invalid = subprocess.run(
                [
                    sys.executable,
                    str(CLI),
                    "validate",
                    str(parsed_path),
                    str(reviews_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(invalid.returncode, 2)
            self.assertIn("001", invalid.stderr)

    def test_validate_command_rejects_non_evidence_parsed_schema(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            parsed_path = directory / "parsed.json"
            reviews_path = directory / "reviews.json"
            parsed = parse_log(FIXTURES / "minimal.log")
            parsed["schemaVersion"] = "llm-capability-doctor.parsed-log.v1"
            parsed_path.write_text(json.dumps(parsed), encoding="utf-8")
            reviews_path.write_text(
                json.dumps({"001": valid_review()}),
                encoding="utf-8",
            )

            result = subprocess.run(
                [
                    sys.executable,
                    str(CLI),
                    "validate",
                    str(parsed_path),
                    str(reviews_path),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 2)
            self.assertIn("parsed-evidence.v1", result.stderr)


if __name__ == "__main__":
    unittest.main()
