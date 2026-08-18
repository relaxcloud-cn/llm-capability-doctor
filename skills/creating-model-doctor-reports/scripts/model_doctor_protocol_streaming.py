"""Validate protocol-specific streaming response framing and lifecycles.

This module stays independent from the non-stream conformance facade.  The
facade injects its difference factory and frozen profile validator so sibling
stream modules can share one result format without creating import cycles.
"""

from __future__ import annotations

import inspect
import json
from dataclasses import dataclass
from typing import Callable, List, Optional


@dataclass(frozen=True)
class StreamEvidence:
    request_id: str
    protocol: str
    response_body: str
    response_headers: object
    http_status: int
    request_body: object
    profile_validator: object


@dataclass(frozen=True)
class _SseEvent:
    index: int
    event: Optional[str]
    data: str


@dataclass(frozen=True)
class _JsonFrame:
    index: int
    value: object


def _actual_type(value: object) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__


def _is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _pointer_token(value: object) -> str:
    return str(value).replace("~", "~0").replace("/", "~1")


class _Context:
    def __init__(
        self,
        evidence: StreamEvidence,
        make_difference: Callable[..., dict],
    ) -> None:
        self.evidence = evidence
        self.make_difference = make_difference
        self.differences: List[dict] = []

    def add(
        self,
        location: str,
        kind: str,
        expected: str,
        actual: str,
    ) -> None:
        self.differences.append(
            self.make_difference(
                self.evidence.request_id,
                self.evidence.protocol,
                location,
                kind,
                expected,
                actual,
            )
        )

    def object_shape(
        self,
        value: object,
        location: str,
        required: tuple[str, ...],
        optional: tuple[str, ...] = (),
    ) -> bool:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return False
        fields = set(value)
        for field in sorted(set(required) - fields):
            self.add(
                f"{location}/{_pointer_token(field)}",
                "MISSING_FIELD",
                "present",
                "missing",
            )
        for field in sorted(fields - set(required) - set(optional)):
            self.add(
                f"{location}/{_pointer_token(field)}",
                "UNEXPECTED_FIELD",
                "absent from the pinned official object",
                "present",
            )
        return True

    def profile(self, method_name: str, value: object, prefix: str) -> None:
        """Run a root-level frozen profile method and relocate its differences."""

        validator = self.evidence.profile_validator
        collected = getattr(validator, "differences")
        before = len(collected)
        method = getattr(validator, method_name)
        accepts_location = len(inspect.signature(method).parameters) >= 2
        if accepts_location:
            method(value, prefix)
        else:
            method(value)
        created = list(collected[before:])
        del collected[before:]
        if accepts_location:
            self.differences.extend(created)
            return
        for difference in created:
            relocated = dict(difference)
            location = relocated.get("location")
            if location == "/responseBody":
                relocated["location"] = prefix
            elif isinstance(location, str) and location.startswith("/"):
                relocated["location"] = prefix + location
            else:
                relocated["location"] = prefix
            self.differences.append(relocated)

    def profile_at(
        self,
        method_name: str,
        value: object,
        location: str,
    ) -> None:
        """Run a frozen profile helper that already accepts an absolute path."""

        validator = self.evidence.profile_validator
        collected = getattr(validator, "differences")
        before = len(collected)
        getattr(validator, method_name)(value, location)
        self.differences.extend(collected[before:])
        del collected[before:]


def _parse_sse(ctx: _Context) -> List[_SseEvent]:
    frames: List[List[str]] = []
    current: List[str] = []
    for raw_line in ctx.evidence.response_body.splitlines():
        line = raw_line[:-1] if raw_line.endswith("\r") else raw_line
        if line == "":
            if current:
                frames.append(current)
                current = []
            continue
        current.append(line)
    if current:
        frames.append(current)

    events: List[_SseEvent] = []
    for lines in frames:
        if all(line.startswith(":") for line in lines):
            continue
        index = len(events)
        event_name: Optional[str] = None
        data_lines: List[str] = []
        for line in lines:
            if line.startswith(":"):
                continue
            field, separator, value = line.partition(":")
            if separator and value.startswith(" "):
                value = value[1:]
            if field == "event":
                if event_name is not None:
                    ctx.add(
                        f"/events/{index}/event",
                        "FRAMING",
                        "at most one event field per SSE event",
                        "multiple event fields",
                    )
                event_name = value
            elif field == "data":
                data_lines.append(value)
            else:
                ctx.add(
                    f"/events/{index}/{_pointer_token(field or 'line')}",
                    "FRAMING",
                    "event and data SSE fields only",
                    "unsupported SSE field",
                )
        if not data_lines:
            ctx.add(
                f"/events/{index}/data",
                "FRAMING",
                "one or more data fields",
                "missing data field",
            )
        events.append(
            _SseEvent(index=index, event=event_name, data="\n".join(data_lines))
        )
    return events


