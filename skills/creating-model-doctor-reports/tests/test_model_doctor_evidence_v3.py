from __future__ import annotations

import base64
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Iterable, Mapping


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_log import RETAINED_TEST_IDS, parse_log  # noqa: E402


V3_TEST_IDS = frozenset(RETAINED_TEST_IDS) | {"046"}


def valid_v3_request_metadata() -> dict[str, str]:
    """Return a complete, conformant request-metadata fixture."""

    return {
        "transport_outcome": "completed_eof",
        "stream_termination": "completed",
        "stream_end_signal": "[DONE]",
        "model_stop_reason": "stop",
        "stream_event_count": "3",
        "tool_contract_status": "conformant",
        "tool_contract_errors_json": "[]",
        "tool_loop_turn": "1",
        "tool_loop_outcome": "completed",
    }


def build_evidence_log(
    *,
    log_schema: str = "llm-capability-doctor.evidence.v3",
    script_version: str = "0.11.0",
    test_ids: Iterable[str] = V3_TEST_IDS,
    request_metadata: Mapping[str, str] | None = None,
    collection_profile: str | None = None,
    include_request: bool | None = None,
) -> str:
    """Build a complete evidence log for parser contract tests."""

    selected_ids = sorted(test_ids)
    is_v3 = log_schema == "llm-capability-doctor.evidence.v3"
    if include_request is None:
        include_request = is_v3
    metadata = dict(
        valid_v3_request_metadata() if request_metadata is None and is_v3
        else request_metadata or {}
    )
    request_test_id = "046" if "046" in selected_ids else selected_ids[0]
    request_id = f"test-{request_test_id}-turn-1"
    request = ""
    if include_request:
        metadata_lines = "".join(
            f"{field}: {value}\n" for field, value in metadata.items()
        )
        empty_b64 = base64.b64encode(b"").decode("ascii")
        json_b64 = base64.b64encode(b"{}").decode("ascii")
        request = (
            f"========== REQUEST {request_id} BEGIN ==========\n"
            f"request_id: {request_id}\n"
            "started_at: 2026-08-19T00:00:00+0800\n"
            "completed_at: 2026-08-19T00:00:01+0800\n"
            "protocol: openai_chat\n"
            "auth_mode: bearer\n"
            "stream: 1\n"
            f"{metadata_lines}\n"
            "----- CURL COMMAND BEGIN -----\n"
            "curl https://example.invalid/v1/chat/completions\n"
            "----- CURL COMMAND END -----\n\n"
            "----- REQUEST BODY BEGIN -----\n"
            f"{json_b64}\n"
            "----- REQUEST BODY END -----\n\n"
            "----- RESPONSE METRICS BEGIN -----\n"
            "curl_exit_code: 0\n"
            "http_status: 200\n"
            "time_total: 0.100000\n"
            "time_starttransfer: 0.050000\n"
            "size_download: 2\n"
            "----- RESPONSE METRICS END -----\n\n"
            "----- RESPONSE HEADERS BEGIN -----\n"
            "content-type: text/event-stream\n"
            "----- RESPONSE HEADERS END -----\n\n"
            "----- CURL STDERR BEGIN -----\n"
            f"{empty_b64}\n"
            "----- CURL STDERR END -----\n\n"
            "----- RESPONSE BODY BEGIN -----\n"
            f"{json_b64}\n"
            "----- RESPONSE BODY END -----\n\n"
            f"========== REQUEST {request_id} END ==========\n"
        )

    manifests = "".join(
        "========== TEST-{0} BEGIN ==========\n"
        "name: fixture-{0}\n"
        "category: fixture\n"
        "request_refs: {1}\n"
        "========== TEST-{0} END ==========\n".format(
            test_id,
            request_id if test_id == request_test_id and request else "",
        )
        for test_id in selected_ids
    )
    profile_line = (
        f"collection_profile: {collection_profile}\n"
        if collection_profile is not None
        else ""
    )
    return (
        "========== MODEL DOCTOR RUN ==========\n"
        f"script_version: {script_version}\n"
        "section_encoding: base64\n"
        f"log_schema: {log_schema}\n"
        f"{profile_line}"
        f"selected_test_count: {len(selected_ids)}\n"
        f"{request}"
        f"{manifests}"
        "========== RUN SUMMARY ==========\n"
        f"request_count: {1 if request else 0}\n"
        f"test_manifest_count: {len(selected_ids)}\n"
        "========== END ==========\n"
    )


