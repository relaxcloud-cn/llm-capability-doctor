#!/usr/bin/env python3
"""Parse supported Model Doctor evidence logs into inert, redacted evidence."""

from __future__ import annotations

import base64
import binascii
import hashlib
import json
import re
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Set, Tuple
from urllib.parse import parse_qsl, urlsplit

from model_doctor_contracts import (
    CONTRACT_TEST_IDS,
    V4_CONTRACT,
    V4_TEST_IDS,
    contract_key,
)
from model_doctor_json import JSON_LOAD_ERRORS, strict_json_loads
from model_doctor_opencodex_compatibility import COMPATIBILITY_PROFILE


PARSED_SCHEMA_VERSION = "llm-capability-doctor.parsed-evidence.v1"
SECTION_ENCODING = "base64"
RETAINED_TEST_IDS = V4_TEST_IDS
SECRET_QUERY_KEYS = {
    "api_key",
    "key",
    "token",
    "access_token",
    "client_secret",
    "password",
}
SECRET_JSON_KEYS = SECRET_QUERY_KEYS | {
    "apiKey",
    "clientSecret",
    "authorization",
    "cookie",
    "set-cookie",
}
SECRET_HEADER_NAMES = {
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
}
FORBIDDEN_MANIFEST_FIELDS = {
    "result",
    "expected",
    "detected",
    "conclusion",
    "status",
}

CURL_HEADER_PATTERN = re.compile(
    r"(?P<option>(?:^|\s)(?:-H|--header)(?:\s+|=))"
    r"(?P<quote>['\"])(?P<name>[A-Za-z0-9-]+)"
    r"(?P<separator>:[ \t]*)(?P<value>[^\r\n]*?)(?P=quote)",
    re.IGNORECASE,
)


def _record_secret(secrets: Set[str], name: str, value: str) -> None:
    item = value.strip()
    if not item or item == "[REDACTED]":
        return
    secrets.add(item)
    if item.lower().startswith("bearer ") and len(item) > 7:
        secrets.add(item[7:].strip())
    if name.lower() in {"cookie", "set-cookie"}:
        for part in item.split(";"):
            if "=" in part:
                cookie_value = part.split("=", 1)[1].strip()
                if cookie_value:
                    secrets.add(cookie_value)


def _secret_is_distinctive(value: str) -> bool:
    if len(value) < 8 or value.lower() in {"true", "false", "null", "none"}:
        return False
    if len(value) >= 16:
        return True
    has_alpha = bool(re.search(r"[A-Za-z]", value))
    has_digit = bool(re.search(r"[0-9]", value))
    has_symbol = bool(re.search(r"[^A-Za-z0-9\s]", value))
    return (has_alpha and has_digit) or has_symbol


def _discover_secrets(value: str) -> Set[str]:
    secrets: Set[str] = set()

    for match in re.finditer(r"https?://[^\s'\"]+", value):
        try:
            for key, item in parse_qsl(
                urlsplit(match.group(0)).query,
                keep_blank_values=True,
            ):
                if key.lower() in SECRET_QUERY_KEYS and item:
                    secrets.add(item)
        except ValueError:
            continue

    header_pattern = re.compile(r"(?im)^([A-Za-z0-9-]+):[ \t]*(.+)$")
    for match in header_pattern.finditer(value):
        if match.group(1).lower() in SECRET_HEADER_NAMES:
            _record_secret(secrets, match.group(1), match.group(2))

    for match in CURL_HEADER_PATTERN.finditer(value):
        if match.group("name").lower() in SECRET_HEADER_NAMES:
            _record_secret(
                secrets,
                match.group("name"),
                match.group("value"),
            )

    json_key_pattern = re.compile(
        r'"('
        + "|".join(re.escape(key) for key in sorted(SECRET_JSON_KEYS))
        + r')"\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)"',
        re.IGNORECASE,
    )
    for match in json_key_pattern.finditer(value):
        _record_secret(secrets, match.group(1), match.group(2))

    return secrets


