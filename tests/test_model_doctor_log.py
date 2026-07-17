import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
CLI = SCRIPT_DIR / "model_doctor_report.py"
FIXTURES = ROOT / "tests" / "fixtures" / "model-doctor"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import parse_log, redact_text, test_packet  # noqa: E402


class ModelDoctorLogTests(unittest.TestCase):
    def write_variant(self, source_name, transform):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        source = (FIXTURES / source_name).read_text(encoding="utf-8")
        path = Path(directory.name) / source_name
        path.write_text(transform(source), encoding="utf-8")
        return path

    def test_parser_requires_evidence_v1(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "old.log"
            path.write_text(
                "========== MODEL DOCTOR RUN ==========\n"
                "script_version: 0.6.1\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "log_schema"):
                parse_log(path)

    def test_parse_log_extracts_strict_evidence_v1(self):
        parsed = parse_log(FIXTURES / "minimal.log")

        self.assertEqual(
            parsed["schemaVersion"],
            "llm-capability-doctor.parsed-evidence.v1",
        )
        self.assertEqual(parsed["run"]["log_schema"], "llm-capability-doctor.evidence.v1")
        self.assertEqual(parsed["run"]["script_version"], "0.7.0")
        self.assertEqual(parsed["requests"]["test-001"]["metrics"]["http_status"], "200")
        self.assertEqual(parsed["tests"]["001"]["requestRefs"], ["test-001"])
        self.assertEqual(parsed["summary"]["request_count"], "1")
        self.assertEqual(parsed["warnings"], [])

    def test_parser_uses_only_explicit_request_refs(self):
        parsed = parse_log(FIXTURES / "mixed.log")

        self.assertEqual(
            parsed["tests"]["047"]["requestRefs"],
            ["test-047", "test-047-follow"],
        )
        self.assertEqual(len(parsed["tests"]["056"]["requestRefs"]), 5)
        for test in parsed["tests"].values():
            self.assertNotIn("result", test)
            self.assertNotIn("rawResponse", test)

    def test_parser_rejects_missing_request_reference(self):
        path = self.write_variant(
            "minimal.log",
            lambda value: value.replace(
                "request_refs: test-001",
                "request_refs: test-missing",
            ),
        )

        with self.assertRaisesRegex(ValueError, "missing request"):
            parse_log(path)

    def test_parser_rejects_empty_or_duplicate_request_references(self):
        for refs in ("test-001,", "test-001,test-001"):
            with self.subTest(refs=refs):
                path = self.write_variant(
                    "minimal.log",
                    lambda value, refs=refs: value.replace(
                        "request_refs: test-001",
                        f"request_refs: {refs}",
                    ),
                )
                with self.assertRaisesRegex(ValueError, "request_refs"):
                    parse_log(path)

    def test_parser_rejects_duplicate_request_and_test_blocks(self):
        source = (FIXTURES / "minimal.log").read_text(encoding="utf-8")
        request_block = source[
            source.index("========== REQUEST test-001 BEGIN ==========") :
            source.index("========== TEST-001 BEGIN ==========")
        ]
        test_block = source[
            source.index("========== TEST-001 BEGIN ==========") :
            source.index("========== RUN SUMMARY ==========")
        ]
        for label, block, insertion in (
            ("request", request_block, "========== TEST-001 BEGIN =========="),
            ("test", test_block, "========== RUN SUMMARY =========="),
        ):
            with self.subTest(label=label):
                path = self.write_variant(
                    "minimal.log",
                    lambda value, block=block, insertion=insertion: value.replace(
                        insertion,
                        block + insertion,
                        1,
                    ),
                )
                with self.assertRaisesRegex(ValueError, f"Duplicate {label}"):
                    parse_log(path)

    def test_parser_rejects_declared_count_mismatches(self):
        replacements = (
            ("selected_test_count: 1", "selected_test_count: 2"),
            ("request_count: 1", "request_count: 2"),
            ("test_manifest_count: 1", "test_manifest_count: 2"),
        )
        for before, after in replacements:
            with self.subTest(field=before.split(":", 1)[0]):
                path = self.write_variant(
                    "minimal.log",
                    lambda value, before=before, after=after: value.replace(
                        before,
                        after,
                    ),
                )
                with self.assertRaisesRegex(ValueError, "count"):
                    parse_log(path)

    def test_test_packet_contains_only_manifest_referenced_evidence(self):
        parsed = parse_log(FIXTURES / "mixed.log")
        packet = test_packet(parsed, "047")

        self.assertEqual(packet["test"]["id"], "047")
        self.assertEqual(
            [item["request_id"] for item in packet["requests"]],
            ["test-047", "test-047-follow"],
        )
        self.assertNotIn("test-055-repeat", str(packet))

    def test_parse_log_records_name_size_hash_but_not_absolute_path(self):
        path = FIXTURES / "minimal.log"
        original = path.read_bytes()
        parsed = parse_log(path)

        self.assertEqual(parsed["source"]["fileName"], "minimal.log")
        self.assertEqual(parsed["source"]["size"], len(original))
        self.assertEqual(parsed["source"]["sha256"], hashlib.sha256(original).hexdigest())
        self.assertNotIn("path", parsed["source"])
        self.assertEqual(path.read_bytes(), original)

    def test_parse_log_preserves_collector_masked_api_key(self):
        path = self.write_variant(
            "minimal.log",
            lambda value: value.replace(
                "api_key: [REDACTED]",
                "api_key: sk-t********5678",
                1,
            ),
        )

        parsed = parse_log(path)

        self.assertEqual(parsed["run"]["api_key"], "sk-t********5678")

    def test_redact_text_removes_headers_query_and_json_credentials(self):
        value = (
            "https://model.example/v1?q=ok&api_key=query-secret\n"
            "Authorization: Bearer header-secret\n"
            "Set-Cookie: session=cookie-secret\n"
            '{"api_key":"json-secret","password":"password-secret"}\n'
            "echo json-secret"
        )
        redacted = redact_text(value)

        for secret in (
            "query-secret",
            "header-secret",
            "cookie-secret",
            "json-secret",
            "password-secret",
        ):
            self.assertNotIn(secret, redacted)
        self.assertGreaterEqual(redacted.count("[REDACTED]"), 5)

    def test_parse_log_redacts_echoed_secret_values_across_blocks(self):
        parsed = parse_log(FIXTURES / "mixed.log")
        serialized = str(parsed)

        for secret in (
            "query-secret",
            "header-secret",
            "cookie-secret",
            "json-secret",
        ):
            self.assertNotIn(secret, serialized)


class ModelDoctorCliLogTests(unittest.TestCase):
    def run_cli(self, *arguments):
        return subprocess.run(
            [sys.executable, str(CLI), *map(str, arguments)],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_parse_summary_and_packet_commands(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            parsed_path = Path(temporary_directory) / "parsed.json"
            result = self.run_cli("parse", FIXTURES / "mixed.log", "--output", parsed_path)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(str(parsed_path.resolve()), result.stdout)
            parsed = json.loads(parsed_path.read_text(encoding="utf-8"))
            self.assertEqual(set(parsed["tests"]), {"047", "056"})

            summary = self.run_cli("summary", parsed_path)
            self.assertEqual(summary.returncode, 0, summary.stderr)
            summary_value = json.loads(summary.stdout)
            self.assertEqual(summary_value["testCount"], 2)
            self.assertEqual(summary_value["requestCount"], 7)
            self.assertNotIn("originalStatusCounts", summary_value)
            self.assertNotIn("REQUEST BODY", summary.stdout)

            packet = self.run_cli("packet", parsed_path, "--ids", "047,056")
            self.assertEqual(packet.returncode, 0, packet.stderr)
            packet_value = json.loads(packet.stdout)
            self.assertEqual(set(packet_value["packets"]), {"047", "056"})
            self.assertEqual(len(packet_value["packets"]["047"]["requests"]), 2)
            self.assertEqual(len(packet_value["packets"]["056"]["requests"]), 5)

    def test_parse_does_not_overwrite_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            parsed_path = Path(temporary_directory) / "parsed.json"
            parsed_path.write_text("keep me", encoding="utf-8")

            result = self.run_cli("parse", FIXTURES / "minimal.log", "--output", parsed_path)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(parsed_path.read_text(encoding="utf-8"), "keep me")
            generated_path = Path(result.stdout.strip())
            self.assertNotEqual(generated_path, parsed_path)
            self.assertRegex(
                generated_path.name,
                r"^parsed-\d{8}-\d{6}(?:-\d+)?\.json$",
            )
            self.assertTrue(generated_path.exists())


if __name__ == "__main__":
    unittest.main()