def _parse_ndjson(ctx: _Context) -> List[_JsonFrame]:
    frames: List[_JsonFrame] = []
    for line in ctx.evidence.response_body.splitlines():
        if not line.strip():
            continue
        index = len(frames)
        stripped = line.lstrip()
        if stripped.startswith("data:") or stripped.startswith("event:"):
            ctx.add(
                f"/records/{index}",
                "FRAMING",
                "one JSON object per NDJSON record without SSE fields",
                "SSE-framed record",
            )
            frames.append(_JsonFrame(index=index, value=None))
            continue
        try:
            value = json.loads(line)
        except (TypeError, ValueError):
            ctx.add(
                f"/records/{index}",
                "INVALID_JSON",
                "one JSON value per NDJSON record",
                "invalid JSON",
            )
            value = None
        else:
            if value is None:
                ctx.add(
                    f"/records/{index}",
                    "TYPE_MISMATCH",
                    "JSON object",
                    "null",
                )
        frames.append(_JsonFrame(index=index, value=value))
    return frames


def _decode_event_json(
    ctx: _Context,
    event: _SseEvent,
    allow_done: bool = False,
) -> object:
    if event.data == "[DONE]":
        if not allow_done:
            ctx.add(
                f"/events/{event.index}/data",
                "FRAMING",
                "JSON event data",
                "unexpected terminal marker",
            )
        return None
    try:
        decoded = json.loads(event.data)
    except (TypeError, ValueError):
        ctx.add(
            f"/events/{event.index}/data",
            "INVALID_JSON",
            "one JSON value in the SSE data field",
            "invalid JSON",
        )
        return None
    if decoded is None:
        ctx.add(
            f"/events/{event.index}/data",
            "TYPE_MISMATCH",
            "JSON object",
            "null",
        )
        return None
    return decoded


def _content_type(headers: object) -> Optional[str]:
    if not isinstance(headers, str):
        return None
    found = None
    for line in headers.splitlines():
        name, separator, value = line.partition(":")
        if separator and name.strip().lower() == "content-type":
            found = value.strip()
    return found


def _validate_content_type(ctx: _Context, expected: str) -> None:
    content_type = _content_type(ctx.evidence.response_headers)
    if content_type is None:
        return
    media_type = content_type.split(";", 1)[0].strip().lower()
    if media_type != expected:
        ctx.add(
            "/responseHeaders/content-type",
            "VALUE_MISMATCH",
            f"{expected} when content-type is present",
            media_type or "empty media type",
        )