def redact_text(value: str, discovered_secrets: Optional[Set[str]] = None) -> str:
    """Redact credential-bearing fields and echoed copies of discovered values."""

    propagated_secrets = set(discovered_secrets or ())
    secrets = set(propagated_secrets)
    secrets.update(_discover_secrets(value))
    redacted = value

    redacted = re.sub(
        r"(?im)^([A-Za-z0-9-]+):[ \t]*(.+)$",
        lambda match: (
            f"{match.group(1)}: [REDACTED]"
            if match.group(1).lower() in SECRET_HEADER_NAMES
            else match.group(0)
        ),
        redacted,
    )
    redacted = CURL_HEADER_PATTERN.sub(
        lambda match: (
            f'{match.group("option")}{match.group("quote")}'
            f'{match.group("name")}{match.group("separator")}[REDACTED]'
            f'{match.group("quote")}'
            if match.group("name").lower() in SECRET_HEADER_NAMES
            else match.group(0)
        ),
        redacted,
    )
    redacted = re.sub(
        r'(?i)("(?:api_key|apiKey|access_token|token|client_secret|clientSecret|password|authorization|cookie|set-cookie)"\s*:\s*)"(?:[^"\\]|\\.)*"',
        r'\1"[REDACTED]"',
        redacted,
    )
    redacted = re.sub(
        r"(?i)([?&](?:api_key|key|token|access_token|client_secret|password)=)[^&\s'\"]*",
        r"\1[REDACTED]",
        redacted,
    )

    for secret in sorted(secrets, key=len, reverse=True):
        if secret != "[REDACTED]" and _secret_is_distinctive(secret):
            redacted = redacted.replace(secret, "[REDACTED]")
    return redacted


def _credential_is_masked(value: str) -> bool:
    return value in {"[MASKED]", "[REDACTED]"} or bool(
        re.fullmatch(r"\*{4,}", value)
    )


def _key_values(value: str) -> Dict[str, str]:
    result: Dict[str, str] = {}
    for line in value.splitlines():
        if not line or line.startswith("-----") or line.startswith("=========="):
            continue
        if ": " in line:
            key, item = line.split(": ", 1)
        elif line.endswith(":"):
            key, item = line[:-1], ""
        else:
            continue
        if re.fullmatch(r"[A-Za-z0-9_]+", key):
            result[key] = item
    return result


def _section(block: str, name: str) -> str:
    pattern = re.compile(
        rf"^----- {re.escape(name)} BEGIN -----\n(.*?)"
        rf"^----- {re.escape(name)} END -----$",
        re.MULTILINE | re.DOTALL,
    )
    match = pattern.search(block)
    if not match:
        raise ValueError(f"Missing or malformed section {name}")
    return match.group(1).rstrip("\n")


def _encoded_section(block: str, name: str) -> str:
    encoded = _section(block, name)
    try:
        raw = base64.b64decode(encoded, validate=True)
        return raw.decode("utf-8")
    except (binascii.Error, UnicodeDecodeError) as error:
        raise ValueError(f"Invalid base64 section {name}") from error


def _blocks(text: str, kind: str) -> List[Tuple[str, str]]:
    if kind == "REQUEST":
        start = re.compile(r"^========== REQUEST (.+) BEGIN ==========$", re.MULTILINE)
        end_template = "========== REQUEST {identifier} END =========="
    else:
        start = re.compile(r"^========== TEST-([0-9]+) BEGIN ==========$", re.MULTILINE)
        end_template = "========== TEST-{identifier} END =========="

    blocks: List[Tuple[str, str]] = []
    for match in start.finditer(text):
        identifier = match.group(1)
        end_marker = end_template.format(identifier=identifier)
        end_index = text.find(end_marker, match.end())
        if end_index < 0:
            raise ValueError(f"Unterminated {kind.lower()} block: {identifier}")
        body_start = match.end()
        if text[body_start : body_start + 1] == "\n":
            body_start += 1
        blocks.append((identifier, text[body_start:end_index].rstrip("\n")))
    return blocks


def _run_header(text: str) -> Dict[str, str]:
    marker = "========== MODEL DOCTOR RUN =========="
    if not text.startswith(marker):
        return {}
    after = text[len(marker) :].lstrip("\n")
    next_block = after.find("==========")
    return _key_values(after if next_block < 0 else after[:next_block])


