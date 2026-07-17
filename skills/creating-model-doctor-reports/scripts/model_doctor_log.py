#!/usr/bin/env python3
"""Parse evidence-v1 Model Doctor logs into inert, redacted evidence."""

from __future__ import annotations

import hashlib
import re
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Set, Tuple
from urllib.parse import parse_qsl, urlsplit


PARSED_SCHEMA_VERSION = "llm-capability-doctor.parsed-evidence.v1"
EVIDENCE_LOG_SCHEMA = "llm-capability-doctor.evidence.v1"
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
            item = match.group(2).strip()
            if item and item != "[REDACTED]":
                secrets.add(item)
                if item.lower().startswith("bearer ") and len(item) > 7:
                    secrets.add(item[7:].strip())

    json_key_pattern = re.compile(
        r'"('
        + "|".join(re.escape(key) for key in sorted(SECRET_JSON_KEYS))
        + r')"\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)"',
        re.IGNORECASE,
    )
    for match in json_key_pattern.finditer(value):
        item = match.group(2)
        if item and item != "[REDACTED]":
            secrets.add(item)

    return secrets


def redact_text(value: str, discovered_secrets: Optional[Set[str]] = None) -> str:
    """Redact credential-bearing fields and echoed copies of discovered values."""

    secrets = set(discovered_secrets or ())
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
    redacted = re.sub(
        r'(?i)("(?:api_key|apiKey|access_token|token|client_secret|clientSecret|password|authorization)"\s*:\s*)"(?:[^"\\]|\\.)*"',
        r'\1"[REDACTED]"',
        redacted,
    )
    redacted = re.sub(
        r"(?i)([?&](?:api_key|key|token|access_token|client_secret|password)=)[^&\s'\"]*",
        r"\1[REDACTED]",
        redacted,
    )

    for secret in sorted(secrets, key=len, reverse=True):
        if secret and secret != "[REDACTED]":
            redacted = redacted.replace(secret, "[REDACTED]")
    return redacted


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
    return match.group(1).rstrip("\n") if match else ""


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


def parse_log(path: Path) -> Dict[str, object]:
    """Parse one strict evidence-v1 log without retaining its absolute path."""

    path = Path(path)
    raw = path.read_bytes()
    decoded = raw.decode("utf-8", errors="replace")
    secrets = _discover_secrets(decoded)
    text = redact_text(decoded, secrets)

    run = _run_header(text)
    if run.get("log_schema") != EVIDENCE_LOG_SCHEMA:
        raise ValueError(
            f"Unsupported or missing log_schema: {run.get('log_schema')!r}"
        )

    requests: Dict[str, Dict[str, object]] = {}
    for identifier, block in _blocks(text, "REQUEST"):
        if identifier in requests:
            raise ValueError(f"Duplicate request block: {identifier}")
        metadata_text = block.split("-----", 1)[0]
        metadata = _key_values(metadata_text)
        if metadata.get("request_id", identifier) != identifier:
            raise ValueError(
                f"Request block {identifier} declares request_id="
                f"{metadata.get('request_id')!r}"
            )
        requests[identifier] = {
            **metadata,
            "request_id": identifier,
            "curlCommand": _section(block, "CURL COMMAND"),
            "requestBody": _section(block, "REQUEST BODY"),
            "metrics": _key_values(_section(block, "RESPONSE METRICS")),
            "responseHeaders": _section(block, "RESPONSE HEADERS"),
            "stderr": _section(block, "CURL STDERR"),
            "responseBody": _section(block, "RESPONSE BODY"),
        }

    tests: Dict[str, Dict[str, object]] = {}
    for identifier, block in _blocks(text, "TEST"):
        if identifier in tests:
            raise ValueError(f"Duplicate test block: {identifier}")
        metadata = _key_values(block)
        forbidden = FORBIDDEN_MANIFEST_FIELDS.intersection(metadata)
        if forbidden:
            raise ValueError(
                f"TEST-{identifier} contains forbidden judgment fields: "
                f"{', '.join(sorted(forbidden))}"
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

    summary = _run_summary(text)
    if not summary:
        raise ValueError("Missing RUN SUMMARY")
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
