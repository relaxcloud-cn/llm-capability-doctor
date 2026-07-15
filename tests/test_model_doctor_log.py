import hashlib
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import parse_log, redact_text, test_packet  # noqa: E402


class ModelDoctorLogTests(unittest.TestCase):
    def test_parse_log_extracts_run_request_test_and_summary(self):
        parsed = parse_log(FIXTURES / "minimal.log")

        self.assertEqual(parsed["schemaVersion"], "llm-capability-doctor.parsed-log.v1")
        self.assertEqual(parsed["run"]["run_id"], "MD-20260715-TEST-001")
        self.assertEqual(parsed["run"]["script_version"], "0.2.0")
        self.assertEqual(parsed["requests"]["test-001"]["metrics"]["http_status"], "200")
        self.assertEqual(parsed["tests"]["001"]["result"], "PASS")
        self.assertEqual(parsed["tests"]["001"]["requestRefs"], ["test-001"])
        self.assertEqual(parsed["summary"]["request_count"], "1")

    def test_parse_log_associates_followup_requests_and_preserves_unknown_tests(self):
        parsed = parse_log(FIXTURES / "mixed.log")

        self.assertEqual(
            parsed["tests"]["047"]["requestRefs"],
            ["test-047", "test-047-follow"],
        )
        self.assertEqual(parsed["tests"]["999"]["name"], "未来检测项")
        self.assertTrue(any("RUN SUMMARY" in warning for warning in parsed["warnings"]))

    def test_test_packet_contains_only_requested_test_evidence(self):
        parsed = parse_log(FIXTURES / "mixed.log")
        packet = test_packet(parsed, "047")

        self.assertEqual(packet["test"]["id"], "047")
        self.assertEqual([item["request_id"] for item in packet["requests"]], ["test-047", "test-047-follow"])
        self.assertNotIn("test-999", str(packet))

    def test_parse_log_records_name_size_hash_but_not_absolute_path(self):
        path = FIXTURES / "minimal.log"
        original = path.read_bytes()
        parsed = parse_log(path)

        self.assertEqual(parsed["source"]["fileName"], "minimal.log")
        self.assertEqual(parsed["source"]["size"], len(original))
        self.assertEqual(parsed["source"]["sha256"], hashlib.sha256(original).hexdigest())
        self.assertNotIn("path", parsed["source"])
        self.assertEqual(path.read_bytes(), original)

    def test_redact_text_removes_headers_query_and_json_credentials(self):
        value = (
            "https://model.example/v1?q=ok&api_key=query-secret\n"
            "Authorization: Bearer header-secret\n"
            "Set-Cookie: session=cookie-secret\n"
            '{"api_key":"json-secret","password":"password-secret"}\n'
            "echo json-secret"
        )
        redacted = redact_text(value)

        for secret in ("query-secret", "header-secret", "cookie-secret", "json-secret", "password-secret"):
            self.assertNotIn(secret, redacted)
        self.assertGreaterEqual(redacted.count("[REDACTED]"), 5)

    def test_parse_log_redacts_echoed_secret_values_across_blocks(self):
        parsed = parse_log(FIXTURES / "mixed.log")
        serialized = str(parsed)

        for secret in ("query-secret", "header-secret", "cookie-secret", "json-secret"):
            self.assertNotIn(secret, serialized)


if __name__ == "__main__":
    unittest.main()