def _run_summary(text: str) -> Dict[str, str]:
    marker = "========== RUN SUMMARY =========="
    start = text.find(marker)
    if start < 0:
        return {}
    body = text[start + len(marker) :]
    end = body.find("========== END ==========")
    if end < 0:
        raise ValueError("Unterminated RUN SUMMARY")
    return _key_values(body[:end])


def _sum_usage(response_bodies: Iterable[str]) -> Dict[str, int]:
    totals = {"input": 0, "output": 0, "reasoning": 0, "total": 0}
    patterns = {
        "input": r'"(?:prompt_tokens|input_tokens)"\s*:\s*([0-9]+)',
        "output": r'"(?:completion_tokens|output_tokens)"\s*:\s*([0-9]+)',
        "reasoning": r'"reasoning_tokens"\s*:\s*([0-9]+)',
        "total": r'"total_tokens"\s*:\s*([0-9]+)',
    }
    for body in response_bodies:
        for key, pattern in patterns.items():
            matches = re.findall(pattern, body)
            if matches:
                totals[key] += int(matches[-1])
    return totals


def _required_count(values: Dict[str, str], field: str) -> int:
    raw = values.get(field)
    if raw is None or not re.fullmatch(r"[0-9]+", raw):
        raise ValueError(f"Missing or invalid count field {field}: {raw!r}")
    return int(raw)


def _validate_count(
    values: Dict[str, str],
    field: str,
    discovered: int,
) -> None:
    declared = _required_count(values, field)
    if declared != discovered:
        raise ValueError(
            f"Declared count {field}={declared} does not match discovered {discovered}"
        )


def _parse_request_refs(value: str, test_id: str) -> List[str]:
    if value == "":
        return []
    refs = [item.strip() for item in value.split(",")]
    if any(not item for item in refs):
        raise ValueError(f"Invalid request_refs for TEST-{test_id}: {value!r}")
    if len(set(refs)) != len(refs):
        raise ValueError(f"Duplicate request_refs for TEST-{test_id}: {value!r}")
    return refs


V4_REQUEST_FIELDS = (
    "transport_outcome",
    "stream_termination",
    "stream_end_signal",
    "model_stop_reason",
    "stream_event_count",
    "tool_contract_status",
    "tool_contract_errors_json",
    "tool_loop_turn",
    "tool_loop_outcome",
)
TRANSPORT_OUTCOMES = frozenset({
    "completed_eof",
    "protocol_terminated",
    "timeout",
    "upstream_disconnect",
    "client_cancelled",
    "transport_error",
})
STREAM_TERMINATIONS = frozenset({
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
})
STREAM_END_SIGNALS = frozenset({
    "none",
    "[DONE]",
    "response.completed",
    "message_stop",
    "done:true",
})
TOOL_CONTRACT_STATUSES = frozenset({
    "not_applicable",
    "conformant",
    "non_conformant",
})
TOOL_LOOP_OUTCOMES = frozenset({
    "not_applicable",
    "continued",
    "completed",
    "invalid_turn",
    "transport_failure",
    "max_turns_exceeded",
})
CANONICAL_NON_NEGATIVE_INTEGER = re.compile(r"^(0|[1-9][0-9]*)$")


