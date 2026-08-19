"""Validate OpenAI Responses API server-sent event streams."""

from __future__ import annotations

from typing import Dict, Mapping, Optional, Sequence, Tuple

from model_doctor_protocol_streaming import _Context, _SseEvent, _decode_event_json


_RESPONSE_EVENTS = {
    "response.created",
    "response.in_progress",
    "response.queued",
    "response.completed",
    "response.incomplete",
    "response.failed",
}
_NORMAL_TERMINALS = {
    "response.completed",
    "response.incomplete",
    "response.failed",
}
_ITEM_EVENTS = {
    "response.output_item.added",
    "response.output_item.done",
}
_PART_EVENTS = {
    "response.content_part.added",
    "response.content_part.done",
}
_OUTPUT_TEXT_EVENTS = {
    "response.output_text.delta": "delta",
    "response.output_text.done": "text",
}
_REFUSAL_EVENTS = {
    "response.refusal.delta": "delta",
    "response.refusal.done": "refusal",
}
_FUNCTION_ARGUMENT_EVENTS = {
    "response.function_call_arguments.delta": "delta",
    "response.function_call_arguments.done": "arguments",
}
_CUSTOM_TOOL_INPUT_EVENTS = {
    "response.custom_tool_call_input.delta": "delta",
    "response.custom_tool_call_input.done": "input",
}
_REASONING_SUMMARY_TEXT_EVENTS = {
    "response.reasoning_summary_text.delta": "delta",
    "response.reasoning_summary_text.done": "text",
}
_REASONING_TEXT_EVENTS = {
    "response.reasoning_text.delta": "delta",
    "response.reasoning_text.done": "text",
}
_ANNOTATION_EVENTS = {"response.output_text.annotation.added"}
_INDEXED_PART_EVENTS = {
    "response.reasoning_summary_part.added",
    "response.reasoning_summary_part.done",
}
_CALL_STATUS_EVENTS = {
    "response.file_search_call.in_progress",
    "response.file_search_call.searching",
    "response.file_search_call.completed",
    "response.web_search_call.in_progress",
    "response.web_search_call.searching",
    "response.web_search_call.completed",
    "response.code_interpreter_call.in_progress",
    "response.code_interpreter_call.interpreting",
    "response.code_interpreter_call.completed",
    "response.image_generation_call.in_progress",
    "response.image_generation_call.generating",
    "response.image_generation_call.completed",
    "response.mcp_call.in_progress",
    "response.mcp_call.completed",
    "response.mcp_call.failed",
    "response.mcp_list_tools.in_progress",
    "response.mcp_list_tools.completed",
    "response.mcp_list_tools.failed",
}
_INDEXED_VALUE_EVENTS = {
    "response.code_interpreter_call_code.delta": "delta",
    "response.code_interpreter_call_code.done": "code",
    "response.mcp_call_arguments.delta": "delta",
    "response.mcp_call_arguments.done": "arguments",
}
_AUDIO_EVENTS = {
    "response.audio.delta": "delta",
    "response.audio.done": None,
    "response.audio.transcript.delta": "delta",
    "response.audio.transcript.done": None,
}
_IMAGE_EVENTS = {"response.image_generation_call.partial_image"}
_SHELL_COMMAND_EVENTS = {
    "response.shell_call_command.added": "command",
    "response.shell_call_command.delta": "delta",
    "response.shell_call_command.done": "command",
}
_SHELL_OUTPUT_EVENTS = {
    "response.shell_call_output_content.delta",
    "response.shell_call_output_content.done",
}
_KNOWN_EVENTS = (
    _RESPONSE_EVENTS
    | _ITEM_EVENTS
    | _PART_EVENTS
    | set(_OUTPUT_TEXT_EVENTS)
    | set(_REFUSAL_EVENTS)
    | set(_FUNCTION_ARGUMENT_EVENTS)
    | set(_CUSTOM_TOOL_INPUT_EVENTS)
    | set(_REASONING_SUMMARY_TEXT_EVENTS)
    | set(_REASONING_TEXT_EVENTS)
    | _ANNOTATION_EVENTS
    | _INDEXED_PART_EVENTS
    | _CALL_STATUS_EVENTS
    | set(_INDEXED_VALUE_EVENTS)
    | set(_AUDIO_EVENTS)
    | _IMAGE_EVENTS
    | set(_SHELL_COMMAND_EVENTS)
    | _SHELL_OUTPUT_EVENTS
    | {"error"}
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
    return type(value).__name__


def _is_integer(value: object) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _closed_shape(
    ctx: _Context,
    value: object,
    location: str,
    required: Sequence[str],
    optional: Sequence[str] = (),
) -> Optional[dict]:
    if not isinstance(value, dict):
        ctx.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
        return None

    allowed = set(required) | set(optional)
    for field in required:
        if field not in value:
            ctx.add(f"{location}/{field}", "MISSING_FIELD", "present", "missing")
    for field in value:
        if field not in allowed:
            ctx.add(
                f"{location}/{field}",
                "UNEXPECTED_FIELD",
                "absent",
                "present",
            )
    return value


def _string(ctx: _Context, value: object, location: str) -> bool:
    if isinstance(value, str):
        return True
    ctx.add(location, "TYPE_MISMATCH", "string", _actual_type(value))
    return False


def _integer(ctx: _Context, value: object, location: str) -> bool:
    if _is_integer(value):
        return True
    ctx.add(location, "TYPE_MISMATCH", "integer", _actual_type(value))
    return False


def _profile(
    ctx: _Context,
    method_name: str,
    value: object,
    prefix: str,
) -> None:
    """Use the non-stream profile facade when supplied by the stream core."""

    profile = getattr(ctx, "profile", None)
    if callable(profile):
        profile(method_name, value, prefix)


def _validate_response(
    ctx: _Context,
    payload: Mapping[str, object],
    location: str,
) -> Optional[str]:
    response = payload.get("response")
    response_location = f"{location}/response"
    if not isinstance(response, dict):
        if "response" in payload:
            ctx.add(
                response_location,
                "TYPE_MISMATCH",
                "object",
                _actual_type(response),
            )
        return None

    # Streaming snapshots legitimately carry usage:null before completion.
    # The shared non-stream profile treats usage as an optional object, so omit
    # that null solely for reuse of its otherwise identical response schema.
    profile_value = dict(response)
    if profile_value.get("usage") is None:
        profile_value.pop("usage", None)
    _profile(ctx, "validate_success", profile_value, response_location)

    response_id = response.get("id")
    if "id" not in response:
        # The profile reports this too; this guard only keeps state handling safe.
        return None
    if not isinstance(response_id, str):
        return None
    return response_id


def _validate_sequence(
    ctx: _Context,
    payload: Mapping[str, object],
    event: _SseEvent,
    previous: Optional[int],
) -> Optional[int]:
    if "sequence_number" not in payload:
        return previous
    value = payload["sequence_number"]
    location = f"/events/{event.index}/data/sequence_number"
    if not _integer(ctx, value, location):
        return previous
    assert isinstance(value, int)
    if previous is not None and value <= previous:
        ctx.add(
            location,
            "SEQUENCE",
            "sequence_number strictly greater than the preceding event",
            "event sequence differs from required lifecycle",
        )
    return value


def _validate_string_list(
    ctx: _Context,
    value: object,
    location: str,
    item_method: Optional[str] = None,
) -> None:
    if not isinstance(value, list):
        ctx.add(location, "TYPE_MISMATCH", "array", _actual_type(value))
        return
    if item_method is None:
        return
    for index, item in enumerate(value):
        ctx.profile_at(item_method, item, f"{location}/{index}")


def _validate_stream_output_item(
    ctx: _Context,
    item: object,
    location: str,
    *,
    provisional: bool,
) -> None:
    if not isinstance(item, dict):
        ctx.add(location, "TYPE_MISMATCH", "object", _actual_type(item))
        return
    item_type = item.get("type")
    if item_type == "local_shell_call":
        if "id" not in item:
            ctx.add(f"{location}/id", "MISSING_FIELD", "present", "missing")
        elif isinstance(item.get("id"), str):
            pass
        else:
            ctx.add(f"{location}/id", "TYPE_MISMATCH", "string", _actual_type(item.get("id")))
        return
    profile_item = item
    if (
        provisional
        and item_type == "function_call"
        and item.get("arguments") == ""
    ):
        profile_item = dict(item)
        profile_item["arguments"] = "{}"
    ctx.profile_at("validate_output_item", profile_item, location)


def _item_state(item: Mapping[str, object]) -> dict[str, object]:
    return {
        "id": item.get("id"),
        "type": item.get("type"),
        "call_id": item.get("call_id"),
        "name": item.get("name"),
    }


def _correlate_item_reference(
    ctx: _Context,
    event: _SseEvent,
    payload: Mapping[str, object],
    open_items: Mapping[int, Mapping[str, object]],
) -> Optional[Tuple[int, Mapping[str, object]]]:
    base = f"/events/{event.index}/data"
    output_index = payload.get("output_index")
    item_id = payload.get("item_id")
    if not _is_integer(output_index):
        return None
    assert isinstance(output_index, int)
    state = open_items.get(output_index)
    if state is None:
        ctx.add(
            f"{base}/output_index",
            "CORRELATION",
            "index of an open output item",
            "correlated values differ",
        )
        return None
    expected_id = state.get("id")
    if isinstance(item_id, str) and isinstance(expected_id, str) and item_id != expected_id:
        ctx.add(
            f"{base}/item_id",
            "CORRELATION",
            "id of the referenced output item",
            "correlated values differ",
        )
        return None
    return output_index, state


def _correlate_part(
    ctx: _Context,
    event: _SseEvent,
    payload: Mapping[str, object],
    open_items: Mapping[int, Mapping[str, object]],
    open_parts: Mapping[Tuple[int, int], Mapping[str, object]],
) -> Optional[Tuple[int, int]]:
    base = f"/events/{event.index}/data"
    output_index = payload.get("output_index")
    content_index = payload.get("content_index")
    item_id = payload.get("item_id")
    if not _is_integer(output_index) or not _is_integer(content_index):
        return None
    assert isinstance(output_index, int) and isinstance(content_index, int)

    item_state = open_items.get(output_index)
    if item_state is None:
        ctx.add(
            f"{base}/output_index",
            "CORRELATION",
            "index of an open output item",
            "correlated values differ",
        )
        return None
    expected_id = item_state.get("id")
    if (
        isinstance(item_id, str)
        and isinstance(expected_id, str)
        and item_id != expected_id
    ):
        ctx.add(
            f"{base}/item_id",
            "CORRELATION",
            "id of the referenced output item",
            "correlated values differ",
        )
        return None

    key = (output_index, content_index)
    if key not in open_parts:
        ctx.add(
            f"{base}/content_index",
            "CORRELATION",
            "index of an open content part",
            "correlated values differ",
        )
        return None
    part_state = open_parts[key]
    if isinstance(item_id, str) and item_id != part_state.get("item_id"):
        ctx.add(
            f"{base}/item_id",
            "CORRELATION",
            "id of the referenced content part owner",
            "correlated values differ",
        )
        return None
    return key


def _validate_error_event(
    ctx: _Context,
    payload: object,
    event: _SseEvent,
) -> None:
    base = f"/events/{event.index}/data"
    value = _closed_shape(
        ctx,
        payload,
        base,
        ("type", "code", "message", "param", "sequence_number"),
    )
    if value is None:
        return
    for field in ("code", "message"):
        if field in value:
            _string(ctx, value[field], f"{base}/{field}")
    if "param" in value and value["param"] is not None:
        _string(ctx, value["param"], f"{base}/param")


def _validate_response_event_state(
    ctx: _Context,
    event_type: str,
    response: object,
    location: str,
) -> None:
    if not isinstance(response, dict):
        return
    expected_status = {
        "response.created": "in_progress",
        "response.in_progress": "in_progress",
        "response.queued": "queued",
        "response.completed": "completed",
        "response.incomplete": "incomplete",
        "response.failed": "failed",
    }[event_type]
    if "status" in response and response.get("status") != expected_status:
        ctx.add(
            f"{location}/status",
            "CORRELATION",
            f"{expected_status} status for {event_type}",
            "response status does not match the event type",
        )
    if event_type == "response.failed":
        if "error" in response and not isinstance(response.get("error"), dict):
            ctx.add(
                f"{location}/error",
                "CORRELATION",
                "non-null error object for response.failed",
                "response error does not match the terminal event",
            )
    elif "error" in response and response.get("error") is not None:
        ctx.add(
            f"{location}/error",
            "CORRELATION",
            f"null error for {event_type}",
            "response error does not match the event type",
        )
    if event_type == "response.incomplete":
        if "incomplete_details" in response and not isinstance(
            response.get("incomplete_details"), dict
        ):
            ctx.add(
                f"{location}/incomplete_details",
                "CORRELATION",
                "non-null incomplete_details object for response.incomplete",
                "response incomplete_details do not match the terminal event",
            )
    elif (
        "incomplete_details" in response
        and response.get("incomplete_details") is not None
    ):
        ctx.add(
            f"{location}/incomplete_details",
            "CORRELATION",
            f"null incomplete_details for {event_type}",
            "response incomplete_details do not match the event type",
        )


def _correlate_terminal_output(
    ctx: _Context,
    response: object,
    location: str,
    items: Mapping[int, Mapping[str, object]],
) -> None:
    if not isinstance(response, dict) or not isinstance(response.get("output"), list):
        return
    output = response["output"]
    assert isinstance(output, list)
    for output_index, item in enumerate(output):
        state = items.get(output_index)
        if state is None or not isinstance(item, dict):
            continue
        for field in ("id", "call_id"):
            expected = state.get(field)
            actual = item.get(field)
            if isinstance(expected, str) and isinstance(actual, str) and actual != expected:
                ctx.add(
                    f"{location}/output/{output_index}/{field}",
                    "CORRELATION",
                    f"same {field} as the streamed output item",
                    "correlated values differ",
                )


def _validate_shell_delta(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    payload = _closed_shape(ctx, value, location, (), ("stdout", "stderr"))
    if payload is None:
        return
    for field in ("stdout", "stderr"):
        if field in payload:
            _string(ctx, payload[field], f"{location}/{field}")


def _validate_shell_outcome(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    if not isinstance(value, dict):
        ctx.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
        return
    outcome_type = value.get("type")
    if outcome_type == "timeout":
        _closed_shape(ctx, value, location, ("type",))
    elif outcome_type == "exit":
        payload = _closed_shape(ctx, value, location, ("type", "exit_code"))
        if payload is not None and "exit_code" in payload:
            _integer(ctx, payload["exit_code"], f"{location}/exit_code")
    else:
        if "type" not in value:
            ctx.add(f"{location}/type", "MISSING_FIELD", "present", "missing")
        else:
            ctx.add(
                f"{location}/type",
                "ENUM_MISMATCH",
                "timeout or exit",
                "value outside allowed enum",
            )


def _validate_shell_output(
    ctx: _Context,
    value: object,
    location: str,
) -> None:
    if not isinstance(value, list):
        ctx.add(location, "TYPE_MISMATCH", "array", _actual_type(value))
        return
    for index, item in enumerate(value):
        item_location = f"{location}/{index}"
        payload = _closed_shape(
            ctx,
            item,
            item_location,
            ("stdout", "stderr", "outcome"),
            ("created_by",),
        )
        if payload is None:
            continue
        for field in ("stdout", "stderr", "created_by"):
            if field in payload:
                _string(ctx, payload[field], f"{item_location}/{field}")
        if "outcome" in payload:
            _validate_shell_outcome(
                ctx,
                payload["outcome"],
                f"{item_location}/outcome",
            )


def validate_responses_stream(ctx: _Context, events: list[_SseEvent]) -> None:
    """Validate one parsed Responses SSE stream and append exact differences."""

    response_id: Optional[str] = None
    open_items: Dict[int, Mapping[str, object]] = {}
    items: Dict[int, Mapping[str, object]] = {}
    open_parts: Dict[Tuple[int, int], Mapping[str, object]] = {}
    open_summary_parts: set[Tuple[int, int]] = set()
    shell_commands: Dict[Tuple[int, int], str] = {}
    completed_shell_commands: set[Tuple[int, int]] = set()
    saw_created = False
    saw_terminal = False
    last_sequence: Optional[int] = None

    for event in events:
        base = f"/events/{event.index}"
        if event.event is None:
            ctx.add(
                f"{base}/event",
                "FRAMING",
                "named SSE event",
                "stream framing differs from required format",
            )

        decoded = _decode_event_json(ctx, event)
        if decoded is None:
            continue
        if not isinstance(decoded, dict):
            ctx.add(
                f"{base}/data",
                "TYPE_MISMATCH",
                "object",
                _actual_type(decoded),
            )
            continue
        payload = decoded

        event_type = payload.get("type")
        type_location = f"{base}/data/type"
        if "type" not in payload:
            ctx.add(type_location, "MISSING_FIELD", "present", "missing")
            type_is_string = False
        else:
            type_is_string = _string(ctx, event_type, type_location)
        if type_is_string and event.event is not None and event.event != event_type:
            ctx.add(
                f"{base}/event",
                "VALUE_MISMATCH",
                "same value as data.type",
                "value differs from required value",
            )
        if type_is_string and event_type not in _KNOWN_EVENTS:
            ctx.add(
                type_location,
                "ENUM_MISMATCH",
                "documented Responses streaming event type",
                "value outside allowed enum",
            )

        last_sequence = _validate_sequence(ctx, payload, event, last_sequence)
        if not isinstance(event_type, str) or event_type not in _KNOWN_EVENTS:
            if "sequence_number" not in payload:
                ctx.add(
                    f"{base}/data/sequence_number",
                    "MISSING_FIELD",
                    "present",
                    "missing",
                )
            continue

        if saw_terminal:
            ctx.add(
                type_location,
                "SEQUENCE",
                "no event after a terminal event",
                "event sequence differs from required lifecycle",
            )

        if event_type == "error":
            # Re-run against the exact error event shape. The broad envelope
            # above deliberately permits fields used by all event variants.
            _validate_error_event(ctx, decoded, event)
            saw_terminal = True
            if event.index != len(events) - 1:
                ctx.add(
                    type_location,
                    "SEQUENCE",
                    "terminal error is the final event",
                    "event sequence differs from required lifecycle",
                )
            continue

        if event_type in _RESPONSE_EVENTS:
            exact = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                ("type", "response", "sequence_number"),
            )
            current_id = _validate_response(ctx, payload, f"{base}/data")
            response = payload.get("response")
            _validate_response_event_state(
                ctx,
                event_type,
                response,
                f"{base}/data/response",
            )
            if event_type == "response.created":
                if event.index != 0 or saw_created:
                    ctx.add(
                        type_location,
                        "SEQUENCE",
                        "one response.created event at stream start",
                        "event sequence differs from required lifecycle",
                    )
                saw_created = True
                if current_id is not None:
                    response_id = current_id
            elif not saw_created:
                ctx.add(
                    type_location,
                    "SEQUENCE",
                    "response.created precedes response lifecycle events",
                    "event sequence differs from required lifecycle",
                )
            if (
                current_id is not None
                and response_id is not None
                and current_id != response_id
            ):
                ctx.add(
                    f"{base}/data/response/id",
                    "CORRELATION",
                    "same response.id as response.created",
                    "correlated values differ",
                )
            if exact is None:
                continue
            if event_type in _NORMAL_TERMINALS:
                if open_items or open_parts or open_summary_parts or shell_commands:
                    ctx.add(
                        type_location,
                        "SEQUENCE",
                        "all output items and content parts closed before terminal event",
                        "event sequence differs from required lifecycle",
                    )
                _correlate_terminal_output(
                    ctx,
                    response,
                    f"{base}/data/response",
                    items,
                )
                if saw_terminal:
                    ctx.add(
                        type_location,
                        "SEQUENCE",
                        "exactly one terminal event",
                        "event sequence differs from required lifecycle",
                    )
                saw_terminal = True
                if event.index != len(events) - 1:
                    ctx.add(
                        type_location,
                        "SEQUENCE",
                        "terminal event is the final event",
                        "event sequence differs from required lifecycle",
                    )
            continue

        if event_type in _ITEM_EVENTS:
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                ("type", "output_index", "item", "sequence_number"),
            )
            if payload is None:
                continue
            output_index = payload.get("output_index")
            if "output_index" in payload:
                _integer(ctx, output_index, f"{base}/data/output_index")
            item = payload.get("item")
            if "item" in payload:
                _validate_stream_output_item(
                    ctx,
                    item,
                    f"{base}/data/item",
                    provisional=event_type == "response.output_item.added",
                )
            item_id = item.get("id") if isinstance(item, dict) else None
            if isinstance(item, dict) and "id" not in item:
                ctx.add(
                    f"{base}/data/item/id",
                    "MISSING_FIELD",
                    "present",
                    "missing",
                )
            elif item_id is not None:
                _string(ctx, item_id, f"{base}/data/item/id")

            if _is_integer(output_index) and isinstance(item_id, str):
                assert isinstance(output_index, int)
                if event_type == "response.output_item.added":
                    if output_index in open_items:
                        ctx.add(
                            f"{base}/data/output_index",
                            "SEQUENCE",
                            "output index opened once",
                            "event sequence differs from required lifecycle",
                        )
                    else:
                        assert isinstance(item, dict)
                        state = _item_state(item)
                        open_items[output_index] = state
                        items[output_index] = state
                else:
                    state = open_items.get(output_index)
                    if state is None:
                        ctx.add(
                            f"{base}/data/output_index",
                            "CORRELATION",
                            "index of an open output item",
                            "correlated values differ",
                        )
                    elif item_id != state.get("id"):
                        ctx.add(
                            f"{base}/data/item/id",
                            "CORRELATION",
                            "same item.id as output_item.added",
                            "correlated values differ",
                        )
                    else:
                        if isinstance(item, dict):
                            for field in ("call_id", "name"):
                                expected = state.get(field)
                                actual = item.get(field)
                                if (
                                    isinstance(expected, str)
                                    and isinstance(actual, str)
                                    and actual != expected
                                ):
                                    ctx.add(
                                        f"{base}/data/item/{field}",
                                        "CORRELATION",
                                        f"same {field} as output_item.added",
                                        "correlated values differ",
                                    )
                        if any(key[0] == output_index for key in open_parts):
                            ctx.add(
                                type_location,
                                "SEQUENCE",
                                "content parts closed before output item",
                                "event sequence differs from required lifecycle",
                            )
                        open_items.pop(output_index, None)
            continue

        if event_type in _PART_EVENTS:
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "item_id",
                    "output_index",
                    "content_index",
                    "part",
                    "sequence_number",
                ),
            )
            if payload is None:
                continue
            for field in ("item_id",):
                if field in payload:
                    _string(ctx, payload[field], f"{base}/data/{field}")
            for field in ("output_index", "content_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            part = payload.get("part")
            if "part" in payload:
                if isinstance(part, dict):
                    ctx.profile_at(
                        "validate_message_content",
                        part,
                        f"{base}/data/part",
                    )
                else:
                    ctx.add(
                        f"{base}/data/part",
                        "TYPE_MISMATCH",
                        "object",
                        _actual_type(part),
                    )

            output_index = payload.get("output_index")
            content_index = payload.get("content_index")
            item_id = payload.get("item_id")
            if (
                _is_integer(output_index)
                and _is_integer(content_index)
                and isinstance(item_id, str)
            ):
                assert isinstance(output_index, int)
                assert isinstance(content_index, int)
                state = open_items.get(output_index)
                if state is None:
                    ctx.add(
                        f"{base}/data/output_index",
                        "CORRELATION",
                        "index of an open output item",
                        "correlated values differ",
                    )
                elif item_id != state.get("id"):
                    ctx.add(
                        f"{base}/data/item_id",
                        "CORRELATION",
                        "id of the referenced output item",
                        "correlated values differ",
                    )
                else:
                    key = (output_index, content_index)
                    if event_type == "response.content_part.added":
                        if key in open_parts:
                            ctx.add(
                                f"{base}/data/content_index",
                                "SEQUENCE",
                                "content index opened once",
                                "event sequence differs from required lifecycle",
                            )
                        else:
                            open_parts[key] = {
                                "item_id": item_id,
                                "type": part.get("type")
                                if isinstance(part, dict)
                                else None,
                            }
                    else:
                        correlated = _correlate_part(
                            ctx,
                            event,
                            payload,
                            open_items,
                            open_parts,
                        )
                        if correlated is not None:
                            open_parts.pop(correlated, None)
            continue

        if event_type in _OUTPUT_TEXT_EVENTS:
            value_field = _OUTPUT_TEXT_EVENTS[event_type]
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "item_id",
                    "output_index",
                    "content_index",
                    value_field,
                    "logprobs",
                    "sequence_number",
                ),
            )
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            for field in ("output_index", "content_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if value_field in payload:
                _string(ctx, payload[value_field], f"{base}/data/{value_field}")
            if "logprobs" in payload:
                _validate_string_list(
                    ctx,
                    payload["logprobs"],
                    f"{base}/data/logprobs",
                    "validate_response_logprob",
                )
            _correlate_part(ctx, event, payload, open_items, open_parts)
            continue

        if event_type in _ANNOTATION_EVENTS:
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "item_id",
                    "output_index",
                    "content_index",
                    "annotation_index",
                    "annotation",
                    "sequence_number",
                ),
            )
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            for field in ("output_index", "content_index", "annotation_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if "annotation" in payload and payload["annotation"] is not None:
                ctx.profile_at(
                    "validate_response_annotation",
                    payload["annotation"],
                    f"{base}/data/annotation",
                )
            correlated = _correlate_part(
                ctx, event, payload, open_items, open_parts
            )
            if (
                correlated is not None
                and open_parts[correlated].get("type") != "output_text"
            ):
                ctx.add(
                    f"{base}/data/content_index",
                    "CORRELATION",
                    "index of an open output_text content part",
                    "referenced content part has another type",
                )
            continue

        if event_type in _REFUSAL_EVENTS:
            value_field = _REFUSAL_EVENTS[event_type]
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "item_id",
                    "output_index",
                    "content_index",
                    value_field,
                    "sequence_number",
                ),
            )
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            for field in ("output_index", "content_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if value_field in payload:
                _string(ctx, payload[value_field], f"{base}/data/{value_field}")
            _correlate_part(ctx, event, payload, open_items, open_parts)
            continue

        if event_type in _FUNCTION_ARGUMENT_EVENTS | _CUSTOM_TOOL_INPUT_EVENTS:
            value_field = (
                _FUNCTION_ARGUMENT_EVENTS.get(event_type)
                or _CUSTOM_TOOL_INPUT_EVENTS[event_type]
            )
            required = [
                "type",
                "item_id",
                "output_index",
                value_field,
                "sequence_number",
            ]
            if event_type == "response.function_call_arguments.done":
                required.append("name")
            payload = _closed_shape(ctx, decoded, f"{base}/data", tuple(required))
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            if "output_index" in payload:
                _integer(ctx, payload["output_index"], f"{base}/data/output_index")
            if value_field in payload:
                _string(ctx, payload[value_field], f"{base}/data/{value_field}")
            if "name" in payload:
                _string(ctx, payload["name"], f"{base}/data/name")
            correlated = _correlate_item_reference(ctx, event, payload, open_items)
            if correlated is not None:
                _, state = correlated
                expected_item_type = (
                    "function_call"
                    if event_type in _FUNCTION_ARGUMENT_EVENTS
                    else "custom_tool_call"
                )
                if state.get("type") != expected_item_type:
                    ctx.add(
                        f"{base}/data/item_id",
                        "CORRELATION",
                        f"id of a {expected_item_type} output item",
                        "referenced item has another type",
                    )
                if event_type in _FUNCTION_ARGUMENT_EVENTS:
                    if (
                        "name" in payload
                        and isinstance(state.get("name"), str)
                        and payload.get("name") != state.get("name")
                    ):
                        ctx.add(
                            f"{base}/data/name",
                            "CORRELATION",
                            "same function name as output_item.added",
                            "correlated values differ",
                        )
            continue

        if event_type in _REASONING_SUMMARY_TEXT_EVENTS | _REASONING_TEXT_EVENTS:
            summary = event_type in _REASONING_SUMMARY_TEXT_EVENTS
            value_field = (
                _REASONING_SUMMARY_TEXT_EVENTS.get(event_type)
                or _REASONING_TEXT_EVENTS[event_type]
            )
            index_field = "summary_index" if summary else "content_index"
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "item_id",
                    "output_index",
                    index_field,
                    value_field,
                    "sequence_number",
                ),
            )
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            for field in ("output_index", index_field):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if value_field in payload:
                _string(ctx, payload[value_field], f"{base}/data/{value_field}")
            correlated = _correlate_item_reference(
                ctx, event, payload, open_items
            )
            if correlated is not None and correlated[1].get("type") != "reasoning":
                ctx.add(
                    f"{base}/data/item_id",
                    "CORRELATION",
                    "id of a reasoning output item",
                    "referenced item has another type",
                )
            continue

        if event_type in _INDEXED_PART_EVENTS:
            required = (
                "type",
                "item_id",
                "output_index",
                "summary_index",
                "part",
                "sequence_number",
            )
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                required,
                ("status",)
                if event_type == "response.reasoning_summary_part.done"
                else (),
            )
            if payload is not None:
                for field in ("output_index", "summary_index"):
                    if field in payload:
                        _integer(ctx, payload[field], f"{base}/data/{field}")
                if "item_id" in payload:
                    _string(ctx, payload["item_id"], f"{base}/data/item_id")
                if "part" in payload:
                    part = _closed_shape(
                        ctx,
                        payload["part"],
                        f"{base}/data/part",
                        ("type", "text"),
                    )
                    if part is not None:
                        if "type" in part and part["type"] != "summary_text":
                            ctx.add(
                                f"{base}/data/part/type",
                                "ENUM_MISMATCH",
                                "summary_text",
                                "value outside allowed enum",
                            )
                        if "text" in part:
                            _string(ctx, part["text"], f"{base}/data/part/text")
                if "status" in payload and payload["status"] != "incomplete":
                    ctx.add(
                        f"{base}/data/status",
                        "ENUM_MISMATCH",
                        "incomplete when status is present",
                        "value outside allowed enum",
                    )
                correlated = _correlate_item_reference(
                    ctx,
                    event,
                    payload,
                    open_items,
                )
                if (
                    correlated is not None
                    and correlated[1].get("type") != "reasoning"
                ):
                    ctx.add(
                        f"{base}/data/item_id",
                        "CORRELATION",
                        "id of a reasoning output item",
                        "referenced item has another type",
                    )
                output_index = payload.get("output_index")
                summary_index = payload.get("summary_index")
                if (
                    correlated is not None
                    and _is_integer(output_index)
                    and _is_integer(summary_index)
                ):
                    assert isinstance(output_index, int)
                    assert isinstance(summary_index, int)
                    key = (output_index, summary_index)
                    if event_type.endswith(".added"):
                        if key in open_summary_parts:
                            ctx.add(
                                f"{base}/data/summary_index",
                                "SEQUENCE",
                                "summary index opened once",
                                "event sequence differs from required lifecycle",
                            )
                        open_summary_parts.add(key)
                    elif key not in open_summary_parts:
                        ctx.add(
                            f"{base}/data/summary_index",
                            "CORRELATION",
                            "index of an open reasoning summary part",
                            "correlated values differ",
                        )
                    else:
                        open_summary_parts.remove(key)
            continue

        if event_type in _CALL_STATUS_EVENTS:
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                ("type", "output_index", "item_id", "sequence_number"),
            )
            if payload is not None:
                if "output_index" in payload:
                    _integer(ctx, payload["output_index"], f"{base}/data/output_index")
                if "item_id" in payload:
                    _string(ctx, payload["item_id"], f"{base}/data/item_id")
                _correlate_item_reference(ctx, event, payload, open_items)
            continue

        if event_type in _INDEXED_VALUE_EVENTS:
            value_field = _INDEXED_VALUE_EVENTS[event_type]
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                ("type", "output_index", "item_id", value_field, "sequence_number"),
            )
            if payload is not None:
                if "output_index" in payload:
                    _integer(ctx, payload["output_index"], f"{base}/data/output_index")
                if "item_id" in payload:
                    _string(ctx, payload["item_id"], f"{base}/data/item_id")
                if value_field in payload:
                    _string(
                        ctx,
                        payload[value_field],
                        f"{base}/data/{value_field}",
                    )
                _correlate_item_reference(ctx, event, payload, open_items)
            continue

        if event_type in _IMAGE_EVENTS:
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "output_index",
                    "item_id",
                    "partial_image_index",
                    "partial_image_b64",
                    "sequence_number",
                ),
                ("background", "output_format", "quality", "size"),
            )
            if payload is not None:
                if "item_id" in payload:
                    _string(ctx, payload["item_id"], f"{base}/data/item_id")
                for field in ("output_index", "partial_image_index"):
                    if field in payload:
                        _integer(ctx, payload[field], f"{base}/data/{field}")
                if "partial_image_b64" in payload:
                    _string(
                        ctx,
                        payload["partial_image_b64"],
                        f"{base}/data/partial_image_b64",
                    )
                for field in ("background", "output_format", "quality", "size"):
                    if field in payload:
                        _string(ctx, payload[field], f"{base}/data/{field}")
                _correlate_item_reference(ctx, event, payload, open_items)
            continue

        if event_type in _AUDIO_EVENTS:
            value_field = _AUDIO_EVENTS[event_type]
            required = ["type", "sequence_number"]
            if value_field is not None:
                required.append(value_field)
            if event_type != "response.audio.delta":
                required.append("response_id")
            payload = _closed_shape(ctx, decoded, f"{base}/data", tuple(required))
            if payload is not None:
                if value_field is not None and value_field in payload:
                    _string(ctx, payload[value_field], f"{base}/data/{value_field}")
                if "response_id" in payload:
                    _string(ctx, payload["response_id"], f"{base}/data/response_id")
            continue

        if event_type in _SHELL_COMMAND_EVENTS:
            value_field = _SHELL_COMMAND_EVENTS[event_type]
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "sequence_number",
                    "output_index",
                    "command_index",
                    value_field,
                ),
                ("obfuscation",)
                if event_type == "response.shell_call_command.delta"
                else (),
            )
            if payload is None:
                continue
            for field in ("output_index", "command_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if value_field in payload:
                _string(ctx, payload[value_field], f"{base}/data/{value_field}")
            if "obfuscation" in payload:
                _string(ctx, payload["obfuscation"], f"{base}/data/obfuscation")
            output_index = payload.get("output_index")
            command_index = payload.get("command_index")
            if _is_integer(output_index):
                assert isinstance(output_index, int)
                item_state = open_items.get(output_index)
                if item_state is None:
                    ctx.add(
                        f"{base}/data/output_index",
                        "CORRELATION",
                        "index of an open shell_call output item",
                        "correlated values differ",
                    )
                elif item_state.get("type") != "shell_call":
                    ctx.add(
                        f"{base}/data/output_index",
                        "CORRELATION",
                        "index of an open shell_call output item",
                        "referenced item has another type",
                    )
            if _is_integer(output_index) and _is_integer(command_index):
                assert isinstance(output_index, int)
                assert isinstance(command_index, int)
                key = (output_index, command_index)
                if event_type == "response.shell_call_command.added":
                    if key in shell_commands or key in completed_shell_commands:
                        ctx.add(
                            f"{base}/data/command_index",
                            "SEQUENCE",
                            "shell command index opened once",
                            "event sequence differs from required lifecycle",
                        )
                    elif isinstance(payload.get(value_field), str):
                        shell_commands[key] = payload[value_field]
                elif key not in shell_commands:
                    ctx.add(
                        f"{base}/data/command_index",
                        "CORRELATION",
                        "index of a previously added shell command",
                        "correlated values differ",
                    )
                elif event_type == "response.shell_call_command.delta":
                    delta = payload.get("delta")
                    if isinstance(delta, str):
                        shell_commands[key] += delta
                elif event_type == "response.shell_call_command.done":
                    command = payload.get("command")
                    if isinstance(command, str) and command != shell_commands[key]:
                        ctx.add(
                            f"{base}/data/command",
                            "CORRELATION",
                            "same command as shell_call_command.added",
                            "correlated values differ",
                        )
                    shell_commands.pop(key, None)
                    completed_shell_commands.add(key)
            continue

        if event_type in _SHELL_OUTPUT_EVENTS:
            value_field = (
                "delta"
                if event_type == "response.shell_call_output_content.delta"
                else "output"
            )
            payload = _closed_shape(
                ctx,
                decoded,
                f"{base}/data",
                (
                    "type",
                    "sequence_number",
                    "item_id",
                    "output_index",
                    "command_index",
                    value_field,
                ),
            )
            if payload is None:
                continue
            if "item_id" in payload:
                _string(ctx, payload["item_id"], f"{base}/data/item_id")
            for field in ("output_index", "command_index"):
                if field in payload:
                    _integer(ctx, payload[field], f"{base}/data/{field}")
            if value_field in payload:
                if value_field == "delta":
                    _validate_shell_delta(
                        ctx,
                        payload[value_field],
                        f"{base}/data/{value_field}",
                    )
                else:
                    _validate_shell_output(
                        ctx,
                        payload[value_field],
                        f"{base}/data/{value_field}",
                    )
            correlated = _correlate_item_reference(
                ctx, event, payload, open_items
            )
            if correlated is not None and correlated[1].get("type") != "shell_call":
                ctx.add(
                    f"{base}/data/item_id",
                    "CORRELATION",
                    "id of a shell_call output item",
                    "referenced item has another type",
                )
            output_index = payload.get("output_index")
            command_index = payload.get("command_index")
            if _is_integer(output_index) and _is_integer(command_index):
                assert isinstance(output_index, int)
                assert isinstance(command_index, int)
                key = (output_index, command_index)
                if key not in shell_commands and key not in completed_shell_commands:
                    ctx.add(
                        f"{base}/data/command_index",
                        "CORRELATION",
                        "index of a streamed shell command",
                        "correlated values differ",
                    )
            continue

    if not saw_terminal:
        ctx.add(
            f"/events/{len(events)}",
            "SEQUENCE",
            "one final response.completed, response.incomplete, response.failed, or error event",
            "event sequence differs from required lifecycle",
        )
