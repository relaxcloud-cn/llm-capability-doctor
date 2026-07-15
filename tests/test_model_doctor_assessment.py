import sys
import unittest
from copy import deepcopy
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

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

    def test_assemble_assessment_preserves_original_and_reviewed_status(self):
        review = valid_review(status="FAIL")
        assessment = assemble_assessment(self.parsed, {"001": review})
        item = assessment["tests"][0]

        self.assertEqual(assessment["schemaVersion"], ASSESSMENT_SCHEMA_VERSION)
        self.assertEqual(item["originalStatus"], "PASS")
        self.assertEqual(item["reviewedStatus"], "FAIL")
        self.assertTrue(item["discrepancy"])

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


if __name__ == "__main__":
    unittest.main()
