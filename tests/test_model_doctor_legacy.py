import sys
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

    def test_legacy_shared_protocol_request_is_linked_by_exact_response(self):
        parsed = parse_log(FIXTURE)

        self.assertEqual(parsed["tests"]["002"]["requestRefs"], ["protocol-1"])
        self.assertEqual(parsed["tests"]["113"]["requestRefs"], [])


if __name__ == "__main__":
    unittest.main()
