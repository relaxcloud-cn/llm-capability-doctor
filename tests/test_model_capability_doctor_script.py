import json
import os
import re
import signal
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "model-capability-doctor.sh"
FAKE_CURL = ROOT / "tests" / "helpers" / "fake_model_curl.py"

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


def delimited_block(log: str, kind: str, identifier: str) -> str:
    label = f"{kind} {identifier}" if identifier else kind
    match = re.search(
        rf"^========== {re.escape(label)} BEGIN ==========$(.*?)"
        rf"^========== {re.escape(label)} END ==========$",
        log,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing {kind.lower()} block {identifier}")
    return match.group(1)


def manifest_request_refs(log: str, test_id: str) -> list[str]:
    block = delimited_block(log, f"TEST-{test_id}", "")
    match = re.search(r"^request_refs: ?(.*)$", block, flags=re.MULTILINE)
    if not match or not match.group(1):
        return []
    return match.group(1).split(",")


def request_body(log: str, request_id: str) -> str:
    block = delimited_block(log, "REQUEST", request_id)
    match = re.search(
        r"^----- REQUEST BODY BEGIN -----$(.*?)"
        r"^----- REQUEST BODY END -----$",
        block,
        flags=re.MULTILINE | re.DOTALL,
    )
    if not match:
        raise AssertionError(f"Missing request body for {request_id}")
    return match.group(1).strip()


class ModelCapabilityDoctorScriptTests(unittest.TestCase):
    def run_script(self, *arguments):
        return subprocess.run(
            ["bash", str(SCRIPT), *arguments],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )

    def run_fixture(self, scenario, only, *, api_key="fixture-key"):
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
            return result, log_path.read_text(encoding="utf-8")

    def test_collector_writes_evidence_schema_without_judgments(self):
        result, log = self.run_fixture("basic", "004")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("script_version: 0.7.0", log)
        self.assertIn("log_schema: llm-capability-doctor.evidence.v1", log)
        self.assertIn("request_refs: test-004", log)
        for forbidden in FORBIDDEN_JUDGMENT_FIELDS:
            self.assertNotIn(forbidden, log.lower())

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
            "semantic_success",
            "visible_answer_extractable",
            "extract_visible_text",
            "is_explicit_guardrail_response",
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

    def test_value_options_reject_missing_values_without_hanging(self):
        for option in (
            "--url",
            "--model",
            "--api-key",
            "--log-file",
            "--timeout",
            "--only",
        ):
            with self.subTest(option=option):
                result = subprocess.run(
                    ["bash", str(SCRIPT), option],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    check=False,
                    timeout=2,
                )
                self.assertEqual(result.returncode, 2)
                self.assertIn(f"Missing value for {option}", result.stderr)

    def test_term_signal_exits_143_and_preserves_written_log_header(self):
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            fake_curl = directory / "curl"
            log_path = directory / "doctor.log"
            shutil.copy2(FAKE_CURL, fake_curl)
            fake_curl.chmod(0o755)
            environment = dict(os.environ)
            environment["PATH"] = f"{directory}:{environment['PATH']}"
            environment["MODEL_DOCTOR_FAKE_DELAY"] = "30"
            process = subprocess.Popen(
                [
                    "bash",
                    str(SCRIPT),
                    "--url",
                    "https://model.example/v1/chat/completions",
                    "--model",
                    "fixture-model",
                    "--api-key",
                    "fixture-key",
                    "--only",
                    "004",
                    "--log-file",
                    str(log_path),
                ],
                cwd=ROOT,
                env=environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline:
                    if log_path.exists() and "log_schema:" in log_path.read_text(
                        encoding="utf-8"
                    ):
                        break
                    time.sleep(0.02)
                self.assertTrue(log_path.exists(), "collector did not create its log")
                os.killpg(process.pid, signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=5)
            finally:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()

            self.assertEqual(process.returncode, 143, stdout + stderr)
            self.assertIn(
                "log_schema: llm-capability-doctor.evidence.v1",
                log_path.read_text(encoding="utf-8"),
            )

    def test_request_audit_keeps_complete_curl_input_and_output(self):
        result, log = self.run_fixture("basic", "004")

        self.assertEqual(result.returncode, 0, result.stderr)
        block = delimited_block(log, "REQUEST", "test-004")
        for marker in (
            "----- CURL COMMAND BEGIN -----",
            "----- REQUEST BODY BEGIN -----",
            "----- RESPONSE METRICS BEGIN -----",
            "----- RESPONSE HEADERS BEGIN -----",
            "----- CURL STDERR BEGIN -----",
            "----- RESPONSE BODY BEGIN -----",
        ):
            self.assertIn(marker, block)
        self.assertIn("MODEL_DOCTOR_CASE_004_OK", request_body(log, "test-004"))
        self.assertIn('"content":"MODEL_DOCTOR_CASE_004_OK"', block)

    def test_anthropic_requests_share_the_2048_output_budget(self):
        result, log = self.run_fixture(
            "anthropic_output_budget",
            "004,031,032,040",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        for request_id in (
            "protocol-3",
            "test-004",
            "test-031",
            "test-032",
            "test-040",
        ):
            body = json.loads(request_body(log, request_id))
            self.assertEqual(body["max_tokens"], 2048, request_id)
        thinking_body = json.loads(request_body(log, "test-032"))
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

    def test_prompt_contracts_are_recorded_without_shell_interpretation(self):
        result, log = self.run_fixture(
            "malformed_visible_answer",
            "029,035,038,060",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("<first-marker>;<second-marker>;<prefix>-<suffix>", log)
        self.assertIn("Compute 19 + 23 internally", log)
        temporal_prompt = json.loads(request_body(log, "test-038"))["messages"][0][
            "content"
        ]
        self.assertIn('"bTime"', temporal_prompt)
        self.assertIn("credential-attack", log)
        for test_id in ("029", "035", "038", "060"):
            self.assertEqual(len(manifest_request_refs(log, test_id)), 1)
        self.assertNotIn("result:", log)

    def test_thinking_effort_comparison_always_collects_both_requests(self):
        result, log = self.run_fixture("basic", "033")

        self.assertEqual(result.returncode, 0, result.stderr)
        refs = manifest_request_refs(log, "033")
        self.assertEqual(refs, ["test-033-low", "test-033-high"])
        self.assertIn('"reasoning_effort":"low"', request_body(log, refs[0]))
        self.assertIn('"reasoning_effort":"high"', request_body(log, refs[1]))

    def test_performance_manifests_reference_raw_requests_without_aggregates(self):
        result, log = self.run_fixture(
            "concurrency_ladder_partial",
            "051,052,053,054,055,056,057,058",
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(manifest_request_refs(log, "055")), 5)
        self.assertEqual(
            manifest_request_refs(log, "055"),
            manifest_request_refs(log, "056"),
        )
        self.assertEqual(len(manifest_request_refs(log, "057")), 60)
        self.assertEqual(len(manifest_request_refs(log, "058")), 11)
        for forbidden in (
            "semantic_success=",
            "p50_ms=",
            "p95_ms=",
            "recovery=PASS",
        ):
            self.assertNotIn(forbidden, log)

    def test_guardrail_responses_are_recorded_without_shell_interpretation(self):
        result, log = self.run_fixture("defensive_echo", "059,060,061,062")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(manifest_request_refs(log, "059")), 1)
        self.assertEqual(len(manifest_request_refs(log, "060")), 1)
        self.assertEqual(len(manifest_request_refs(log, "061")), 2)
        self.assertEqual(len(manifest_request_refs(log, "062")), 2)
        self.assertNotIn("越权破坏请求被明确拦截", log)
        self.assertNotIn("疑似过度拒答", log)

    def test_help_declares_current_version_catalog_size_and_default_timeout(self):
        result = self.run_script("--help")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Model Capability Doctor 0.7.0", result.stdout)
        self.assertIn("Defaults to 120", result.stdout)
        self.assertIn("62-item core catalog", result.stdout)

    def test_catalog_is_contiguous_and_keeps_all_capability_domains(self):
        result = self.run_script("--list-tests")

        self.assertEqual(result.returncode, 0, result.stderr)
        rows = [line.split("\t") for line in result.stdout.splitlines()]
        self.assertEqual(
            [row[0] for row in rows],
            [f"{value:03d}" for value in range(1, 63)],
        )
        categories = {row[1] for row in rows}
        self.assertEqual(
            categories,
            {
                "接口与协议",
                "结构化结果",
                "上下文",
                "指令与文本",
                "Thinking 与推理",
                "工具调用",
                "性能与稳定性",
                "护栏与词汇",
            },
        )

    def test_run_summary_contains_collection_facts_only(self):
        result, log = self.run_fixture("basic", "001,002")

        self.assertEqual(result.returncode, 0, result.stderr)
        summary = log.split("========== RUN SUMMARY ==========", 1)[1]
        self.assertRegex(summary, r"(?m)^request_count: \d+$")
        self.assertIn("test_manifest_count: 2", summary)
        for forbidden in FORBIDDEN_JUDGMENT_FIELDS:
            self.assertNotIn(forbidden, summary.lower())
        self.assertNotIn("检测结果：", result.stdout)
        self.assertNotIn("检测结论：", result.stdout)


if __name__ == "__main__":
    unittest.main()
