"""Streaming validators for Anthropic, Gemini, and Ollama protocols."""

from __future__ import annotations

from typing import Callable, Mapping, Optional, Sequence

from model_doctor_json import JSON_LOAD_ERRORS, strict_json_loads
from model_doctor_protocol_streaming import (
    _Context,
    _JsonFrame,
    _SseEvent,
    _decode_event_json,
)


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
    return "value of unexpected type"


def _is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _join(location: str, field: object) -> str:
    token = str(field).replace("~", "~0").replace("/", "~1")
    return f"{location}/{token}" if location else f"/{token}"


def _shape(
    ctx: _Context,
    value: object,
    location: str,
    required: Sequence[str],
    optional: Sequence[str] = (),
) -> bool:
    if not isinstance(value, dict):
        ctx.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
        return False
    fields = set(value)
    for field in sorted(set(required) - fields):
        ctx.add(_join(location, field), "MISSING_FIELD", "present", "missing")
    for field in sorted(fields - set(required) - set(optional)):
        ctx.add(
            _join(location, field),
            "UNEXPECTED_FIELD",
            "absent from the pinned official object",
            "present",
        )
    return True


def _typed(
    ctx: _Context,
    value: object,
    location: str,
    expected: str,
    predicate: Callable[[object], bool],
) -> bool:
    if predicate(value):
        return True
    ctx.add(location, "TYPE_MISMATCH", expected, _actual_type(value))
    return False


def _string(ctx: _Context, value: object, location: str) -> bool:
    return _typed(ctx, value, location, "string", lambda item: isinstance(item, str))


def _integer(ctx: _Context, value: object, location: str) -> bool:
    return _typed(ctx, value, location, "integer", _is_integer)


def _nullable_string(ctx: _Context, value: object, location: str) -> bool:
    return _typed(
        ctx,
        value,
        location,
        "string or null",
        lambda item: item is None or isinstance(item, str),
    )


def _enum(
    ctx: _Context,
    value: object,
    location: str,
    allowed: Sequence[str],
) -> bool:
    if isinstance(value, str) and value in allowed:
        return True
    ctx.add(
        location,
        "ENUM_MISMATCH",
        "one of " + ", ".join(allowed),
        _actual_type(value),
    )
    return False


def _decode(ctx: _Context, event: _SseEvent) -> tuple[bool, object]:
    """Decode through the shared parser; it records invalid JSON and JSON null."""

    value = _decode_event_json(ctx, event)
    return value is not None, value


def _profile_success_at(ctx: _Context, value: object, prefix: str) -> None:
    """Rebase response fields while preserving request-evidence pointers."""

    validator = ctx.evidence.profile_validator
    differences = getattr(validator, "differences", None)
    callback = getattr(validator, "validate_success", None)
    if not isinstance(differences, list) or not callable(callback):
        return
    before = len(differences)
    callback(value)
    additions = list(differences[before:])
    del differences[before:]
    for difference in additions:
        relocated = dict(difference)
        location = relocated.get("location")
        if location != "/requestBody":
            if isinstance(location, str) and location.startswith("/"):
                relocated["location"] = prefix + location
            else:
                relocated["location"] = prefix
        ctx.differences.append(relocated)


_ANTHROPIC_EVENT_TYPES = (
    "message_start",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
    "ping",
    "error",
)

_ANTHROPIC_STOP_REASONS = (
    "end_turn",
    "max_tokens",
    "stop_sequence",
    "tool_use",
    "pause_turn",
    "refusal",
    "model_context_window_exceeded",
)


