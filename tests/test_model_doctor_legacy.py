import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
FIXTURE = ROOT / "tests" / "fixtures" / "model-doctor" / "legacy-v0.1.log"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import parse_log  # noqa: E402


class ModelDoctorLegacyTests(unittest.TestCase):
    def test_legacy_catalog_metadata_and_unknown_tests_are_preserved(self):
        parsed = parse_log(FIXTURE)

        self.assertEqual(parsed["run"]["script_version"], "0.1.0")
        self.assertEqual(parsed["run"]["test_count"], "113")
        self.assertEqual(set(parsed["tests"]), {"002", "113"})
        self.assertEqual(parsed["tests"]["113"]["name"], "调查阶段与证据引用完整性")
        self.assertTrue(any("Legacy" in warning and "113" in warning for warning in parsed["warnings"]))
        self.assertTrue(
            any("declared test_count=113" in warning and "discovered 2" in warning for warning in parsed["warnings"])
        )

    def test_legacy_shared_protocol_request_is_linked_by_exact_response(self):
        parsed = parse_log(FIXTURE)

        self.assertEqual(parsed["tests"]["002"]["requestRefs"], ["protocol-1"])
        self.assertEqual(parsed["tests"]["113"]["requestRefs"], [])

    def test_exact_response_compatibility_linking_is_limited_to_v0_1(self):
        text = FIXTURE.read_text(encoding="utf-8")
        text = text.replace("script_version: 0.1.0", "script_version: 0.2.0")
        text = text.replace("test_count: 113", "test_count: 2")
        with tempfile.TemporaryDirectory() as temporary_directory:
            path = Path(temporary_directory) / "current.log"
            path.write_text(text, encoding="utf-8")

            parsed = parse_log(path)

        self.assertEqual(parsed["tests"]["002"]["requestRefs"], [])


if __name__ == "__main__":
    unittest.main()
