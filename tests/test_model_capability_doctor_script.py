import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "model-capability-doctor.sh"


class ModelCapabilityDoctorScriptTests(unittest.TestCase):
    def run_script(self, *arguments):
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def test_help_declares_version_catalog_size_and_default_timeout(self):
        source = SCRIPT.read_text(encoding="utf-8")
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('SCRIPT_VERSION="0.3.0"', source)
        self.assertIn("Defaults to 120", result.stdout)
        self.assertIn("65-item core catalog", result.stdout)

    def test_catalog_is_contiguous_and_places_long_output_at_014_through_018(self):
        result = self.run_script("--list-tests")

        self.assertEqual(result.returncode, 0, result.stderr)
        rows = [line.split("\t") for line in result.stdout.splitlines()]
        self.assertEqual(
            [row[0] for row in rows],
            [f"{value:03d}" for value in range(1, 66)],
        )
        names = {row[0]: row[2] for row in rows}
        self.assertEqual(
            {
                identifier: names[identifier]
                for identifier in ("014", "015", "016", "017", "018")
            },
            {
                "014": "8K 完整 Result JSON",
                "015": "16K 完整 Result JSON",
                "016": "32K 完整 Result JSON",
                "017": "64K 完整 Result JSON",
                "018": "128K 完整 Result JSON",
            },
        )
        self.assertEqual(names["019"], "精确输出")
        self.assertEqual(names["026"], "8K 级上下文（字符近似）")
        self.assertEqual(names["035"], "Thinking 参数接受")
        self.assertEqual(names["043"], "单工具调用")
        self.assertEqual(names["054"], "冷请求总延迟")
        self.assertEqual(names["062"], "越权请求护栏")

    def test_long_output_handler_defines_all_five_target_sizes(self):
        source = SCRIPT.read_text(encoding="utf-8")

        for expected in (
            "local size_k=$((target_tokens / 1024))",
            '014) target_tokens=8192 ;;',
            '015) target_tokens=16384 ;;',
            '016) target_tokens=32768 ;;',
            '017) target_tokens=65536 ;;',
            '018) target_tokens=131072 ;;',
            "local limit_tokens=$((target_tokens + 1024))",
            "014|015|016|017|018) run_core_long_output_test",
        ):
            self.assertIn(expected, source)

    def test_shifted_internal_markers_match_their_new_test_ids(self):
        source = SCRIPT.read_text(encoding="utf-8")

        self.assertIn("grep -Fq 'ctx_032_a'", source)
        self.assertIn("grep -Fq 'ctx_032_b'", source)
        self.assertNotIn("grep -Fq 'ctx_029_a'", source)
        self.assertIn('"test-058-repeat-${index}"', source)
        self.assertNotIn('"test-061-repeat-${index}"', source)


if __name__ == "__main__":
    unittest.main()
