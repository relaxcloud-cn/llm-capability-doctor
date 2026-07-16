import os
import subprocess
import tempfile
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

    def test_terminal_shows_only_current_test_while_log_keeps_result_details(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            fake_curl = directory / "curl"
            log_path = directory / "doctor.log"
            fake_curl.write_text(
                """#!/usr/bin/env bash
if [[ "${1:-}" == "--version" ]]; then
  echo "curl mock"
  exit 0
fi
output_file=""
headers_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output_file="$2"; shift 2 ;;
    --dump-header) headers_file="$2"; shift 2 ;;
    --write-out) shift 2 ;;
    *) shift ;;
  esac
done
printf '%s' '{"choices":[{"message":{"content":"MODEL_DOCTOR_OK"}}]}' >"$output_file"
printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n' >"$headers_file"
printf '200\t0.010\t0.005\t61'
""",
                encoding="utf-8",
            )
            fake_curl.chmod(0o755)
            environment = dict(os.environ)
            environment["PATH"] = f"{directory}:{environment['PATH']}"

            result = subprocess.run(
                [
                    "bash",
                    str(SCRIPT),
                    "--url",
                    "https://model.example/v1/chat/completions",
                    "--model",
                    "test-model",
                    "--api-key",
                    "test-key",
                    "--only",
                    "001,002",
                    "--log-file",
                    str(log_path),
                ],
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("正在执行检测项 001：URL 可达性", result.stdout)
            self.assertIn("正在执行检测项 002：协议识别", result.stdout)
            self.assertLess(
                result.stdout.index("正在执行检测项 001"),
                result.stdout.index("正在执行检测项 002"),
            )
            self.assertNotIn("检测结果：", result.stdout)
            self.assertNotIn("检测结论：", result.stdout)
            log_text = log_path.read_text(encoding="utf-8")
            self.assertIn("result: PASS", log_text)
            self.assertIn("conclusion: URL 可连接", log_text)
            self.assertIn("conclusion: 识别为 OpenAI Chat Completions 兼容接口", log_text)


if __name__ == "__main__":
    unittest.main()
