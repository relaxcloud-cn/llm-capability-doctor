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
SKILL_ROOT = ROOT / "skills" / "creating-model-doctor-reports"
SCRIPT_DIR = SKILL_ROOT / "scripts"
ASSET_DIR = SKILL_ROOT / "assets"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_assessment import (  # noqa: E402
    CURRENT_CATALOG_GATE_LEVELS,
    assemble_assessment,
    validate_assessment,
)
from model_doctor_html import render_report  # noqa: E402
from model_doctor_log import parse_log  # noqa: E402


SUPPORTED_PROTOCOLS = (
    "openai_chat",
    "openai_responses",
    "anthropic_messages",
    "gemini_generate_content",
    "ollama_chat",
)


def protocol_envelope_matches(protocol, body):
    try:
        payload = json.loads(body)
    except json.JSONDecodeError:
        return False
    if protocol == "openai_chat":
        return "choices" in payload
    if protocol == "openai_responses":
        return payload.get("object") == "response" and "output" in payload
    if protocol == "anthropic_messages":
        return payload.get("type") == "message" and "content" in payload
    if protocol == "gemini_generate_content":
        return "candidates" in payload
    return "message" in payload and "done" in payload


def deterministic_reviews(parsed):
    reviews = {}
    for test_id, test in parsed["tests"].items():
        status = "PASS" if int(test_id) % 2 else "FAIL"
        refs = test["requestRefs"]
        evidence_refs = (
            [f"request:{request_id}" for request_id in refs]
            if refs
            else [f"test:{test_id}:manifest"]
        )
        reviews[test_id] = {
            "testId": test_id,
            "reviewedStatus": status,
            "confidence": "high",
            "gateLevel": CURRENT_CATALOG_GATE_LEVELS[test_id],
            "conclusion": (
                "夹具证据满足检测契约。"
                if status == "PASS"
                else "夹具证据未满足检测契约。"
            ),
            "logic": {
                "purpose": "验证端到端证据流。",
                "method": "检查 manifest 引用的请求和响应。",
                "passCriteria": ["完整证据满足检测契约。"],
                "failCriteria": ["证据缺失或与检测契约冲突。"],
                "capabilityBoundary": "仅验证夹具中的可观察行为。",
            },
            "evidenceRefs": evidence_refs,
            "evidenceExcerpts": ["端到端夹具证据。"],
            "limitations": [],
            "retestInstructions": [],
        }
    return reviews


class ModelDoctorEndToEndTests(unittest.TestCase):
    def run_pipeline(self, protocol):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        fake_curl = directory / "curl"
        log_path = directory / f"{protocol}.log"
        shutil.copy2(FAKE_CURL, fake_curl)
        fake_curl.chmod(0o755)
        environment = dict(os.environ)
        environment["PATH"] = f"{directory}:{environment['PATH']}"
        environment["MODEL_DOCTOR_FAKE_SCENARIO"] = f"{protocol}_tool_chain"
        result = subprocess.run(
            [
                "bash",
                str(SCRIPT),
                "--url",
                "https://model.example/v1/chat/completions",
                "--model",
                "fixture-model",
                "--api-key",
                "fixture-key",
                "--log-file",
                str(log_path),
            ],
            cwd=ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=False,
            timeout=90,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        log = log_path.read_text(encoding="utf-8")
        parsed = parse_log(log_path)
        assessment = assemble_assessment(parsed, deterministic_reviews(parsed))
        self.assertEqual(validate_assessment(assessment), [])
        html = render_report(assessment, ASSET_DIR)
        return log, parsed, assessment, html

    def test_all_protocols_complete_evidence_to_binary_report(self):
        for protocol in SUPPORTED_PROTOCOLS:
            with self.subTest(protocol=protocol):
                log, parsed, assessment, html = self.run_pipeline(protocol)
                self.assertEqual(len(parsed["tests"]), 62)
                self.assertEqual(
                    set(assessment["overall"]["counts"]),
                    {"PASS", "FAIL"},
                )
                self.assertEqual(
                    assessment["schemaVersion"],
                    "llm-capability-doctor.assessment.v3",
                )
                self.assertNotIn("CONDITIONAL", html)
                self.assertNotRegex(
                    log,
                    r"(?m)^(?:result|expected|detected|conclusion|pass|fail|unsupported|undetermined|skipped|error):",
                )

                for item in assessment["tests"]:
                    test_id = item["testId"]
                    expected_refs = parsed["tests"][test_id]["requestRefs"]
                    actual_refs = [
                        request["request_id"] for request in item["requests"]
                    ]
                    self.assertEqual(actual_refs, expected_refs, test_id)
                    for request_id in expected_refs:
                        self.assertIn(request_id, html)

                for request_id, request in parsed["requests"].items():
                    if request_id.startswith("protocol-"):
                        continue
                    self.assertTrue(
                        protocol_envelope_matches(
                            protocol,
                            request["responseBody"],
                        ),
                        request_id,
                    )


if __name__ == "__main__":
    unittest.main()