def _validate_chat_delta(ctx: _Context, value: object, location: str) -> None:
    if not ctx.object_shape(
        value,
        location,
        (),
        ("content", "function_call", "refusal", "role", "tool_calls"),
    ):
        return
    assert isinstance(value, dict)
    for field in ("content", "refusal"):
        if field in value and value[field] is not None and not isinstance(value[field], str):
            ctx.add(
                f"{location}/{field}",
                "TYPE_MISMATCH",
                "string or null",
                _actual_type(value[field]),
            )
    if "role" in value and value["role"] != "assistant":
        ctx.add(
            f"{location}/role",
            "ENUM_MISMATCH",
            "assistant",
            _actual_type(value["role"]),
        )
    if "function_call" in value:
        function_call = value["function_call"]
        path = f"{location}/function_call"
        if ctx.object_shape(function_call, path, (), ("name", "arguments")):
            assert isinstance(function_call, dict)
            for field in ("name", "arguments"):
                if field in function_call and not isinstance(function_call[field], str):
                    ctx.add(
                        f"{path}/{field}",
                        "TYPE_MISMATCH",
                        "string",
                        _actual_type(function_call[field]),
                    )
    if "tool_calls" not in value:
        return
    tool_calls = value["tool_calls"]
    tool_path = f"{location}/tool_calls"
    if not isinstance(tool_calls, list):
        ctx.add(tool_path, "TYPE_MISMATCH", "array", _actual_type(tool_calls))
        return
    for item_index, tool_call in enumerate(tool_calls):
        item_path = f"{tool_path}/{item_index}"
        if not ctx.object_shape(
            tool_call,
            item_path,
            ("index",),
            ("id", "type", "function"),
        ):
            continue
        assert isinstance(tool_call, dict)
        if "index" in tool_call and not _is_integer(tool_call["index"]):
            ctx.add(
                f"{item_path}/index",
                "TYPE_MISMATCH",
                "integer",
                _actual_type(tool_call["index"]),
            )
        for field in ("id", "type"):
            if field in tool_call and not isinstance(tool_call[field], str):
                ctx.add(
                    f"{item_path}/{field}",
                    "TYPE_MISMATCH",
                    "string",
                    _actual_type(tool_call[field]),
                )
        if "type" in tool_call and tool_call["type"] != "function":
            ctx.add(
                f"{item_path}/type",
                "ENUM_MISMATCH",
                "function",
                _actual_type(tool_call["type"]),
            )
        if "function" in tool_call:
            function = tool_call["function"]
            function_path = f"{item_path}/function"
            if ctx.object_shape(function, function_path, (), ("name", "arguments")):
                assert isinstance(function, dict)
                for field in ("name", "arguments"):
                    if field in function and not isinstance(function[field], str):
                        ctx.add(
                            f"{function_path}/{field}",
                            "TYPE_MISMATCH",
                            "string",
                            _actual_type(function[field]),
                        )


def _validate_chat_chunk(
    ctx: _Context,
    event: _SseEvent,
    value: object,
    correlated: dict,
    choice_count: Optional[int],
) -> None:
    location = f"/events/{event.index}/data"
    if not ctx.object_shape(
        value,
        location,
        ("id", "object", "created", "model", "choices"),
        ("service_tier", "system_fingerprint", "usage"),
    ):
        return
    assert isinstance(value, dict)

    validators = {
        "id": lambda item: isinstance(item, str),
        "created": _is_integer,
        "model": lambda item: isinstance(item, str),
    }
    for field, predicate in validators.items():
        if field not in value:
            continue
        field_path = f"{location}/{field}"
        if not predicate(value[field]):
            ctx.add(
                field_path,
                "TYPE_MISMATCH",
                "integer" if field == "created" else "string",
                _actual_type(value[field]),
            )
        elif field not in correlated:
            correlated[field] = value[field]
        elif value[field] != correlated[field]:
            ctx.add(
                field_path,
                "CORRELATION",
                f"same {field} as earlier chunks",
                "correlated values differ",
            )
    if "object" in value and value["object"] != "chat.completion.chunk":
        ctx.add(
            f"{location}/object",
            "ENUM_MISMATCH",
            "chat.completion.chunk",
            _actual_type(value["object"]),
        )
    for field in ("service_tier", "system_fingerprint"):
        if field in value and value[field] is not None and not isinstance(value[field], str):
            ctx.add(
                f"{location}/{field}",
                "TYPE_MISMATCH",
                "string or null",
                _actual_type(value[field]),
            )

    choices = value.get("choices")
    if not isinstance(choices, list):
        if "choices" in value:
            ctx.add(
                f"{location}/choices",
                "TYPE_MISMATCH",
                "array",
                _actual_type(choices),
            )
    else:
        seen_indexes = set()
        for choice_index, choice in enumerate(choices):
            choice_path = f"{location}/choices/{choice_index}"
            if not ctx.object_shape(
                choice,
                choice_path,
                ("index", "delta", "finish_reason"),
                ("logprobs",),
            ):
                continue
            assert isinstance(choice, dict)
            if "index" in choice:
                index = choice["index"]
                if not _is_integer(index):
                    ctx.add(
                        f"{choice_path}/index",
                        "TYPE_MISMATCH",
                        "integer",
                        _actual_type(index),
                    )
                elif index in seen_indexes:
                    ctx.add(
                        f"{choice_path}/index",
                        "CORRELATION",
                        "unique choice index within the chunk",
                        "duplicate choice index",
                    )
                else:
                    seen_indexes.add(index)
                    if choice_count is not None and not 0 <= index < choice_count:
                        ctx.add(
                            f"{choice_path}/index",
                            "CORRELATION",
                            f"choice index in the request n range 0..{choice_count - 1}",
                            "choice index is outside the requested range",
                        )
            if "delta" in choice:
                _validate_chat_delta(ctx, choice["delta"], f"{choice_path}/delta")
            if "finish_reason" in choice:
                reason = choice["finish_reason"]
                allowed = {
                    None,
                    "stop",
                    "length",
                    "tool_calls",
                    "content_filter",
                    "function_call",
                }
                if reason not in allowed:
                    ctx.add(
                        f"{choice_path}/finish_reason",
                        "ENUM_MISMATCH",
                        "null or an official finish reason",
                        _actual_type(reason),
                    )
            if "logprobs" in choice and choice["logprobs"] is not None:
                ctx.profile_at(
                    "validate_logprobs",
                    choice["logprobs"],
                    f"{choice_path}/logprobs",
                )
    if "usage" in value and value["usage"] is not None:
        ctx.profile_at("validate_usage", value["usage"], f"{location}/usage")


