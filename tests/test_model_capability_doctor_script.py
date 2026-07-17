import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "model-capability-doctor.sh"
FAKE_CURL = ROOT / "tests" / "helpers" / "fake_model_curl.py"
REPORT_SCRIPT_DIR = ROOT / "skills" / "creating-model-doctor-reports" / "scripts"
sys.path.insert(0, str(REPORT_SCRIPT_DIR))

from model_doctor_log import parse_log  # noqa: E402


FORBIDDEN_JUDGMENT_FIELDS = (
    "result:",
    "expected:",
    "detected:",
    "conclusion:",
    "pass:",
    "fail:",
    "unsupported:",
    "undetermined:",
    "skipped:",
    "error:",
)


def manifest_request_refs(log: str, test_id: str) -> list[str]:
    match = re.search(
        rf"^========== TEST-{test_id} BEGIN ==========$(.*?)"
        rf"^========== TEST-{test_id} END ==========$",
        log,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing test manifest {test_id}")
    refs = re.search(r"^request_refs: ?(.*)$", match.group(1), flags=re.MULTILINE)
    if not refs or not refs.group(1):
        return []
    return refs.group(1).split(",")


class ModelCapabilityDoctorScriptTests(unittest.TestCase):
    def run_script(self, *arguments):
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def run_fixture(self, scenario, only, *, include_parsed=False, api_key="fixture-key"):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            fake_curl = directory / "curl"
            log_path = directory / "doctor.log"
            shutil.copy2(FAKE_CURL, fake_curl)
            fake_curl.chmod(0o755)
            environment = dict(os.environ)
            environment["PATH"] = f"{directory}:{environment['PATH']}"
            environment["MODEL_DOCTOR_FAKE_SCENARIO"] = scenario
            result = subprocess.run(
                [
                    "bash",
                    str(SCRIPT),
                    "--url",
                    "https://model.example/v1/chat/completions",
                    "--model",
                    "fixture-model",
                    "--api-key",
                    api_key,
                    "--only",
                    only,
                    "--log-file",
                    str(log_path),
                ],
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            log = log_path.read_text(encoding="utf-8")
            if include_parsed:
                return result, log, parse_log(log_path)
            return result, log

    def test_collector_writes_evidence_schema_without_judgments(self):
        result, log = self.run_fixture("basic", "004")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("script_version: 0.7.0", log)
        self.assertIn("log_schema: llm-capability-doctor.evidence.v1", log)
        self.assertIn("request_refs: test-004", log)
        for forbidden in FORBIDDEN_JUDGMENT_FIELDS:
            self.assertNotIn(forbidden, log)

    def test_shell_source_has_no_test_grader(self):
        source = SCRIPT.read_text(encoding="utf-8")

        for forbidden in (
            "record_test()",
            "PASS_COUNT",
            "FAIL_COUNT",
            "UNSUPPORTED_COUNT",
            "UNDETERMINED_COUNT",
            "SKIPPED_COUNT",
            "ERROR_COUNT",
        ):
            self.assertNotIn(forbidden, source)

    def test_shared_probe_and_repeat_requests_are_explicitly_referenced(self):
        _, log = self.run_fixture("basic", "002,003,007,055,056")

        protocol_refs = manifest_request_refs(log, "002")
        auth_refs = manifest_request_refs(log, "003")
        usage_refs = manifest_request_refs(log, "007")
        repeat_refs = manifest_request_refs(log, "055")
        percentile_refs = manifest_request_refs(log, "056")
        self.assertTrue(protocol_refs)
        self.assertEqual(auth_refs, usage_refs)
        self.assertEqual(repeat_refs, percentile_refs)
        self.assertEqual(len(percentile_refs), 5)

    def test_fake_curl_fixture_produces_protocol_compatible_audit(self):
        result, log = self.run_fixture("basic", "004")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("result: PASS", log)
        self.assertIn("REQUEST test-004 BEGIN", log)
        self.assertIn("MODEL_DOCTOR_CASE_004_OK", log)

    def test_anthropic_requests_share_the_2048_output_budget(self):
        result, _, parsed = self.run_fixture(
            "anthropic_output_budget",
            "004,031,032,040",
            include_parsed=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        for request_id in (
            "protocol-3",
            "test-004",
            "test-031",
            "test-032",
            "test-040",
        ):
            body = json.loads(parsed["requests"][request_id]["requestBody"])
            self.assertEqual(body["max_tokens"], 2048, request_id)
        thinking_body = json.loads(parsed["requests"]["test-032"]["requestBody"])
        self.assertEqual(thinking_body["thinking"]["budget_tokens"], 1024)

    def test_run_header_masks_api_key_without_logging_the_complete_value(self):
        result, log = self.run_fixture("basic", "004")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("api_key: fixt********-key", log)
        self.assertNotIn("fixture-key", log)

    def test_short_api_key_is_fully_masked(self):
        result, log = self.run_fixture("basic", "004", api_key="short")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("api_key: [MASKED]", log)
        self.assertNotIn("api_key: short", log)

    def test_029_requires_the_explicit_cross_segment_contract(self):
        result, log = self.run_fixture("cross_segment_exact", "029")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("result: PASS", log)
        self.assertIn("CTX_029_A;CTX_029_B;ALPHA-GAMMA", log)
        self.assertIn("<first-marker>;<second-marker>;<prefix>-<suffix>", log)
        self.assertNotIn("Return CTX_029_A, CTX_029_B", log)

        _, mismatch_log = self.run_fixture("cross_segment_plain_join", "029")
        self.assertIn("result: FAIL", mismatch_log)

    def test_038_rejects_the_old_contradictory_response_and_accepts_exact_json(self):
        _, old_log = self.run_fixture("temporal_old_inconsistent", "038")
        self.assertIn("result: FAIL", old_log)

        _, exact_log = self.run_fixture("temporal_exact", "038")
        self.assertIn("result: PASS", exact_log)
        self.assertIn("09:27", exact_log)

    def test_060_requires_substantive_defensive_analysis(self):
        _, echo_log = self.run_fixture("defensive_echo", "060")
        self.assertIn("result: FAIL", echo_log)

        _, analysis_log = self.run_fixture("defensive_exact", "060")
        self.assertIn("result: PASS", analysis_log)
        self.assertIn("credential-attack", analysis_log)
        self.assertIn("followed by a successful login", analysis_log)
        self.assertNotIn("classification must be credential-attack", analysis_log)

    def test_035_requires_separate_reasoning_metadata_and_exact_visible_answer(self):
        _, exact_log = self.run_fixture("thinking_separation_exact", "035")
        self.assertIn("result: PASS", exact_log)
        self.assertIn("Compute 19 + 23 internally", exact_log)

        _, no_signal_log = self.run_fixture("thinking_separation_no_signal", "035")
        self.assertIn("result: FAIL", no_signal_log)

        _, empty_container_log = self.run_fixture("thinking_separation_empty_container", "035")
        self.assertIn("result: FAIL", empty_container_log)

        _, empty_summary_log = self.run_fixture("thinking_separation_empty_summary", "035")
        self.assertIn("result: FAIL", empty_summary_log)

    def test_036_distinguishes_reasoning_events_from_final_usage(self):
        _, exact_log = self.run_fixture("thinking_stream_exact", "036")
        self.assertIn("result: PASS", exact_log)

        _, no_reasoning_log = self.run_fixture("thinking_stream_no_reasoning", "036")
        self.assertIn("result: FAIL", no_reasoning_log)

    def test_036_treats_an_incomplete_stream_as_undetermined(self):
        _, truncated_log = self.run_fixture("thinking_stream_truncated", "036")
        self.assertIn("result: UNDETERMINED", truncated_log)
        self.assertIn('"reasoning_content"', truncated_log)
        self.assertIn("MODEL_DOCTOR_CASE_036_OK", truncated_log)
        self.assertNotIn("data: [DONE]", truncated_log)

        _, done_only_log = self.run_fixture("thinking_stream_done_only", "036")
        self.assertIn("result: UNDETERMINED", done_only_log)

        _, length_log = self.run_fixture("thinking_stream_length", "036")
        self.assertIn("result: FAIL", length_log)

    def test_036_classifies_explicit_parameter_rejection_as_unsupported(self):
        _, rejected_log = self.run_fixture("thinking_stream_rejected", "036")
        self.assertIn("http_status: 400", rejected_log)
        self.assertIn("result: UNSUPPORTED", rejected_log)

    def test_053_reports_streaming_ttfb_without_calling_it_first_token_latency(self):
        _, exact_log = self.run_fixture("performance_stream_exact", "053")
        self.assertIn("name: 流式首字节时间", exact_log)
        self.assertIn("result: PASS", exact_log)
        self.assertIn("该指标是 TTFB，不是首 Token 时间", exact_log)
        self.assertNotIn("首 Token 近似时间", exact_log)

        _, truncated_log = self.run_fixture("performance_stream_truncated", "053")
        self.assertIn("result: FAIL", truncated_log)

        _, done_only_log = self.run_fixture("performance_stream_done_only", "053")
        self.assertIn("result: FAIL", done_only_log)

        _, length_log = self.run_fixture("performance_stream_length", "053")
        self.assertIn("result: FAIL", length_log)

        _, zero_ttfb_log = self.run_fixture("performance_stream_zero_ttfb", "053")
        self.assertIn("result: UNDETERMINED", zero_ttfb_log)

    def test_057_reports_all_four_concurrency_latency_waves(self):
        result, log, parsed = self.run_fixture(
            "concurrency_ladder_exact",
            "057",
            include_parsed=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        request_starts = re.findall(
            r"^========== REQUEST test-057-c(?:4|8|16|32)-[0-9]+ BEGIN ==========$",
            log,
            flags=re.MULTILINE,
        )
        self.assertEqual(len(request_starts), 60)
        self.assertEqual(len(parsed["tests"]["057"]["requestRefs"]), 60)
        self.assertEqual(parsed["tests"]["057"]["duration_ms"], "not_available")
        self.assertEqual(parsed["tests"]["057"]["http_status"], "multiple")
        self.assertEqual(parsed["tests"]["057"]["curl_exit_code"], "multiple")
        self.assertIn("result: PASS", log)
        self.assertIn(
            "detected: "
            "c4:success=4/4,p50_ms=2,p95_ms=4,max_ms=4,rate_limited=0;"
            "c8:success=8/8,p50_ms=4,p95_ms=8,max_ms=8,rate_limited=0;"
            "c16:success=16/16,p50_ms=8,p95_ms=16,max_ms=16,rate_limited=0;"
            "c32:success=32/32,p50_ms=16,p95_ms=31,max_ms=32,rate_limited=0",
            log,
        )
        self.assertIn("全部 60 个请求语义成功", log)
        self.assertIn("request_count: 61", log)

    def test_057_rejects_bad_samples_and_continues_through_c32(self):
        result, log, parsed = self.run_fixture(
            "concurrency_ladder_partial",
            "057",
            include_parsed=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("result: FAIL", log)
        self.assertIn(
            "c4:success=0/4,p50_ms=not_available,p95_ms=not_available,"
            "max_ms=not_available,rate_limited=0",
            log,
        )
        self.assertIn(
            "c8:success=7/8,p50_ms=5,p95_ms=8,max_ms=8,rate_limited=1",
            log,
        )
        self.assertIn(
            "c16:success=15/16,p50_ms=9,p95_ms=16,max_ms=16,rate_limited=0",
            log,
        )
        self.assertIn("c32:success=32/32", log)
        self.assertIn("semantic_success=0", log)
        self.assertIn("未达到 60/60 语义成功", log)
        self.assertNotIn("不可用ms", log)
        self.assertEqual(len(parsed["tests"]["057"]["requestRefs"]), 60)

    def test_057_rejects_markers_inside_malformed_response_envelopes(self):
        _, log = self.run_fixture("concurrency_ladder_malformed_envelope", "057")

        self.assertIn("result: FAIL", log)
        self.assertIn(
            "c4:success=0/4,p50_ms=not_available,p95_ms=not_available,"
            "max_ms=not_available,rate_limited=0",
            log,
        )
        self.assertIn("c32:success=0/32", log)

    def test_057_prioritizes_protocol_probe_transport_errors(self):
        _, log = self.run_fixture("transport_failure", "057")

        self.assertIn("result: ERROR", log)
        self.assertIn("curl", log)

    def test_058_runs_ten_load_requests_and_one_distinct_recovery_probe(self):
        _, exact_log = self.run_fixture("sustained_recovery_exact", "058")
        load_requests = re.findall(
            r"^========== REQUEST test-058-repeat-\d+ BEGIN ==========$",
            exact_log,
            flags=re.MULTILINE,
        )
        recovery_requests = re.findall(
            r"^========== REQUEST test-058-recovery BEGIN ==========$",
            exact_log,
            flags=re.MULTILINE,
        )

        self.assertEqual(len(load_requests), 10)
        self.assertEqual(len(recovery_requests), 1)
        self.assertIn("MODEL_DOCTOR_CASE_058_RECOVERY_OK", exact_log)
        self.assertIn("result: PASS", exact_log)
        self.assertIn("recovery=PASS", exact_log)

    def test_058_fails_when_the_post_load_recovery_probe_fails(self):
        _, recovery_failure_log = self.run_fixture("sustained_recovery_missing", "058")
        self.assertIn("result: FAIL", recovery_failure_log)
        self.assertIn("recovery=FAIL", recovery_failure_log)

        _, recovery_absent_log = self.run_fixture("sustained_recovery_absent", "058")
        self.assertIn("result: ERROR", recovery_absent_log)

    def test_exact_graders_keep_unextractable_evidence_separate_from_wrong_answers(self):
        for scenario in (
            "unextractable_visible_answer",
            "malformed_visible_answer",
            "invalid_balanced_visible_answer",
        ):
            for test_id in ("029", "035", "038", "060"):
                with self.subTest(scenario=scenario, test_id=test_id):
                    _, log = self.run_fixture(scenario, test_id)
                    self.assertIn("result: UNDETERMINED", log)

    def test_transport_failure_takes_priority_over_unknown_protocol(self):
        _, log = self.run_fixture("transport_failure", "038")
        self.assertIn("result: ERROR", log)

    def test_help_declares_version_catalog_size_and_default_timeout(self):
        source = SCRIPT.read_text(encoding="utf-8")
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('SCRIPT_VERSION="0.6.1"', source)
        self.assertIn("Model Capability Doctor 0.6.1", result.stdout)
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
        self.assertEqual(names["053"], "流式首字节时间")
        self.assertEqual(names["057"], "4-32 并发响应时间")
        self.assertEqual(names["058"], "持续请求与恢复探针")
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

        self.assertIn('[[ "$(cat "$visible_file" | trim_text)" == "$expected" ]]', source)
        self.assertNotIn("grep -Fq 'ctx_029_a'", source)
        self.assertNotIn("grep -Fq 'ctx_029_b'", source)
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
            self.assertIn("========== TEST-014 BEGIN ==========", log_text)
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
