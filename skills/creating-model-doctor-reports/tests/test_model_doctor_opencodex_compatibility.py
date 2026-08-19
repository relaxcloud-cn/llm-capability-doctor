from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import RETAINED_TEST_IDS, parse_log  # noqa: E402
from model_doctor_opencodex_compatibility import (  # noqa: E402
    COMPATIBILITY_PROFILE,
    REQUIRED_TEST_IDS,
    SCOPE_BOUNDARY,
    derive_opencodex_compatibility,
)


SUPPORTED_FAMILIES = (
    "OPENAI_CHAT_COMPLETIONS",
    "OPENAI_RESPONSES",
    "ANTHROPIC_MESSAGES",
    "GEMINI_GENERATE_CONTENT",
)
EXPECTED_FIELDS = {
    "profile",
    "level",
    "label",
    "protocolFamily",
    "requiredTestIds",
    "failedTestIds",
    "statement",
    "scopeBoundary",
}


def _full_log(
    schema: str = "llm-capability-doctor.evidence.v3",
    version: str = "0.11.0",
    profile_line: str = (
        "compatibility_profile: opencodex-2.7.42-data-format\n"
    ),
) -> str:
    manifests = "".join(
        "========== TEST-{0} BEGIN ==========\n"
        "name: fixture-{0}\n"
        "category: fixture\n"
        "request_refs: \n"
        "========== TEST-{0} END ==========\n".format(test_id)
        for test_id in sorted(RETAINED_TEST_IDS)
    )
    return (
        "========== MODEL DOCTOR RUN ==========\n"
        f"script_version: {version}\n"
        "section_encoding: base64\n"
        f"log_schema: {schema}\n"
        f"{profile_line}"
        "selected_test_count: 46\n"
        + manifests
        + "========== RUN SUMMARY ==========\n"
        "request_count: 0\n"
        "test_manifest_count: 46\n"
        "========== END ==========\n"
    )


class EvidenceV3ParserTests(unittest.TestCase):
    def _parse(self, content: str, name: str = "evidence.log") -> dict:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / name
        path.write_text(content, encoding="utf-8")
        return parse_log(path)

    def test_accepts_exact_v3_contract_and_exposes_camel_case_profile(self) -> None:
        parsed = self._parse(_full_log())

        self.assertEqual(
            "opencodex-2.7.42-data-format",
            parsed["run"]["compatibilityProfile"],
        )
        self.assertNotIn("compatibility_profile", parsed["run"])
        self.assertEqual(46, len(parsed["tests"]))

    def test_rejects_missing_or_unknown_v3_profile(self) -> None:
        for name, profile_line in {
            "missing": "",
            "unknown": "compatibility_profile: opencodex-next\n",
        }.items():
            with self.subTest(name=name), self.assertRaisesRegex(
                ValueError,
                "Unsupported or missing compatibility_profile",
            ):
                self._parse(_full_log(profile_line=profile_line), f"{name}.log")

    def test_rejects_mixed_v3_schema_version_tuple(self) -> None:
        with self.assertRaisesRegex(ValueError, "schema/version pair"):
            self._parse(_full_log(version="0.10.0"))

    def test_rejects_incomplete_v3_evidence(self) -> None:
        incomplete = re.sub(
            r"^========== TEST-060 BEGIN ==========\n.*?"
            r"^========== TEST-060 END ==========\n",
            "",
            _full_log()
            .replace("selected_test_count: 46", "selected_test_count: 45")
            .replace("test_manifest_count: 46", "test_manifest_count: 45"),
            count=1,
            flags=re.MULTILINE | re.DOTALL,
        )

        with self.assertRaisesRegex(ValueError, "must contain all 46 retained tests"):
            self._parse(incomplete)

    def test_rejects_legacy_collection_profile_in_v3(self) -> None:
        value = _full_log().replace(
            "compatibility_profile: opencodex-2.7.42-data-format\n",
            "compatibility_profile: opencodex-2.7.42-data-format\n"
            "collection_profile: full\n",
        )

        with self.assertRaisesRegex(
            ValueError,
            "Evidence v3 must not contain collection_profile",
        ):
            self._parse(value)

    def test_continues_to_accept_v1_and_v2_without_compatibility_profile(
        self,
    ) -> None:
        v2 = _full_log(
            schema="llm-capability-doctor.evidence.v2",
            version="0.10.0",
            profile_line="",
        )
        v1 = """========== MODEL DOCTOR RUN ==========
log_schema: llm-capability-doctor.evidence.v1
script_version: 0.9.0
section_encoding: base64
collection_profile: custom
selected_test_count: 1
========== TEST-002 BEGIN ==========
name: protocol
category: fixture
request_refs:
========== TEST-002 END ==========
========== RUN SUMMARY ==========
request_count: 0
test_manifest_count: 1
========== END ==========
"""

        for name, value in {"v1": v1, "v2": v2}.items():
            with self.subTest(name=name):
                parsed = self._parse(value, f"{name}.log")
                self.assertNotIn("compatibilityProfile", parsed["run"])
                self.assertNotIn("compatibility_profile", parsed["run"])

    def test_rejects_compatibility_profile_claims_from_old_contracts(self) -> None:
        v2 = _full_log(
            schema="llm-capability-doctor.evidence.v2",
            version="0.10.0",
        )
        v1 = """========== MODEL DOCTOR RUN ==========
log_schema: llm-capability-doctor.evidence.v1
script_version: 0.9.0
section_encoding: base64
collection_profile: custom
compatibility_profile: opencodex-2.7.42-data-format
selected_test_count: 1
========== TEST-002 BEGIN ==========
name: protocol
category: fixture
request_refs:
========== TEST-002 END ==========
========== RUN SUMMARY ==========
request_count: 0
test_manifest_count: 1
========== END ==========
"""

        for version, value in {"v1": v1, "v2": v2}.items():
            with self.subTest(version=version), self.assertRaisesRegex(
                ValueError,
                f"Evidence {version} must not contain compatibility_profile",
            ):
                self._parse(value, f"{version}-false-profile.log")


