#!/usr/bin/env python3
"""Parse Model Doctor audit logs into inert, redacted evidence."""

from __future__ import annotations

import hashlib
import re
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Set, Tuple
from urllib.parse import parse_qsl, urlsplit


PARSED_SCHEMA_VERSION = "llm-capability-doctor.parsed-log.v1"
SECRET_QUERY_KEYS = {"api_key", "key", "token", "access_token", "client_secret", "password"}
SECRET_JSON_KEYS = SECRET_QUERY_KEYS | {"apiKey", "clientSecret", "authorization"}
SECRET_HEADER_NAMES = {
    "authorization",
    "proxy-authorization",
    "api-key",
    "x-api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
}


def _discover_secrets(value: str) -> Set[str]:
    secrets: Set[str] = set()

    for match in re.finditer(r"https?://[^\s'\"]+", value):
        try:
            for key, item in parse_qsl(urlsplit(match.group(0)).query, keep_blank_values=True):
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
        r'"(' + "|".join(re.escape(key) for key in sorted(SECRET_JSON_KEYS)) + r')"\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)"',
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
        rf"^----- {re.escape(name)} BEGIN -----\n(.*?)^----- {re.escape(name)} END -----$",
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
            continue
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
    return _key_values(body if end < 0 else body[:end])


def _request_test_id(request_id: str) -> Optional[str]:
    match = re.match(r"^test-([0-9]+)(?:-|$)", request_id)
    return match.group(1) if match else None


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


def parse_log(path: Path) -> Dict[str, object]:
    """Parse one audit log without changing it or retaining its absolute path."""

    path = Path(path)
    raw = path.read_bytes()
    decoded = raw.decode("utf-8", errors="replace")
    secrets = _discover_secrets(decoded)
    text = redact_text(decoded, secrets)
    warnings: List[str] = []

    run = _run_header(text)
    if not run:
        warnings.append("Missing or invalid MODEL DOCTOR RUN header")

    requests: Dict[str, Dict[str, object]] = {}
    for identifier, block in _blocks(text, "REQUEST"):
        metadata_text = block.split("-----", 1)[0]
        metadata = _key_values(metadata_text)
        metrics = _key_values(_section(block, "RESPONSE METRICS"))
        requests[identifier] = {
            **metadata,
            "request_id": metadata.get("request_id", identifier),
            "requestBody": _section(block, "REQUEST BODY"),
            "metrics": metrics,
            "responseHeaders": _section(block, "RESPONSE HEADERS"),
            "stderr": _section(block, "CURL STDERR"),
            "responseBody": _section(block, "RESPONSE BODY"),
        }

    tests: Dict[str, Dict[str, object]] = {}
    for identifier, block in _blocks(text, "TEST"):
        metadata_text = block.split("----- RAW RESPONSE BEGIN -----", 1)[0]
        metadata: Dict[str, object] = _key_values(metadata_text)
        metadata["id"] = identifier
        metadata["rawResponse"] = _section(block, "RAW RESPONSE")
        metadata["requestRefs"] = [
            request_id
            for request_id in requests
            if _request_test_id(request_id) == identifier
        ]
        tests[identifier] = metadata

    summary = _run_summary(text)
    if not summary:
        warnings.append("Missing RUN SUMMARY; counts must be reconstructed")

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
        "warnings": warnings,
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
        "requests": [requests[request_id] for request_id in test.get("requestRefs", [])],
        "warnings": parsed.get("warnings", []),
    }