def _validate_anthropic_usage(
    ctx: _Context,
    value: object,
    location: str,
    *,
    start: bool,
) -> None:
    required = ("input_tokens", "output_tokens") if start else ("output_tokens",)
    optional = (
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
        "cache_creation",
        "server_tool_use",
        "service_tier",
        "inference_geo",
    )
    if not _shape(ctx, value, location, required, optional):
        return
    assert isinstance(value, dict)
    for field in (
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ):
        if field in value:
            _typed(
                ctx,
                value[field],
                _join(location, field),
                "non-negative integer",
                lambda item: _is_integer(item) and item >= 0,
            )
    if "cache_creation" in value:
        ctx.profile_at(
            "validate_cache_creation",
            value["cache_creation"],
            _join(location, "cache_creation"),
        )
    if "server_tool_use" in value:
        ctx.profile_at(
            "validate_server_tool_use",
            value["server_tool_use"],
            _join(location, "server_tool_use"),
        )
    if "service_tier" in value:
        _enum(
            ctx,
            value["service_tier"],
            _join(location, "service_tier"),
            ("standard", "priority", "batch"),
        )
    if "inference_geo" in value:
        _string(ctx, value["inference_geo"], _join(location, "inference_geo"))


def _validate_anthropic_message_start(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    required = (
        "id",
        "type",
        "role",
        "model",
        "content",
        "stop_reason",
        "stop_sequence",
        "usage",
    )
    if not _shape(
        ctx,
        value,
        location,
        required,
        ("container", "context_management"),
    ):
        return
    assert isinstance(value, dict)
    for field in ("id", "model"):
        if field in value:
            _string(ctx, value[field], _join(location, field))
    if "type" in value:
        _enum(ctx, value["type"], _join(location, "type"), ("message",))
    if "role" in value:
        _enum(ctx, value["role"], _join(location, "role"), ("assistant",))
    if "content" in value:
        _typed(
            ctx,
            value["content"],
            _join(location, "content"),
            "array",
            lambda item: isinstance(item, list),
        )
    for field in ("stop_reason", "stop_sequence"):
        if field in value and value[field] is not None:
            ctx.add(
                _join(location, field),
                "VALUE_MISMATCH",
                "null in message_start",
                "value differs from required value",
            )
    if "usage" in value:
        _validate_anthropic_usage(ctx, value["usage"], _join(location, "usage"), start=True)
    if "container" in value:
        _typed(
            ctx,
            value["container"],
            _join(location, "container"),
            "object or null",
            lambda item: item is None or isinstance(item, dict),
        )
    if "context_management" in value:
        _typed(
            ctx,
            value["context_management"],
            _join(location, "context_management"),
            "object or null",
            lambda item: item is None or isinstance(item, dict),
        )


def _validate_anthropic_delta(
    ctx: _Context,
    value: object,
    location: str,
) -> tuple[Optional[str], Optional[str]]:
    if not isinstance(value, dict):
        ctx.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
        return None, None
    delta_type = value.get("type")
    if not isinstance(delta_type, str):
        if "type" not in value:
            ctx.add(_join(location, "type"), "MISSING_FIELD", "present", "missing")
        else:
            ctx.add(_join(location, "type"), "TYPE_MISMATCH", "string", _actual_type(delta_type))
        return None, None
    fields_by_type = {
        "text_delta": ("text",),
        "input_json_delta": ("partial_json",),
        "thinking_delta": ("thinking",),
        "signature_delta": ("signature",),
        "citations_delta": ("citation",),
    }
    if delta_type not in fields_by_type:
        _enum(ctx, delta_type, _join(location, "type"), tuple(fields_by_type))
        return None, None
    payload_field = fields_by_type[delta_type][0]
    if not _shape(ctx, value, location, ("type", payload_field)):
        return delta_type, None
    if payload_field == "citation":
        ctx.profile_at(
            "validate_text_citation",
            value.get(payload_field),
            _join(location, payload_field),
        )
        return delta_type, None
    payload = value.get(payload_field)
    if _string(ctx, payload, _join(location, payload_field)):
        return delta_type, payload if isinstance(payload, str) else None
    return delta_type, None


def _validate_anthropic_error(
    ctx: _Context,
    value: Mapping[str, object],
    location: str,
) -> None:
    if "error" not in value:
        return
    error_location = _join(location, "error")
    error = value["error"]
    if not _shape(ctx, error, error_location, ("type", "message")):
        return
    assert isinstance(error, dict)
    for field in ("type", "message"):
        if field in error:
            _string(ctx, error[field], _join(error_location, field))


def validate_anthropic_stream(ctx: _Context, events: Sequence[_SseEvent]) -> None:
    """Validate named Anthropic SSE events and their message/block lifecycle."""

    started = False
    message_delta = False
    message_stopped = False
    error_terminal = False
    undecodable = False
    saw_tool = False
    open_blocks: dict[int, dict[str, object]] = {}
    next_block_index = 0

    for event in events:
        event_location = f"/events/{event.index}"
        if error_terminal or message_stopped:
            ctx.add(
                f"{event_location}/data/type",
                "SEQUENCE",
                "no event after the terminal event",
                "event after terminal",
            )
            continue
        if event.event is None:
            ctx.add(
                _join(event_location, "event"),
                "FRAMING",
                "named SSE event matching data.type",
                "missing event name",
            )

        decoded, value = _decode(ctx, event)
        if not decoded:
            undecodable = True
            continue
        data_location = _join(event_location, "data")
        if not isinstance(value, dict):
            ctx.add(
                data_location,
                "TYPE_MISMATCH",
                "object",
                _actual_type(value),
            )
            continue
        if "type" not in value:
            ctx.add(
                _join(data_location, "type"),
                "MISSING_FIELD",
                "present",
                "missing",
            )
            continue
        event_type = value.get("type")
        if not isinstance(event_type, str):
            if "type" in value:
                ctx.add(
                    _join(data_location, "type"),
                    "TYPE_MISMATCH",
                    "string",
                    _actual_type(event_type),
                )
            continue
        if event.event is not None and event.event != event_type:
            ctx.add(
                _join(event_location, "event"),
                "VALUE_MISMATCH",
                "same value as data.type",
                "different event name",
            )
        if event_type not in _ANTHROPIC_EVENT_TYPES:
            # Anthropic may add event variants; matching named events are extensions.
            continue
        if event_type == "ping":
            _shape(ctx, value, data_location, ("type",))
            continue
        if event_type == "error":
            _shape(ctx, value, data_location, ("type", "error"), ("request_id",))
            if "request_id" in value:
                _string(ctx, value["request_id"], _join(data_location, "request_id"))
            _validate_anthropic_error(ctx, value, data_location)
            error_terminal = True
            continue
        if not started and event_type != "message_start":
            ctx.add(
                _join(data_location, "type"),
                "SEQUENCE",
                "message_start before content or message events",
                "event sequence differs from required lifecycle",
            )
        if event_type in {
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
        } and message_delta:
            ctx.add(
                _join(data_location, "type"),
                "SEQUENCE",
                "all content block events before message_delta",
                "content block event after message_delta",
            )
        if event_type == "message_start":
            _shape(ctx, value, data_location, ("type", "message"))
            if started:
                ctx.add(
                    _join(data_location, "type"),
                    "SEQUENCE",
                    "exactly one initial message_start",
                    "duplicate message_start",
                )
            started = True
            if "message" in value:
                _validate_anthropic_message_start(
                    ctx,
                    value["message"],
                    _join(data_location, "message"),
                )
            continue
        if event_type == "content_block_start":
            _shape(ctx, value, data_location, ("type", "index", "content_block"))
            index = value.get("index")
            index_valid = _integer(ctx, index, _join(data_location, "index"))
            block = value.get("content_block")
            if "content_block" in value:
                validator = ctx.evidence.profile_validator
                callback = getattr(validator, "validate_content_block", None)
                if callable(callback):
                    ctx.profile_at(
                        "validate_content_block",
                        block,
                        _join(data_location, "content_block"),
                    )
                else:
                    _typed(
                        ctx,
                        block,
                        _join(data_location, "content_block"),
                        "object",
                        lambda item: isinstance(item, dict),
                    )
            if index_valid and isinstance(index, int):
                if index != next_block_index:
                    ctx.add(
                        _join(data_location, "index"),
                        "SEQUENCE",
                        f"next contiguous content block index {next_block_index}",
                        "content block index differs from required sequence",
                    )
                else:
                    next_block_index += 1
                if open_blocks:
                    ctx.add(
                        _join(data_location, "type"),
                        "SEQUENCE",
                        "previous content block stopped before the next starts",
                        "overlapping content block lifecycle",
                    )
                if index in open_blocks:
                    ctx.add(
                        _join(data_location, "index"),
                        "SEQUENCE",
                        "one start for each open content block index",
                        "duplicate open block",
                    )
                else:
                    block_type = block.get("type") if isinstance(block, dict) else None
                    open_blocks[index] = {"type": block_type, "fragments": []}
                    saw_tool = saw_tool or block_type == "tool_use"
            continue
        if event_type == "content_block_delta":
            _shape(ctx, value, data_location, ("type", "index", "delta"))
            index = value.get("index")
            index_valid = _integer(ctx, index, _join(data_location, "index"))
            delta_type: Optional[str] = None
            fragment: Optional[str] = None
            if "delta" in value:
                delta_type, fragment = _validate_anthropic_delta(
                    ctx,
                    value["delta"],
                    _join(data_location, "delta"),
                )
            if index_valid and isinstance(index, int):
                block_state = open_blocks.get(index)
                if block_state is None:
                    ctx.add(
                        _join(data_location, "index"),
                        "CORRELATION",
                        "index of an open content block",
                        "index does not identify an open block",
                    )
                elif delta_type is not None:
                    block_type = block_state.get("type")
                    allowed_delta_types = {
                        "text": {"text_delta", "citations_delta"},
                        "thinking": {"thinking_delta", "signature_delta"},
                        "redacted_thinking": set(),
                        "tool_use": {"input_json_delta"},
                    }.get(block_type, set())
                    if delta_type not in allowed_delta_types:
                        ctx.add(
                            f"{data_location}/delta/type",
                            "CORRELATION",
                            "delta type documented for the open content block type",
                            "delta type does not match the open block type",
                        )
                    elif delta_type == "input_json_delta" and fragment is not None:
                        fragments = block_state.get("fragments")
                        if isinstance(fragments, list):
                            fragments.append(fragment)
            continue
        if event_type == "content_block_stop":
            _shape(ctx, value, data_location, ("type", "index"))
            index = value.get("index")
            index_valid = _integer(ctx, index, _join(data_location, "index"))
            if index_valid and isinstance(index, int):
                block_state = open_blocks.pop(index, None)
                if block_state is None:
                    ctx.add(
                        _join(data_location, "index"),
                        "CORRELATION",
                        "index of an open content block",
                        "index does not identify an open block",
                    )
                elif block_state.get("type") == "tool_use":
                    fragments = block_state.get("fragments")
                    if isinstance(fragments, list) and fragments:
                        try:
                            tool_input = strict_json_loads("".join(fragments))
                        except JSON_LOAD_ERRORS:
                            tool_input = None
                        if not isinstance(tool_input, dict):
                            ctx.add(
                                data_location,
                                "INVALID_JSON",
                                "concatenated partial_json forming one JSON object",
                                "invalid JSON",
                            )
            continue
        if event_type == "message_delta":
            _shape(ctx, value, data_location, ("type", "delta", "usage"))
            if open_blocks:
                ctx.add(
                    _join(data_location, "type"),
                    "SEQUENCE",
                    "all content blocks stopped before message_delta",
                    "open content block remains",
                )
            delta = value.get("delta")
            if _shape(
                ctx,
                delta,
                _join(data_location, "delta"),
                (),
                ("stop_reason", "stop_sequence"),
            ):
                assert isinstance(delta, dict)
                valid_reason = False
                if "stop_reason" in delta:
                    valid_reason = _enum(
                        ctx,
                        delta["stop_reason"],
                        f"{data_location}/delta/stop_reason",
                        _ANTHROPIC_STOP_REASONS,
                    )
                if "stop_sequence" in delta:
                    _nullable_string(
                        ctx,
                        delta["stop_sequence"],
                        f"{data_location}/delta/stop_sequence",
                    )
                if (
                    saw_tool
                    and valid_reason
                    and delta.get("stop_reason") != "tool_use"
                ):
                    ctx.add(
                        f"{data_location}/delta/stop_reason",
                        "CORRELATION",
                        "tool_use because a tool_use content block was emitted",
                        "stop reason does not match emitted blocks",
                    )
            if "usage" in value:
                _validate_anthropic_usage(
                    ctx,
                    value["usage"],
                    _join(data_location, "usage"),
                    start=False,
                )
            message_delta = True
            continue
        if event_type == "message_stop":
            _shape(ctx, value, data_location, ("type",))
            if not message_delta and not undecodable:
                ctx.add(
                    _join(data_location, "type"),
                    "SEQUENCE",
                    "message_delta immediately before message_stop",
                    "message_delta missing",
                )
            if open_blocks:
                ctx.add(
                    _join(data_location, "type"),
                    "SEQUENCE",
                    "all content blocks stopped before message_stop",
                    "open content block remains",
                )
            message_stopped = True

    if error_terminal:
        return
    if undecodable:
        return
    if not started and not events:
        ctx.add(
            "/events/0/data/type",
            "SEQUENCE",
            "initial message_start event",
            "empty stream",
        )
        return
    if not message_delta and not message_stopped:
        ctx.add(
            f"/events/{len(events)}/data/type",
            "SEQUENCE",
            "message_delta after all content blocks",
            "event sequence ended early",
        )
    if not message_stopped:
        ctx.add(
            f"/events/{len(events)}",
            "SEQUENCE",
            "terminal message_stop event",
            "event sequence ended early",
        )


def validate_gemini_stream(ctx: _Context, events: Sequence[_SseEvent]) -> None:
    """Validate data-only Gemini SSE frames, with EOF as the terminal marker."""

    if not events:
        ctx.add(
            "/events/0",
            "SEQUENCE",
            "at least one GenerateContentResponse frame before EOF",
            "empty stream",
        )
        return
    identities: dict[str, str] = {}
    for event in events:
        event_location = f"/events/{event.index}"
        if event.event is not None:
            ctx.add(
                _join(event_location, "event"),
                "FRAMING",
                "data-only SSE frame without event name",
                "named SSE event",
            )
        if event.data.strip() == "[DONE]":
            ctx.add(
                _join(event_location, "data"),
                "FRAMING",
                "GenerateContentResponse JSON followed by normal EOF",
                "foreign terminal marker",
            )
            continue
        decoded, value = _decode(ctx, event)
        if not decoded:
            continue
        data_location = _join(event_location, "data")
        if isinstance(value, dict):
            for field in ("responseId", "modelVersion"):
                observed = value.get(field)
                if not isinstance(observed, str):
                    continue
                if field not in identities:
                    identities[field] = observed
                elif identities[field] != observed:
                    ctx.add(
                        _join(data_location, field),
                        "CORRELATION",
                        f"same {field} as earlier stream frames",
                        "identity differs from earlier frame",
                    )
        callback = getattr(ctx.evidence.profile_validator, "validate_success", None)
        if callable(callback):
            _profile_success_at(ctx, value, data_location)
        else:
            _shape(
                ctx,
                value,
                data_location,
                (),
                (
                    "modelVersion",
                    "responseId",
                    "modelStatus",
                    "candidates",
                    "promptFeedback",
                    "usageMetadata",
                ),
            )


_OLLAMA_USAGE_FIELDS = (
    "total_duration",
    "load_duration",
    "prompt_eval_count",
    "prompt_eval_duration",
    "eval_count",
    "eval_duration",
)


def _validate_ollama_message(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    validator = ctx.evidence.profile_validator
    callback = getattr(validator, "validate_message", None)
    if callable(callback):
        ctx.profile_at("validate_message", value, location)
        return
    if not _shape(
        ctx,
        value,
        location,
        ("role", "content"),
        ("thinking", "tool_calls", "images"),
    ):
        return
    assert isinstance(value, dict)
    if "role" in value:
        _enum(ctx, value["role"], _join(location, "role"), ("assistant",))
    for field in ("content", "thinking"):
        if field in value:
            _string(ctx, value[field], _join(location, field))


def _validate_ollama_logprobs(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    if not _typed(ctx, value, location, "array", lambda item: isinstance(item, list)):
        return
    assert isinstance(value, list)
    validator = ctx.evidence.profile_validator
    callback = getattr(validator, "validate_logprob", None)
    if not callable(callback):
        return
    for index, logprob in enumerate(value):
        ctx.profile_at("validate_logprob", logprob, f"{location}/{index}")


def validate_ollama_stream(ctx: _Context, frames: Sequence[_JsonFrame]) -> None:
    """Validate Ollama NDJSON records and their single terminal record."""

    model: Optional[str] = None
    terminal_seen = False
    undecodable = False

    for frame in frames:
        record_location = f"/records/{frame.index}"
        value = frame.value
        if value is None:
            undecodable = True
            continue
        if not isinstance(value, dict):
            ctx.add(record_location, "TYPE_MISMATCH", "object", _actual_type(value))
            undecodable = True
            continue
        if "error" in value:
            _shape(ctx, value, record_location, ("error",))
            if terminal_seen:
                ctx.add(
                    _join(record_location, "error"),
                    "SEQUENCE",
                    "no record after the terminal record",
                    "additional terminal record",
                )
            if _string(ctx, value["error"], _join(record_location, "error")):
                terminal_seen = True
            else:
                # The envelope still terminates the stream even when its payload differs.
                terminal_seen = True
            continue

        done = value.get("done")
        required = ("model", "created_at", "message", "done")
        if isinstance(done, bool) and done is True:
            required = (*required, *_OLLAMA_USAGE_FIELDS)
        optional = ("done_reason", *_OLLAMA_USAGE_FIELDS, "logprobs")
        _shape(ctx, value, record_location, required, optional)
        current_model = value.get("model")
        if "model" in value and _string(
            ctx, current_model, _join(record_location, "model")
        ):
            assert isinstance(current_model, str)
            if model is None:
                model = current_model
            elif current_model != model:
                ctx.add(
                    _join(record_location, "model"),
                    "CORRELATION",
                    "same model in every stream record",
                    "model differs from earlier record",
                )
        if "created_at" in value:
            _string(ctx, value["created_at"], _join(record_location, "created_at"))
        if "message" in value:
            _validate_ollama_message(
                ctx,
                value["message"],
                _join(record_location, "message"),
            )
        valid_done = "done" in value and _typed(
            ctx,
            done,
            _join(record_location, "done"),
            "boolean",
            lambda item: isinstance(item, bool),
        )
        if terminal_seen:
            ctx.add(
                _join(record_location, "done"),
                "SEQUENCE",
                "no record after the terminal record",
                "additional stream record",
            )
        if valid_done and done is True:
            terminal_seen = True
        elif valid_done and done is False:
            for field in _OLLAMA_USAGE_FIELDS:
                if field in value:
                    ctx.add(
                        _join(record_location, field),
                        "SEQUENCE",
                        f"{field} only in the final done:true record",
                        "final-only field in a non-final record",
                    )
        elif not valid_done:
            undecodable = True
        if "done_reason" in value:
            _string(ctx, value["done_reason"], _join(record_location, "done_reason"))
        for field in _OLLAMA_USAGE_FIELDS:
            if field in value:
                _integer(ctx, value[field], _join(record_location, field))
        if "logprobs" in value:
            _validate_ollama_logprobs(
                ctx,
                value["logprobs"],
                _join(record_location, "logprobs"),
            )

    if not terminal_seen and not undecodable:
        ctx.add(
            f"/records/{len(frames)}",
            "SEQUENCE",
            "one final done:true record or error record",
            "stream ended without a terminal record",
        )