class ModelDoctorEvidenceV3Tests(unittest.TestCase):
    def parse_text_log(self, value: str, name: str = "fixture.log") -> dict:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / name
        path.write_text(value, encoding="utf-8")
        return parse_log(path)

    def test_parser_accepts_complete_profile_free_v3_with_47_manifests(self) -> None:
        parsed = self.parse_text_log(build_evidence_log())

        self.assertEqual(47, len(parsed["tests"]))
        self.assertNotIn("collection_profile", parsed["run"])
        self.assertEqual(
            "llm-capability-doctor.evidence.v3",
            parsed["run"]["log_schema"],
        )
        with self.assertRaisesRegex(ValueError, "collection_profile"):
            self.parse_text_log(
                build_evidence_log(collection_profile="full"),
                "profile-v3.log",
            )
        count_mutations = {
            "selected": ("selected_test_count: 47", "selected_test_count: 46"),
            "requests": ("request_count: 1", "request_count: 0"),
            "manifests": ("test_manifest_count: 47", "test_manifest_count: 46"),
        }
        for name, (before, after) in count_mutations.items():
            with self.subTest(count=name), self.assertRaisesRegex(
                ValueError,
                "does not match discovered",
            ):
                self.parse_text_log(
                    build_evidence_log().replace(before, after, 1),
                    f"{name}-count-v3.log",
                )

    def test_parser_rejects_v3_missing_046(self) -> None:
        with self.assertRaisesRegex(ValueError, "47"):
            self.parse_text_log(
                build_evidence_log(test_ids=RETAINED_TEST_IDS),
                "missing-046.log",
            )

    def test_parser_rejects_mixed_v3_contract_variants(self) -> None:
        variants = (
            ("llm-capability-doctor.evidence.v3", "0.10.0"),
            ("llm-capability-doctor.evidence.v2", "0.11.0"),
        )
        for schema, version in variants:
            with self.subTest(schema=schema, version=version):
                with self.assertRaisesRegex(ValueError, "schema/version pair"):
                    self.parse_text_log(
                        build_evidence_log(
                            log_schema=schema,
                            script_version=version,
                        ),
                        "mixed.log",
                    )

    def test_parser_validates_v3_request_metadata(self) -> None:
        parsed = self.parse_text_log(build_evidence_log())

        request = parsed["requests"]["test-046-turn-1"]
        self.assertEqual(valid_v3_request_metadata(), {
            field: request[field] for field in valid_v3_request_metadata()
        })

        for field in valid_v3_request_metadata():
            metadata = valid_v3_request_metadata()
            del metadata[field]
            with self.subTest(missing=field), self.assertRaisesRegex(
                ValueError,
                field,
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    f"missing-{field}.log",
                )

        invalid_enums = {
            "transport_outcome": "http_error",
            "stream_termination": "clean",
            "stream_end_signal": "finishReason:",
            "tool_contract_status": "valid",
            "tool_loop_outcome": "stopped",
        }
        for field, value in invalid_enums.items():
            metadata = valid_v3_request_metadata()
            metadata[field] = value
            with self.subTest(invalid_enum=field), self.assertRaisesRegex(
                ValueError,
                field,
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    f"invalid-{field}.log",
                )

        valid_enums = {
            "transport_outcome": (
                "completed_eof",
                "timeout",
                "upstream_disconnect",
                "client_cancelled",
                "transport_error",
            ),
            "stream_termination": (
                "not_applicable",
                "completed",
                "missing_terminal_event",
                "timeout",
                "upstream_disconnect",
                "client_cancelled",
                "transport_error",
                "http_error",
                "malformed_stream",
                "protocol_error",
                "model_incomplete",
            ),
            "stream_end_signal": (
                "none",
                "[DONE]",
                "response.completed",
                "message_stop",
                "finishReason:STOP",
                "done:true",
            ),
            "tool_contract_status": (
                "not_applicable",
                "conformant",
                "non_conformant",
            ),
            "tool_loop_outcome": (
                "not_applicable",
                "continued",
                "completed",
                "invalid_turn",
                "transport_failure",
                "max_turns_exceeded",
            ),
        }
        for field, values in valid_enums.items():
            for value in values:
                metadata = valid_v3_request_metadata()
                metadata[field] = value
                if field == "tool_contract_status" and value == "non_conformant":
                    metadata["tool_contract_errors_json"] = '["fixture.error:/"]'
                with self.subTest(valid_enum=field, value=value):
                    parsed_variant = self.parse_text_log(
                        build_evidence_log(request_metadata=metadata),
                        f"valid-{field}.log",
                    )
                    self.assertEqual(
                        value,
                        parsed_variant["requests"]["test-046-turn-1"][field],
                    )

        for field in ("stream_event_count", "tool_loop_turn"):
            for value in ("", "00", "+1", "-1", " 1", "1 "):
                metadata = valid_v3_request_metadata()
                metadata[field] = value
                with self.subTest(integer=field, value=value), self.assertRaisesRegex(
                    ValueError,
                    field,
                ):
                    self.parse_text_log(
                        build_evidence_log(request_metadata=metadata),
                        f"invalid-{field}.log",
                    )

        metadata = valid_v3_request_metadata()
        metadata["model_stop_reason"] = ""
        with self.assertRaisesRegex(ValueError, "model_stop_reason"):
            self.parse_text_log(
                build_evidence_log(request_metadata=metadata),
                "empty-stop-reason.log",
            )

        invalid_error_arrays = (
            "[",
            "{}",
            "null",
            "1",
            '"error"',
            '[""]',
            "[1]",
            "[null]",
        )
        for value in invalid_error_arrays:
            metadata = valid_v3_request_metadata()
            metadata["tool_contract_errors_json"] = value
            with self.subTest(errors_json=value), self.assertRaisesRegex(
                ValueError,
                "tool_contract_errors_json",
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    "invalid-errors-json.log",
                )

        status_error_mismatches = (
            ("conformant", '["fixture.error:/"]'),
            ("non_conformant", "[]"),
        )
        for status, errors_json in status_error_mismatches:
            metadata = valid_v3_request_metadata()
            metadata["tool_contract_status"] = status
            metadata["tool_contract_errors_json"] = errors_json
            with self.subTest(status=status), self.assertRaisesRegex(
                ValueError,
                "tool_contract",
            ):
                self.parse_text_log(
                    build_evidence_log(request_metadata=metadata),
                    "invalid-status-errors.log",
                )

    def test_parser_accepts_complete_profile_free_v2(self) -> None:
        parsed = self.parse_text_log(
            build_evidence_log(
                log_schema="llm-capability-doctor.evidence.v2",
                script_version="0.10.0",
                test_ids=RETAINED_TEST_IDS,
                include_request=True,
            ),
            "full-v2.log",
        )

        request = next(iter(parsed["requests"].values()))
        self.assertEqual(46, len(parsed["tests"]))
        self.assertTrue(
            valid_v3_request_metadata().keys().isdisjoint(request),
            request,
        )

    def test_v2_contract_remains_46_checks_without_046(self) -> None:
        from model_doctor_contracts import (  # noqa: PLC0415
            CONTRACT_TEST_IDS,
            LEGACY_TEST_IDS,
            V2_CONTRACT,
        )

        self.assertEqual(46, len(CONTRACT_TEST_IDS[V2_CONTRACT]))
        self.assertEqual(LEGACY_TEST_IDS, CONTRACT_TEST_IDS[V2_CONTRACT])
        self.assertNotIn("046", CONTRACT_TEST_IDS[V2_CONTRACT])
        ids_with_046 = (set(RETAINED_TEST_IDS) - {"060"}) | {"046"}
        with self.assertRaisesRegex(ValueError, "046"):
            self.parse_text_log(
                build_evidence_log(
                    log_schema="llm-capability-doctor.evidence.v2",
                    script_version="0.10.0",
                    test_ids=ids_with_046,
                    include_request=False,
                ),
                "v2-with-046.log",
            )

    def test_v1_custom_rejects_046(self) -> None:
        with self.assertRaisesRegex(ValueError, "046"):
            self.parse_text_log(
                build_evidence_log(
                    log_schema="llm-capability-doctor.evidence.v1",
                    script_version="0.9.0",
                    test_ids={"046"},
                    collection_profile="custom",
                    include_request=False,
                ),
                "v1-custom-046.log",
            )

    def test_v1_full_still_requires_legacy_46(self) -> None:
        parsed = self.parse_text_log(
            build_evidence_log(
                log_schema="llm-capability-doctor.evidence.v1",
                script_version="0.9.0",
                test_ids=RETAINED_TEST_IDS,
                collection_profile="full",
                include_request=True,
            ),
            "v1-full.log",
        )
        request = next(iter(parsed["requests"].values()))
        self.assertEqual(46, len(parsed["tests"]))
        self.assertTrue(valid_v3_request_metadata().keys().isdisjoint(request))

        with self.assertRaisesRegex(ValueError, "all 46"):
            self.parse_text_log(
                build_evidence_log(
                    log_schema="llm-capability-doctor.evidence.v1",
                    script_version="0.9.0",
                    test_ids=set(RETAINED_TEST_IDS) - {"060"},
                    collection_profile="full",
                    include_request=False,
                ),
                "v1-incomplete-full.log",
            )

    def test_contract_key_rejects_non_mapping_run_without_crashing(self) -> None:
        from model_doctor_contracts import V3_CONTRACT, contract_key  # noqa: PLC0415

        class UnhashableString(str):
            __hash__ = None

        self.assertEqual(
            V3_CONTRACT,
            contract_key({
                "log_schema": "llm-capability-doctor.evidence.v3",
                "script_version": "0.11.0",
            }),
        )
        malformed_runs = (
            None,
            [],
            "not-a-mapping",
            1,
            {},
            {"log_schema": "llm-capability-doctor.evidence.v3"},
            {
                "log_schema": None,
                "script_version": "0.11.0",
            },
            {
                "log_schema": ["llm-capability-doctor.evidence.v3"],
                "script_version": "0.11.0",
            },
            {
                "log_schema": "llm-capability-doctor.evidence.v3",
                "script_version": {"value": "0.11.0"},
            },
            {
                "log_schema": UnhashableString("llm-capability-doctor.evidence.v3"),
                "script_version": "0.11.0",
            },
            {
                "log_schema": "llm-capability-doctor.evidence.v3",
                "script_version": "0.10.0",
            },
        )
        for run in malformed_runs:
            with self.subTest(run=run):
                try:
                    contract_key(run)
                except ValueError:
                    pass
                except (AttributeError, TypeError) as error:
                    self.fail(f"contract_key crashed for {run!r}: {error}")
                else:
                    self.fail(f"contract_key accepted malformed run: {run!r}")


if __name__ == "__main__":
    unittest.main()