def _validate_v4_request_metadata(
    metadata: Dict[str, str],
    request_id: str,
) -> None:
    for field in V4_REQUEST_FIELDS:
        if field not in metadata or not isinstance(metadata[field], str):
            raise ValueError(
                f"Request {request_id} is missing v4 metadata field {field}"
            )

    enum_fields = {
        "transport_outcome": TRANSPORT_OUTCOMES,
        "stream_termination": STREAM_TERMINATIONS,
        "tool_contract_status": TOOL_CONTRACT_STATUSES,
        "tool_loop_outcome": TOOL_LOOP_OUTCOMES,
    }
    for field, approved in enum_fields.items():
        if metadata[field] not in approved:
            raise ValueError(
                f"Request {request_id} has invalid {field}: {metadata[field]!r}"
            )

    end_signal = metadata["stream_end_signal"]
    if end_signal not in STREAM_END_SIGNALS and not (
        end_signal.startswith("finishReason:")
        and end_signal.removeprefix("finishReason:").strip()
    ):
        raise ValueError(
            f"Request {request_id} has invalid stream_end_signal: {end_signal!r}"
        )

    if not metadata["model_stop_reason"].strip():
        raise ValueError(
            f"Request {request_id} has invalid model_stop_reason: "
            f"{metadata['model_stop_reason']!r}"
        )

    for field in ("stream_event_count", "tool_loop_turn"):
        if not CANONICAL_NON_NEGATIVE_INTEGER.fullmatch(metadata[field]):
            raise ValueError(
                f"Request {request_id} has invalid {field}: {metadata[field]!r}"
            )

    errors_raw = metadata["tool_contract_errors_json"]
    try:
        errors = strict_json_loads(errors_raw)
    except JSON_LOAD_ERRORS as error:
        raise ValueError(
            f"Request {request_id} has invalid tool_contract_errors_json"
        ) from error
    if not isinstance(errors, list) or any(
        not isinstance(error, str) or not error.strip() for error in errors
    ):
        raise ValueError(
            f"Request {request_id} has invalid tool_contract_errors_json"
        )

    status = metadata["tool_contract_status"]
    if status == "conformant" and errors:
        raise ValueError(
            f"Request {request_id} conformant tool_contract_status requires "
            "an empty tool_contract_errors_json array"
        )
    if status == "non_conformant" and not errors:
        raise ValueError(
            f"Request {request_id} non_conformant tool_contract_status requires "
            "a non-empty tool_contract_errors_json array"
        )


def _redact_v4_dynamic_metadata(
    metadata: Dict[str, str],
    secrets: Set[str],
) -> Dict[str, str]:
    redacted = dict(metadata)
    for field in ("stream_end_signal", "model_stop_reason"):
        redacted[field] = redact_text(metadata[field], secrets)

    errors = strict_json_loads(metadata["tool_contract_errors_json"])
    assert isinstance(errors, list)
    redacted["tool_contract_errors_json"] = json.dumps(
        [redact_text(error, secrets) for error in errors],
        ensure_ascii=False,
        separators=(",", ":"),
    )
    return redacted