def _chat_choice_count(request_body: object) -> Optional[int]:
    decoded = request_body
    if isinstance(decoded, str):
        try:
            decoded = json.loads(decoded)
        except (TypeError, ValueError):
            return None
    if not isinstance(decoded, dict):
        return None
    value = decoded.get("n", 1)
    return value if _is_integer(value) and value > 0 else None


def _validate_chat_stream(ctx: _Context, events: List[_SseEvent]) -> None:
    done_indexes: List[int] = []
    correlated: dict = {}
    choice_count = _chat_choice_count(ctx.evidence.request_body)
    response_chunks = 0
    for event in events:
        if event.event is not None:
            ctx.add(
                f"/events/{event.index}/event",
                "FRAMING",
                "data-only SSE event without an event field",
                "named SSE event",
            )
        if event.data == "[DONE]":
            done_indexes.append(event.index)
            continue
        decoded = _decode_event_json(ctx, event, allow_done=True)
        if decoded is not None:
            if isinstance(decoded, dict):
                response_chunks += 1
            _validate_chat_chunk(ctx, event, decoded, correlated, choice_count)

    if done_indexes and response_chunks == 0:
        ctx.add(
            f"/events/{done_indexes[0]}/data",
            "SEQUENCE",
            "at least one response chunk before [DONE]",
            "terminal marker without a response chunk",
        )

    expected_done_index = len(events) - 1
    if done_indexes == [expected_done_index]:
        return
    if not done_indexes:
        location = f"/events/{len(events)}"
    elif done_indexes[0] != expected_done_index:
        location = f"/events/{done_indexes[0]}"
    else:
        location = f"/events/{done_indexes[1]}"
    ctx.add(
        location,
        "SEQUENCE",
        "exactly one [DONE] marker as the final SSE event",
        "terminal marker sequence differs",
    )


def validate_stream_response(
    evidence: StreamEvidence,
    *,
    make_difference: Callable[..., dict],
    validate_http_error: Callable[[object], List[dict]],
) -> List[dict]:
    """Return official streaming-protocol differences for one raw response."""

    ctx = _Context(evidence, make_difference)
    if not 200 <= evidence.http_status <= 299:
        _validate_content_type(ctx, "application/json")
        try:
            decoded = json.loads(evidence.response_body)
        except (TypeError, ValueError):
            ctx.add(
                "/responseBody",
                "INVALID_JSON",
                "one JSON error response object",
                "invalid JSON",
            )
        else:
            ctx.differences.extend(validate_http_error(decoded))
        return ctx.differences

    expected_media_type = (
        "application/x-ndjson"
        if evidence.protocol == "ollama_chat"
        else "text/event-stream"
    )
    _validate_content_type(ctx, expected_media_type)

    if evidence.protocol == "ollama_chat":
        from model_doctor_protocol_streaming_other import validate_ollama_stream

        validate_ollama_stream(ctx, _parse_ndjson(ctx))
        return ctx.differences

    events = _parse_sse(ctx)
    if evidence.protocol == "openai_chat":
        _validate_chat_stream(ctx, events)
    elif evidence.protocol == "openai_responses":
        from model_doctor_protocol_streaming_responses import validate_responses_stream

        validate_responses_stream(ctx, events)
    elif evidence.protocol == "anthropic_messages":
        from model_doctor_protocol_streaming_other import validate_anthropic_stream

        validate_anthropic_stream(ctx, events)
    elif evidence.protocol == "gemini_generate_content":
        from model_doctor_protocol_streaming_other import validate_gemini_stream

        validate_gemini_stream(ctx, events)
    return ctx.differences