class OpenCodexCompatibilityTests(unittest.TestCase):
    def _statuses(self, value: str = "PASS") -> dict[str, str]:
        return {test_id: value for test_id in REQUIRED_TEST_IDS}

    def _derive(
        self,
        statuses: dict[str, str] | None = None,
        family: str = "OPENAI_CHAT_COMPLETIONS",
        run: dict[str, str] | None = None,
    ) -> dict:
        return derive_opencodex_compatibility(
            run
            if run is not None
            else {
                "log_schema": "llm-capability-doctor.evidence.v3",
                "script_version": "0.11.0",
                "compatibilityProfile": COMPATIBILITY_PROFILE,
            },
            statuses if statuses is not None else self._statuses(),
            family,
        )

    def test_all_supported_protocol_families_pass_all_hard_gates(self) -> None:
        for family in SUPPORTED_FAMILIES:
            with self.subTest(family=family):
                result = self._derive(family=family)
                self.assertEqual("PASS", result["level"])
                self.assertEqual("OpenCodex 数据格式兼容", result["label"])
                self.assertEqual([], result["failedTestIds"])
                self.assertEqual(
                    "本轮八项必需数据格式检查全部通过，因此判定符合 "
                    "OpenCodex 2.7.42 数据格式合同。",
                    result["statement"],
                )

    def test_required_failure_fails_and_lists_ids_in_contract_order(self) -> None:
        statuses = self._statuses()
        statuses.update({"047": "FAIL", "004": "FAIL", "040": "FAIL"})

        result = self._derive(statuses)

        self.assertEqual("FAIL", result["level"])
        self.assertEqual("OpenCodex 数据格式不兼容", result["label"])
        self.assertEqual(["004", "040", "047"], result["failedTestIds"])
        self.assertEqual(
            "本轮八项必需数据格式检查有 3 项未通过（004、040、047），"
            "因此判定不符合 OpenCodex 2.7.42 数据格式合同。",
            result["statement"],
        )

    def test_test_045_failure_does_not_affect_compatibility(self) -> None:
        statuses = self._statuses()
        statuses["045"] = "FAIL"

        self.assertEqual("PASS", self._derive(statuses)["level"])

    def test_unsupported_protocol_fails_including_ollama(self) -> None:
        for family in ("OLLAMA_CHAT", "CUSTOM", "UNKNOWN"):
            with self.subTest(family=family):
                result = self._derive(family=family)
                self.assertEqual("FAIL", result["level"])
                self.assertEqual(family, result["protocolFamily"])
                self.assertEqual(
                    f"本轮识别协议 {family} 不属于 OpenCodex 2.7.42 数据格式"
                    "配置支持的四类协议，因此判定不符合该数据格式合同。",
                    result["statement"],
                )

    def test_unsupported_protocol_and_hard_failures_report_both_reasons(
        self,
    ) -> None:
        statuses = self._statuses()
        statuses.update({"047": "FAIL", "002": "FAIL"})

        result = self._derive(statuses, family="OLLAMA_CHAT")

        self.assertEqual(["002", "047"], result["failedTestIds"])
        self.assertEqual(
            "本轮识别协议 OLLAMA_CHAT 不属于 OpenCodex 2.7.42 数据格式配置"
            "支持的四类协议，且八项必需数据格式检查有 2 项未通过（002、047），"
            "因此判定不符合该数据格式合同。",
            result["statement"],
        )

    def test_old_contract_is_not_assessed_without_manufactured_failures(self) -> None:
        for schema in (
            "llm-capability-doctor.evidence.v1",
            "llm-capability-doctor.evidence.v2",
        ):
            with self.subTest(schema=schema):
                result = self._derive(
                    {},
                    run={
                        "log_schema": schema,
                        "compatibilityProfile": COMPATIBILITY_PROFILE,
                    },
                )
                self.assertEqual("NOT_ASSESSED", result["level"])
                self.assertEqual("OpenCodex 数据格式未评定", result["label"])
                self.assertEqual([], result["failedTestIds"])
                self.assertEqual(
                    "本轮日志未按 OpenCodex 2.7.42 数据格式配置采集，"
                    "因此本轮兼容性未评定。",
                    result["statement"],
                )

    def test_output_is_closed_and_inputs_are_not_mutated(self) -> None:
        run = {
            "log_schema": "llm-capability-doctor.evidence.v3",
            "script_version": "0.11.0",
            "compatibilityProfile": COMPATIBILITY_PROFILE,
        }
        statuses = self._statuses()
        original_run = dict(run)
        original_statuses = dict(statuses)

        result = self._derive(statuses, run=run)

        self.assertEqual(EXPECTED_FIELDS, set(result))
        self.assertEqual(COMPATIBILITY_PROFILE, result["profile"])
        self.assertEqual(list(REQUIRED_TEST_IDS), result["requiredTestIds"])
        self.assertEqual(SCOPE_BOUNDARY, result["scopeBoundary"])
        self.assertEqual(original_run, run)
        self.assertEqual(original_statuses, statuses)


if __name__ == "__main__":
    unittest.main()