def parse_log(path: Path) -> Dict[str, object]:
    """Parse one supported evidence log without retaining its absolute path."""

    path = Path(path)
    raw = path.read_bytes()
    decoded = raw.decode("utf-8", errors="replace")
    raw_run = _run_header(decoded)
    contract = contract_key(raw_run)
    if raw_run.get("section_encoding") != SECTION_ENCODING:
        raise ValueError(
            f"Unsupported or missing section_encoding: "
            f"{raw_run.get('section_encoding')!r}; expected {SECTION_ENCODING}"
        )

    raw_request_blocks = _blocks(decoded, "REQUEST")
    secrets = _discover_secrets(decoded)
    decoded_request_sections = {}
    for identifier, block in raw_request_blocks:
        if identifier in decoded_request_sections:
            raise ValueError(f"Duplicate request block: {identifier}")
        request_body = _encoded_section(block, "REQUEST BODY")
        decoded_request_sections[identifier] = {
            "requestBody": request_body,
            "stderr": _encoded_section(block, "CURL STDERR"),
            "responseBody": _encoded_section(block, "RESPONSE BODY"),
        }
        secrets.update(_discover_secrets(request_body))
    raw_api_key = raw_run.get("api_key", "")
    if raw_api_key and not _credential_is_masked(raw_api_key):
        secrets.add(raw_api_key)
    run = dict(raw_run)
    for field in ("url", "model"):
        if field in run:
            run[field] = redact_text(run[field], secrets)
    if raw_api_key and not _credential_is_masked(raw_api_key):
        run["api_key"] = "[REDACTED]"

    requests: Dict[str, Dict[str, object]] = {}
    for identifier, block in raw_request_blocks:
        metadata_text = block.split("-----", 1)[0]
        metadata = _key_values(metadata_text)
        if metadata.get("request_id", identifier) != identifier:
            raise ValueError(
                f"Request block {identifier} declares request_id="
                f"{metadata.get('request_id')!r}"
            )
        if contract == V4_CONTRACT:
            _validate_v4_request_metadata(metadata, identifier)
            metadata = _redact_v4_dynamic_metadata(metadata, secrets)
        requests[identifier] = {
            **metadata,
            "request_id": identifier,
            "curlCommand": redact_text(
                _section(block, "CURL COMMAND"),
                secrets,
            ),
            "requestBody": redact_text(
                decoded_request_sections[identifier]["requestBody"],
                secrets,
            ),
            "metrics": _key_values(_section(block, "RESPONSE METRICS")),
            "responseHeaders": redact_text(
                _section(block, "RESPONSE HEADERS"),
                secrets,
            ),
            "stderr": redact_text(
                decoded_request_sections[identifier]["stderr"],
                secrets,
            ),
            "responseBody": redact_text(
                decoded_request_sections[identifier]["responseBody"],
                secrets,
            ),
        }

    tests: Dict[str, Dict[str, object]] = {}
    for identifier, block in _blocks(decoded, "TEST"):
        if identifier in tests:
            raise ValueError(f"Duplicate test block: {identifier}")
        if identifier not in CONTRACT_TEST_IDS[contract]:
            raise ValueError(f"Unsupported test ID in evidence log: {identifier}")
        metadata = _key_values(block)
        forbidden = FORBIDDEN_MANIFEST_FIELDS.intersection(metadata)
        if forbidden:
            raise ValueError(
                f"TEST-{identifier} contains forbidden judgment fields: "
                f"{', '.join(sorted(forbidden))}"
            )
        for field in ("name", "category"):
            if not metadata.get(field, "").strip():
                raise ValueError(
                    f"TEST-{identifier} {field} must be non-empty"
                )
        if "request_refs" not in metadata:
            raise ValueError(f"TEST-{identifier} is missing request_refs")
        refs = _parse_request_refs(metadata.pop("request_refs"), identifier)
        for request_id in refs:
            if request_id not in requests:
                raise ValueError(
                    f"TEST-{identifier} references missing request: {request_id}"
                )
        tests[identifier] = {
            **metadata,
            "id": identifier,
            "requestRefs": refs,
        }

    summary = _run_summary(decoded)
    if not summary:
        raise ValueError("Missing RUN SUMMARY")
    discovered_test_ids = set(tests)
    if "collection_profile" in run:
        raise ValueError("Evidence v4 must not contain collection_profile")
    profile = run.pop("compatibility_profile", None)
    if profile != COMPATIBILITY_PROFILE:
        raise ValueError("Unsupported or missing compatibility_profile")
    if discovered_test_ids != V4_TEST_IDS:
        raise ValueError("Evidence v4 must contain all 46 retained tests")
    run["compatibilityProfile"] = profile
    _validate_count(run, "selected_test_count", len(tests))
    _validate_count(summary, "request_count", len(requests))
    _validate_count(summary, "test_manifest_count", len(tests))

    return {
        "schemaVersion": PARSED_SCHEMA_VERSION,
        "source": {
            "fileName": path.name,
            "size": len(raw),
            "sha256": hashlib.sha256(raw).hexdigest(),
        },
        "run": run,
        "requests": requests,
        "tests": tests,
        "summary": summary,
        "tokenTotals": _sum_usage(
            request["responseBody"] for request in requests.values()
        ),
        "warnings": [],
    }


def test_packet(parsed: Dict[str, object], test_id: str) -> Dict[str, object]:
    """Return compact evidence for one test without unrelated request content."""

    tests = parsed.get("tests", {})
    if test_id not in tests:
        raise KeyError(f"Unknown test ID: {test_id}")
    test = tests[test_id]
    requests = parsed.get("requests", {})
    return {
        "source": parsed.get("source", {}),
        "run": parsed.get("run", {}),
        "test": test,
        "requests": [
            requests[request_id] for request_id in test.get("requestRefs", [])
        ],
        "warnings": parsed.get("warnings", []),
    }
