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
        self.assertIn('SCRIPT_VERSION="0.4.0"', source)
        self.assertIn("Defaults to 120", result.stdout)
        self.assertIn("62-item core catalog", result.stdout)

    def test_catalog_is_contiguous_and_places_context_capacity_at_014_through_018(self):
        result = self.run_script("--list-tests")

        self.assertEqual(result.returncode, 0, result.stderr)
        rows = [line.split("\t") for line in result.stdout.splitlines()]
        self.assertEqual(
            [row[0] for row in rows],
            [f"{value:03d}" for value in range(1, 63)],
        )
        categories = {row[0]: row[1] for row in rows}
        names = {row[0]: row[2] for row in rows}
        self.assertEqual(
            {
                identifier: (categories[identifier], names[identifier])
                for identifier in ("014", "015", "016", "017", "018")
            },
            {
                "014": ("上下文", "8K 级上下文（字符近似）"),
                "015": ("上下文", "16K 级上下文（字符近似）"),
                "016": ("上下文", "32K 级上下文（字符近似）"),
                "017": ("上下文", "64K 级上下文（字符近似）"),
                "018": ("上下文", "128K 级上下文（字符近似）"),
            },
        )
        self.assertEqual(names["019"], "精确输出")
        self.assertEqual(names["026"], "开头信息召回")
        self.assertEqual(names["032"], "Thinking 参数接受")
        self.assertEqual(names["040"], "单工具调用")
        self.assertEqual(names["051"], "冷请求总延迟")
        self.assertEqual(names["059"], "越权请求护栏")

    def test_context_capacity_handler_defines_all_five_target_sizes(self):
        source = SCRIPT.read_text(encoding="utf-8")

        for expected in (
            "014) echo 32000 ;;",
            "015) echo 64000 ;;",
            "016) echo 128000 ;;",
            "017) echo 256000 ;;",
            "018) echo 512000 ;;",
            "014|015|016|017|018) run_core_context_capacity_test",
        ):
            self.assertIn(expected, source)
        for removed in (
            "long_output_body()",
            "perform_long_output_request()",
            "run_core_long_output_test()",
            "MODEL_DOCTOR_OUTPUT_",
            "LONG_OUTPUT_",
        ):
            self.assertNotIn(removed, source)

    def test_shifted_internal_markers_match_their_new_test_ids(self):
        source = SCRIPT.read_text(encoding="utf-8")

        self.assertIn("grep -Fq 'ctx_029_a'", source)
        self.assertIn("grep -Fq 'ctx_029_b'", source)
        self.assertNotIn("grep -Fq 'ctx_032_a'", source)
        self.assertIn('"test-055-repeat-${index}"', source)
        self.assertNotIn('"test-058-repeat-${index}"', source)

    def test_context_capacity_request_records_characters_and_observed_input_tokens(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            fake_curl = directory / "curl"
            log_path = directory / "capacity.log"
            fake_curl.write_text(
                """#!/usr/bin/env bash
if [[ "${1:-}" == "--version" ]]; then
  echo "curl mock"
  exit 0
fi
output_file=""
headers_file=""
request_body=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output_file="$2"; shift 2 ;;
    --dump-header) headers_file="$2"; shift 2 ;;
    --write-out) shift 2 ;;
    --data-binary) request_body="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [[ "$request_body" == *"CTX_014_OK"* ]]; then
  content="CTX_014_OK"
else
  content="MODEL_DOCTOR_PROTOCOL_OK"
fi
printf '{"object":"chat.completion","choices":[{"message":{"content":"%s"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12000,"completion_tokens":8,"total_tokens":12008}}' "$content" >"$output_file"
printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n' >"$headers_file"
printf '200\t0.010\t0.005\t200'
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
                    "014",
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
            log_text = log_path.read_text(encoding="utf-8")
            self.assertIn("test_id: 014", log_text)
            self.assertIn("request_chars=32000,input_tokens=12000", log_text)
            self.assertIn("CTX_014_OK", log_text)
            self.assertIn("字符负载", log_text)
            self.assertIn("输入 Token", log_text)

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
