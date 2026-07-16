#!/usr/bin/env python3
"""Orchestrate Model Doctor log parsing, review validation, and reporting."""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from datetime import datetime
from pathlib import Path
from typing import Sequence

from model_doctor_assessment import assemble_assessment, validate_reviews
from model_doctor_html import render_report
from model_doctor_log import parse_log, test_packet


SCRIPT_DIR = Path(__file__).resolve().parent
ASSET_DIR = SCRIPT_DIR.parent / "assets"


class CliUsageError(Exception):
    """Report invalid command input with exit code 2."""


def _json_text(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def _load_json(path: Path) -> object:
    with Path(path).open(encoding="utf-8") as handle:
        return json.load(handle)


def _collision_safe_path(requested: Path) -> Path:
    requested = Path(requested).expanduser().resolve()
    if not requested.exists():
        return requested

    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    candidate = requested.with_name(f"{requested.stem}-{timestamp}{requested.suffix}")
    counter = 2
    while candidate.exists():
        candidate = requested.with_name(
            f"{requested.stem}-{timestamp}-{counter}{requested.suffix}"
        )
        counter += 1
    return candidate


def _write_output(requested: Path, content: str) -> Path:
    destination = _collision_safe_path(requested)
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("x", encoding="utf-8") as handle:
        handle.write(content)
    return destination


def _summary(parsed: dict) -> dict:
    tests = parsed.get("tests", {})
    requests = parsed.get("requests", {})
    return {
        "schemaVersion": parsed.get("schemaVersion"),
        "source": parsed.get("source", {}),
        "run": parsed.get("run", {}),
        "testCount": len(tests),
        "requestCount": len(requests),
        "categoryCounts": dict(
            sorted(Counter(test.get("category", "Unclassified") for test in tests.values()).items())
        ),
        "tokenTotals": parsed.get("tokenTotals", {}),
        "warnings": parsed.get("warnings", []),
    }


def _parse_command(arguments: argparse.Namespace) -> int:
    parsed = parse_log(arguments.log)
    destination = _write_output(arguments.output, _json_text(parsed))
    print(destination)
    return 0


def _summary_command(arguments: argparse.Namespace) -> int:
    parsed = _load_json(arguments.parsed)
    if not isinstance(parsed, dict):
        raise CliUsageError("Parsed evidence must be a JSON object")
    print(_json_text(_summary(parsed)), end="")
    return 0


def _packet_command(arguments: argparse.Namespace) -> int:
    parsed = _load_json(arguments.parsed)
    if not isinstance(parsed, dict):
        raise CliUsageError("Parsed evidence must be a JSON object")
    identifiers = [item.strip() for item in arguments.ids.split(",") if item.strip()]
    if not identifiers:
        raise CliUsageError("--ids must contain at least one test ID")
    try:
        packets = {identifier: test_packet(parsed, identifier) for identifier in identifiers}
    except KeyError as error:
        raise CliUsageError(str(error)) from error
    print(_json_text({"packets": packets}), end="")
    return 0


def _validated_inputs(parsed_path: Path, reviews_path: Path) -> tuple[dict, dict, list[str]]:
    parsed = _load_json(parsed_path)
    reviews = _load_json(reviews_path)
    if not isinstance(parsed, dict):
        raise CliUsageError("Parsed evidence must be a JSON object")
    if not isinstance(reviews, dict):
        raise CliUsageError("Reviews must be a JSON object keyed by test ID")
    return parsed, reviews, validate_reviews(parsed, reviews)


def _validate_command(arguments: argparse.Namespace) -> int:
    _, _, errors = _validated_inputs(arguments.parsed, arguments.reviews)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 2
    print("Reviews are valid.")
    return 0


def _render_command(arguments: argparse.Namespace) -> int:
    parsed, reviews, errors = _validated_inputs(arguments.parsed, arguments.reviews)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 2

    assessment = assemble_assessment(parsed, reviews)
    assessment_text = _json_text(assessment)
    html_text = render_report(assessment, ASSET_DIR)
    assessment_path = _write_output(arguments.assessment, assessment_text)
    html_path = _write_output(arguments.html, html_text)
    print(assessment_path)
    print(html_path)
    return 0


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Turn Model Doctor audit logs and semantic reviews into customer reports."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    parse_parser = subparsers.add_parser("parse", help="Parse and redact one Model Doctor log")
    parse_parser.add_argument("log", type=Path)
    parse_parser.add_argument("--output", type=Path, required=True)
    parse_parser.set_defaults(handler=_parse_command)

    summary_parser = subparsers.add_parser("summary", help="Print compact parsed-log metadata")
    summary_parser.add_argument("parsed", type=Path)
    summary_parser.set_defaults(handler=_summary_command)

    packet_parser = subparsers.add_parser("packet", help="Print evidence for selected test IDs")
    packet_parser.add_argument("parsed", type=Path)
    packet_parser.add_argument("--ids", required=True)
    packet_parser.set_defaults(handler=_packet_command)

    validate_parser = subparsers.add_parser("validate", help="Validate semantic review JSON")
    validate_parser.add_argument("parsed", type=Path)
    validate_parser.add_argument("reviews", type=Path)
    validate_parser.set_defaults(handler=_validate_command)

    render_parser = subparsers.add_parser("render", help="Generate assessment JSON and offline HTML")
    render_parser.add_argument("parsed", type=Path)
    render_parser.add_argument("reviews", type=Path)
    render_parser.add_argument("--assessment", type=Path, required=True)
    render_parser.add_argument("--html", type=Path, required=True)
    render_parser.set_defaults(handler=_render_command)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    arguments = _parser().parse_args(argv)
    try:
        return arguments.handler(arguments)
    except CliUsageError as error:
        print(f"Invalid input: {error}", file=sys.stderr)
        return 2
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError) as error:
        print(f"Model Doctor report failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
