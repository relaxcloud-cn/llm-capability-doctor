"""Validate evidence.v3 native tool-loop structure and transitions."""

from __future__ import annotations

from copy import deepcopy
import json
from typing import Mapping, Optional

from model_doctor_contracts import V3_CONTRACT, contract_key


V3_TOOL_TEST_IDS = frozenset({"046", "047", "048", "049"})
V3_MIN_TOOL_TURNS = {"046": 2, "047": 3, "048": 2, "049": 3}

_OFFICIAL_REFERENCES = {
    "openai_chat": (
        "https://github.com/openai/openai-openapi/blob/"
        "2186421dca0cca7c1e67caa7739005e8b1ccc4dd/openapi.json"
    ),
    "openai_responses": (
        "https://github.com/openai/openai-openapi/blob/"
        "2186421dca0cca7c1e67caa7739005e8b1ccc4dd/openapi.json"
    ),
    "anthropic_messages": "https://platform.claude.com/docs/en/api/messages/create",
    "gemini_generate_content": (
        "https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta"
    ),
    "ollama_chat": (
        "https://github.com/ollama/ollama/blob/"
        "d67ad83426633195089509347ffd4fe795120198/docs/openapi.yaml"
    ),
}


def _type_name(value: object) -> str:
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


def _token(value: object) -> str:
    return str(value).replace("~", "~0").replace("/", "~1")


def _path(parent: str, field: object) -> str:
    return f"{parent}/{_token(field)}"


def _difference(
    request_id: str,
    protocol: Optional[str],
    location: str,
    kind: str,
    expected: str,
) -> dict[str, str]:
    actual = {
        "CORRELATION": "correlated values differ",
        "MISSING_FIELD": "missing",
        "TYPE_MISMATCH": "value of unexpected type",
        "UNEXPECTED_FIELD": "present",
        "VALUE_MISMATCH": "value differs from required value",
    }[kind]
    return {
        "requestId": request_id,
        "protocol": protocol or "unknown",
        "location": location,
        "differenceKind": kind,
        "expected": expected,
        "actual": actual,
        "officialReference": _OFFICIAL_REFERENCES.get(
            protocol or "", "https://www.rfc-editor.org/rfc/rfc8259"
        ),
    }


class _Audit:
    def __init__(self, request_id: str, protocol: Optional[str]) -> None:
        self.request_id = request_id
        self.protocol = protocol
        self.differences: list[dict[str, str]] = []

    def add(self, location: str, kind: str, expected: str) -> None:
        self.differences.append(
            _difference(
                self.request_id,
                self.protocol,
                location,
                kind,
                expected,
            )
        )

    def require_mapping(self, value: object, location: str) -> Optional[dict]:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object")
            return None
        return value

    def require_list(self, value: object, location: str) -> Optional[list]:
        if not isinstance(value, list):
            self.add(location, "TYPE_MISMATCH", "array")
            return None
        return value

    def require_string(
        self, value: object, location: str, *, non_empty: bool = False
    ) -> bool:
        if not isinstance(value, str):
            self.add(location, "TYPE_MISMATCH", "string")
            return False
        if non_empty and not value.strip():
            self.add(location, "VALUE_MISMATCH", "non-empty string")
            return False
        return True

    def required(self, value: Mapping[str, object], field: str, location: str) -> bool:
        if field not in value:
            self.add(_path(location, field), "MISSING_FIELD", "present")
            return False
        return True

    def reject(self, value: Mapping[str, object], fields: set[str], location: str) -> None:
        for field in sorted(fields & set(value)):
            self.add(
                _path(location, field),
                "UNEXPECTED_FIELD",
                "absent from this official protocol request",
            )


def _decode_body(request: object, audit: _Audit) -> Optional[dict]:
    if not isinstance(request, dict):
        audit.add("/requestBody", "TYPE_MISMATCH", "request object")
        return None
    raw = request.get("requestBody")
    if not isinstance(raw, str):
        audit.add("/requestBody", "TYPE_MISMATCH", "JSON object encoded as a string")
        return None
    try:
        value = json.loads(raw)
    except (TypeError, ValueError):
        audit.add("/requestBody", "TYPE_MISMATCH", "valid JSON object")
        return None
    return audit.require_mapping(value, "/requestBody")


def _validate_schema(value: object, location: str, audit: _Audit) -> None:
    schema = audit.require_mapping(value, location)
    if schema is None:
        return
    if schema.get("type") != "object":
        kind = "MISSING_FIELD" if "type" not in schema else "VALUE_MISMATCH"
        audit.add(_path(location, "type"), kind, '"object"')
    if "properties" not in schema:
        audit.add(_path(location, "properties"), "MISSING_FIELD", "object")
    elif not isinstance(schema.get("properties"), dict):
        audit.add(_path(location, "properties"), "TYPE_MISMATCH", "object")
    if "required" in schema and not isinstance(schema.get("required"), list):
        audit.add(_path(location, "required"), "TYPE_MISMATCH", "array")


def _validate_function(
    value: object,
    location: str,
    audit: _Audit,
    *,
    schema_field: str,
    allow_strict: bool,
) -> None:
    function = audit.require_mapping(value, location)
    if function is None:
        return
    for field in ("name", schema_field):
        audit.required(function, field, location)
    foreign_schema_field = (
        "input_schema" if schema_field == "parameters" else "parameters"
    )
    if foreign_schema_field in function:
        audit.add(
            _path(location, foreign_schema_field),
            "UNEXPECTED_FIELD",
            f"{schema_field} for this protocol",
        )
    if "name" in function:
        audit.require_string(function["name"], _path(location, "name"), non_empty=True)
    if schema_field in function:
        _validate_schema(function[schema_field], _path(location, schema_field), audit)
    if "description" in function:
        audit.require_string(function["description"], _path(location, "description"))
    if "strict" in function:
        if not allow_strict:
            audit.add(
                _path(location, "strict"),
                "UNEXPECTED_FIELD",
                "absent from the native tool declaration",
            )
        elif not isinstance(function["strict"], bool):
            audit.add(_path(location, "strict"), "TYPE_MISMATCH", "boolean")


def _validate_tools(protocol: str, body: dict, audit: _Audit) -> None:
    location = "/requestBody/tools"
    if "tools" not in body:
        audit.add(location, "MISSING_FIELD", "non-empty official tool declarations")
        return
    tools = audit.require_list(body["tools"], location)
    if tools is None:
        return
    if not tools:
        audit.add(location, "VALUE_MISMATCH", "non-empty official tool declarations")
        return
    if protocol == "gemini_generate_content":
        for tool_index, value in enumerate(tools):
            tool_path = f"{location}/{tool_index}"
            tool = audit.require_mapping(value, tool_path)
            if tool is None:
                continue
            declarations = tool.get("functionDeclarations")
            if declarations is None:
                audit.add(
                    f"{tool_path}/functionDeclarations",
                    "MISSING_FIELD",
                    "non-empty function declaration array",
                )
                continue
            items = audit.require_list(declarations, f"{tool_path}/functionDeclarations")
            if items is None:
                continue
            if not items:
                audit.add(
                    f"{tool_path}/functionDeclarations",
                    "VALUE_MISMATCH",
                    "non-empty function declaration array",
                )
            for index, declaration in enumerate(items):
                _validate_function(
                    declaration,
                    f"{tool_path}/functionDeclarations/{index}",
                    audit,
                    schema_field="parameters",
                    allow_strict=False,
                )
        return

    for index, value in enumerate(tools):
        tool_path = f"{location}/{index}"
        tool = audit.require_mapping(value, tool_path)
        if tool is None:
            continue
        if protocol == "anthropic_messages":
            audit.reject(
                tool,
                {"function", "type"},
                tool_path,
            )
            _validate_function(
                tool,
                tool_path,
                audit,
                schema_field="input_schema",
                allow_strict=False,
            )
            continue
        if protocol == "openai_responses":
            audit.reject(tool, {"function"}, tool_path)
            if tool.get("type") != "function":
                kind = "MISSING_FIELD" if "type" not in tool else "VALUE_MISMATCH"
                audit.add(f"{tool_path}/type", kind, '"function"')
            _validate_function(
                tool,
                tool_path,
                audit,
                schema_field="parameters",
                allow_strict=True,
            )
            continue
        if tool.get("type") != "function":
            kind = "MISSING_FIELD" if "type" not in tool else "VALUE_MISMATCH"
            audit.add(f"{tool_path}/type", kind, '"function"')
        if "function" not in tool:
            audit.add(f"{tool_path}/function", "MISSING_FIELD", "function object")
            continue
        audit.reject(
            tool,
            {"input_schema", "name", "parameters", "strict"},
            tool_path,
        )
        _validate_function(
            tool["function"],
            f"{tool_path}/function",
            audit,
            schema_field="parameters",
            allow_strict=protocol == "openai_chat",
        )


def _validate_history_tool_calls(
    protocol: str,
    value: object,
    location: str,
    audit: _Audit,
) -> None:
    calls = audit.require_list(value, location)
    if calls is None:
        return
    for index, raw_call in enumerate(calls):
        path = f"{location}/{index}"
        call = audit.require_mapping(raw_call, path)
        if call is None:
            continue
        function = call.get("function")
        if function is None:
            audit.add(f"{path}/function", "MISSING_FIELD", "function object")
            continue
        function = audit.require_mapping(function, f"{path}/function")
        if function is None:
            continue
        if audit.required(function, "name", f"{path}/function"):
            audit.require_string(
                function["name"],
                f"{path}/function/name",
                non_empty=True,
            )
        if audit.required(function, "arguments", f"{path}/function"):
            arguments = function["arguments"]
            if protocol == "openai_chat":
                audit.require_string(arguments, f"{path}/function/arguments")
            elif not isinstance(arguments, dict):
                audit.add(f"{path}/function/arguments", "TYPE_MISMATCH", "object")
        if protocol == "openai_chat":
            if call.get("type") != "function":
                kind = "MISSING_FIELD" if "type" not in call else "VALUE_MISMATCH"
                audit.add(f"{path}/type", kind, '"function"')
            if audit.required(call, "id", path):
                audit.require_string(call["id"], f"{path}/id", non_empty=True)
            if "index" in call:
                audit.add(
                    f"{path}/index",
                    "UNEXPECTED_FIELD",
                    "stream-only indexes removed from assistant history",
                )


def _validate_anthropic_content(value: object, location: str, audit: _Audit) -> None:
    if isinstance(value, str):
        return
    parts = audit.require_list(value, location)
    if parts is None:
        return
    for index, raw_part in enumerate(parts):
        path = f"{location}/{index}"
        part = audit.require_mapping(raw_part, path)
        if part is None:
            continue
        kind = part.get("type")
        if kind == "tool_use":
            for field in ("id", "name"):
                if audit.required(part, field, path):
                    audit.require_string(part[field], f"{path}/{field}", non_empty=True)
            if audit.required(part, "input", path):
                audit.require_mapping(part["input"], f"{path}/input")
        elif kind == "tool_result":
            if audit.required(part, "tool_use_id", path):
                audit.require_string(
                    part["tool_use_id"],
                    f"{path}/tool_use_id",
                    non_empty=True,
                )
            if "is_error" in part and not isinstance(part["is_error"], bool):
                audit.add(f"{path}/is_error", "TYPE_MISMATCH", "boolean")
        elif kind == "text":
            if audit.required(part, "text", path):
                audit.require_string(part["text"], f"{path}/text")
        else:
            field_kind = "MISSING_FIELD" if "type" not in part else "VALUE_MISMATCH"
            audit.add(f"{path}/type", field_kind, "provider-native content block type")


def _validate_messages(protocol: str, body: dict, audit: _Audit) -> None:
    messages = audit.require_list(body.get("messages"), "/requestBody/messages")
    if messages is None:
        return
    if not messages:
        audit.add("/requestBody/messages", "VALUE_MISMATCH", "non-empty conversation")
    allowed_roles = {"user", "assistant"}
    if protocol in {"openai_chat", "ollama_chat"}:
        allowed_roles.add("system")
        allowed_roles.add("tool")
    for index, value in enumerate(messages):
        path = f"/requestBody/messages/{index}"
        message = audit.require_mapping(value, path)
        if message is None:
            continue
        role = message.get("role")
        if not isinstance(role, str):
            audit.add(f"{path}/role", "TYPE_MISMATCH", "provider-native role string")
        elif role not in allowed_roles:
            audit.add(f"{path}/role", "VALUE_MISMATCH", "provider-native role")
        if protocol == "openai_chat" and role == "tool":
            if not audit.required(message, "tool_call_id", path):
                continue
            audit.require_string(message["tool_call_id"], f"{path}/tool_call_id", non_empty=True)
            if "content" in message:
                audit.require_string(message["content"], f"{path}/content")
        if protocol == "openai_chat" and role == "assistant":
            content = message.get("content")
            if content is not None:
                audit.require_string(content, f"{path}/content")
            if "tool_calls" in message:
                _validate_history_tool_calls(
                    protocol,
                    message["tool_calls"],
                    f"{path}/tool_calls",
                    audit,
                )
        if protocol == "ollama_chat":
            if "tool_call_id" in message:
                audit.add(
                    f"{path}/tool_call_id",
                    "UNEXPECTED_FIELD",
                    "tool_name correlation without OpenAI tool_call_id",
                )
            if role == "tool":
                if audit.required(message, "tool_name", path):
                    audit.require_string(message["tool_name"], f"{path}/tool_name", non_empty=True)
                if audit.required(message, "content", path):
                    audit.require_string(message["content"], f"{path}/content")
            if role == "assistant" and "tool_calls" in message:
                _validate_history_tool_calls(
                    protocol,
                    message["tool_calls"],
                    f"{path}/tool_calls",
                    audit,
                )
        if protocol == "anthropic_messages":
            _validate_anthropic_content(message.get("content"), f"{path}/content", audit)
        if protocol == "anthropic_messages" and isinstance(message.get("content"), list):
            saw_non_result = False
            for part_index, part_value in enumerate(message["content"]):
                part_path = f"{path}/content/{part_index}"
                part = audit.require_mapping(part_value, part_path)
                if part is None:
                    continue
                if part.get("type") == "tool_result":
                    if saw_non_result:
                        audit.add(
                            part_path,
                            "VALUE_MISMATCH",
                            "all tool_result blocks before any text block",
                        )
                else:
                    saw_non_result = True


def _validate_contents(body: dict, audit: _Audit) -> None:
    contents = audit.require_list(body.get("contents"), "/requestBody/contents")
    if contents is None:
        return
    if not contents:
        audit.add("/requestBody/contents", "VALUE_MISMATCH", "non-empty conversation")
    for index, value in enumerate(contents):
        path = f"/requestBody/contents/{index}"
        content = audit.require_mapping(value, path)
        if content is None:
            continue
        if content.get("role") not in {"user", "model"}:
            kind = "MISSING_FIELD" if "role" not in content else "VALUE_MISMATCH"
            audit.add(f"{path}/role", kind, "user or model")
        parts = audit.require_list(content.get("parts"), f"{path}/parts")
        if parts is None:
            continue
        for part_index, part_value in enumerate(parts):
            part_path = f"{path}/parts/{part_index}"
            part = audit.require_mapping(part_value, part_path)
            if part is None:
                continue
            if "thoughtSignature" in part:
                audit.require_string(
                    part["thoughtSignature"],
                    f"{part_path}/thoughtSignature",
                    non_empty=True,
                )
            function_call = part.get("functionCall")
            if function_call is not None:
                call_path = f"{part_path}/functionCall"
                call = audit.require_mapping(function_call, call_path)
                if call is not None:
                    if audit.required(call, "name", call_path):
                        audit.require_string(
                            call["name"],
                            f"{call_path}/name",
                            non_empty=True,
                        )
                    if audit.required(call, "args", call_path):
                        audit.require_mapping(call["args"], f"{call_path}/args")
                    if "id" in call:
                        audit.require_string(
                            call["id"],
                            f"{call_path}/id",
                            non_empty=True,
                        )
            response = part.get("functionResponse")
            if response is not None:
                response_path = f"{part_path}/functionResponse"
                response_object = audit.require_mapping(response, response_path)
                if response_object is None:
                    continue
                if audit.required(response_object, "name", response_path):
                    audit.require_string(response_object["name"], f"{response_path}/name", non_empty=True)
                if audit.required(response_object, "response", response_path):
                    audit.require_mapping(response_object["response"], f"{response_path}/response")


def _validate_responses_input(value: object, audit: _Audit) -> None:
    if isinstance(value, str):
        return
    items = audit.require_list(value, "/requestBody/input")
    if items is None:
        return
    if not items:
        audit.add("/requestBody/input", "VALUE_MISMATCH", "non-empty native input")
    for index, raw_item in enumerate(items):
        path = f"/requestBody/input/{index}"
        item = audit.require_mapping(raw_item, path)
        if item is None:
            continue
        if item.get("type") != "function_call_output":
            kind = "MISSING_FIELD" if "type" not in item else "VALUE_MISMATCH"
            audit.add(f"{path}/type", kind, '"function_call_output"')
        if audit.required(item, "call_id", path):
            audit.require_string(item["call_id"], f"{path}/call_id", non_empty=True)
        if audit.required(item, "output", path):
            audit.require_string(item["output"], f"{path}/output")


def _validate_request_body(request_id: str, request: object) -> tuple[Optional[dict], list[dict]]:
    protocol = request.get("protocol") if isinstance(request, dict) else None
    protocol = protocol if isinstance(protocol, str) else None
    audit = _Audit(request_id, protocol)
    body = _decode_body(request, audit)
    if body is None:
        return None, audit.differences
    if protocol not in _OFFICIAL_REFERENCES:
        audit.add("/protocol", "VALUE_MISMATCH", "one supported official protocol")
        return body, audit.differences

    assert protocol is not None
    if protocol == "gemini_generate_content":
        audit.reject(
            body,
            {"input", "messages", "parallel_tool_calls", "previous_response_id", "stream", "tool_choice"},
            "/requestBody",
        )
        _validate_contents(body, audit)
    else:
        if body.get("stream") is not True:
            kind = "MISSING_FIELD" if "stream" not in body else "VALUE_MISMATCH"
            audit.add("/requestBody/stream", kind, "true")
        if protocol == "openai_chat":
            audit.reject(body, {"contents", "input", "previous_response_id"}, "/requestBody")
            _validate_messages(protocol, body, audit)
        elif protocol == "openai_responses":
            audit.reject(body, {"contents", "messages"}, "/requestBody")
            if "input" not in body:
                audit.add("/requestBody/input", "MISSING_FIELD", "string or function_call_output array")
            else:
                _validate_responses_input(body["input"], audit)
        elif protocol == "anthropic_messages":
            audit.reject(body, {"contents", "input", "parallel_tool_calls", "previous_response_id"}, "/requestBody")
            _validate_messages(protocol, body, audit)
        elif protocol == "ollama_chat":
            audit.reject(
                body,
                {"contents", "input", "parallel_tool_calls", "previous_response_id", "tool_choice"},
                "/requestBody",
            )
            _validate_messages(protocol, body, audit)
    _validate_tools(protocol, body, audit)
    return body, audit.differences


def _sse_payloads(raw: object) -> list[tuple[Optional[str], dict]]:
    if not isinstance(raw, str):
        return []
    frames: list[list[str]] = []
    current: list[str] = []
    for line in raw.splitlines():
        if not line:
            if current:
                frames.append(current)
                current = []
            continue
        current.append(line[:-1] if line.endswith("\r") else line)
    if current:
        frames.append(current)
    payloads: list[tuple[Optional[str], dict]] = []
    for frame in frames:
        event: Optional[str] = None
        data: list[str] = []
        for line in frame:
            field, separator, value = line.partition(":")
            if separator and value.startswith(" "):
                value = value[1:]
            if field == "event":
                event = value
            elif field == "data":
                data.append(value)
        joined = "\n".join(data)
        if not joined or joined == "[DONE]":
            continue
        try:
            decoded = json.loads(joined)
        except (TypeError, ValueError):
            continue
        if isinstance(decoded, dict):
            payloads.append((event, decoded))
    return payloads


def _ndjson_payloads(raw: object) -> list[dict]:
    if not isinstance(raw, str):
        return []
    payloads: list[dict] = []
    for line in raw.splitlines():
        if not line.strip():
            continue
        try:
            decoded = json.loads(line)
        except (TypeError, ValueError):
            continue
        if isinstance(decoded, dict):
            payloads.append(decoded)
    return payloads


def _chat_turn(raw: object) -> Optional[tuple[dict, list[dict]]]:
    role = "assistant"
    content = ""
    saw_content = False
    calls: dict[int, dict] = {}
    for _, payload in _sse_payloads(raw):
        choices = payload.get("choices")
        if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
            continue
        delta = choices[0].get("delta")
        if not isinstance(delta, dict):
            continue
        if isinstance(delta.get("role"), str):
            role = delta["role"]
        if isinstance(delta.get("content"), str):
            saw_content = True
            content += delta["content"]
        for raw_call in delta.get("tool_calls", []) if isinstance(delta.get("tool_calls"), list) else []:
            if not isinstance(raw_call, dict) or not isinstance(raw_call.get("index"), int):
                continue
            index = raw_call["index"]
            call = calls.setdefault(index, {"function": {"name": "", "arguments": ""}})
            for field in ("id", "type"):
                if isinstance(raw_call.get(field), str):
                    call[field] = raw_call[field]
            function = raw_call.get("function")
            if isinstance(function, dict):
                for field in ("name", "arguments"):
                    if isinstance(function.get(field), str):
                        call["function"][field] += function[field]
    if not calls:
        return None
    native_calls = [calls[index] for index in sorted(calls)]
    history = {
        "role": role,
        "content": content if saw_content and content else None,
        "tool_calls": native_calls,
    }
    normalized = [
        {
            "id": call.get("id"),
            "name": call.get("function", {}).get("name"),
            "arguments": call.get("function", {}).get("arguments"),
        }
        for call in native_calls
    ]
    return history, normalized


def _responses_turn(raw: object) -> Optional[tuple[str, list[dict]]]:
    completed: Optional[dict] = None
    for event, payload in _sse_payloads(raw):
        if event == "response.completed" or payload.get("type") == "response.completed":
            response = payload.get("response")
            if isinstance(response, dict):
                completed = response
    if completed is None or not isinstance(completed.get("id"), str):
        return None
    calls = []
    output = completed.get("output")
    if isinstance(output, list):
        for item in output:
            if isinstance(item, dict) and item.get("type") == "function_call":
                calls.append(
                    {
                        "id": item.get("call_id"),
                        "name": item.get("name"),
                        "arguments": item.get("arguments"),
                    }
                )
    return completed["id"], calls


def _anthropic_turn(raw: object) -> Optional[tuple[list[dict], list[dict]]]:
    blocks: dict[int, dict] = {}
    json_fragments: dict[int, str] = {}
    for event, payload in _sse_payloads(raw):
        kind = event or payload.get("type")
        index = payload.get("index")
        if kind == "content_block_start" and isinstance(index, int):
            block = payload.get("content_block")
            if isinstance(block, dict):
                blocks[index] = deepcopy(block)
                json_fragments[index] = ""
        elif kind == "content_block_delta" and isinstance(index, int):
            delta = payload.get("delta")
            block = blocks.get(index)
            if not isinstance(delta, dict) or block is None:
                continue
            if delta.get("type") == "text_delta" and isinstance(delta.get("text"), str):
                block["text"] = str(block.get("text", "")) + delta["text"]
            if delta.get("type") == "input_json_delta" and isinstance(delta.get("partial_json"), str):
                json_fragments[index] += delta["partial_json"]
    history = [blocks[index] for index in sorted(blocks)]
    calls = []
    for index in sorted(blocks):
        block = blocks[index]
        if block.get("type") != "tool_use":
            continue
        if json_fragments.get(index):
            try:
                block["input"] = json.loads(json_fragments[index])
            except (TypeError, ValueError):
                pass
        calls.append({"id": block.get("id"), "name": block.get("name")})
    return (history, calls) if calls else None


def _gemini_turn(raw: object) -> Optional[tuple[dict, list[dict]]]:
    role = "model"
    parts: list[dict] = []
    for _, payload in _sse_payloads(raw):
        candidates = payload.get("candidates")
        if not isinstance(candidates, list) or not candidates or not isinstance(candidates[0], dict):
            continue
        content = candidates[0].get("content")
        if not isinstance(content, dict):
            continue
        if isinstance(content.get("role"), str):
            role = content["role"]
        if isinstance(content.get("parts"), list):
            parts.extend(deepcopy(part) for part in content["parts"] if isinstance(part, dict))
    calls = []
    for part in parts:
        call = part.get("functionCall")
        if isinstance(call, dict):
            calls.append({"id": call.get("id"), "name": call.get("name")})
    return ({"role": role, "parts": parts}, calls) if calls else None


def _ollama_turn(raw: object) -> Optional[tuple[dict, list[dict]]]:
    history: dict[str, object] = {}
    content = ""
    thinking = ""
    raw_calls: list[dict] = []
    for payload in _ndjson_payloads(raw):
        message = payload.get("message")
        if not isinstance(message, dict):
            continue
        if isinstance(message.get("role"), str):
            history["role"] = message["role"]
        if isinstance(message.get("content"), str):
            content += message["content"]
            history["content"] = content
        if isinstance(message.get("thinking"), str):
            thinking += message["thinking"]
            history["thinking"] = thinking
        images = message.get("images")
        if images is None and "images" in message:
            history.setdefault("images", None)
        elif isinstance(images, list):
            existing = history.get("images")
            if not isinstance(existing, list):
                existing = []
                history["images"] = existing
            existing.extend(deepcopy(images))
        if isinstance(message.get("tool_calls"), list):
            raw_calls.extend(deepcopy(call) for call in message["tool_calls"] if isinstance(call, dict))
            history["tool_calls"] = raw_calls
        for field, value in message.items():
            if field not in {"role", "content", "thinking", "images", "tool_calls"}:
                history[field] = deepcopy(value)
    if not raw_calls:
        return None
    normalized = []
    for position, call in enumerate(raw_calls):
        function = call.get("function")
        if isinstance(function, dict):
            normalized.append(
                {
                    "id": call.get("id"),
                    "name": function.get("name"),
                    "native_index": function.get("index"),
                    "position": position,
                }
            )
    if normalized and all(
        isinstance(call["native_index"], int)
        and not isinstance(call["native_index"], bool)
        for call in normalized
    ):
        normalized.sort(key=lambda call: call["native_index"])
    for call in normalized:
        call.pop("native_index", None)
        call.pop("position", None)
    return history, normalized


def _stable_fields(previous: dict, following: dict, dynamic: set[str], audit: _Audit) -> None:
    keys = (set(previous) | set(following)) - dynamic
    for key in sorted(keys):
        if key not in previous or key not in following or previous[key] != following[key]:
            audit.add(
                _path("/requestBody", key),
                "VALUE_MISMATCH",
                "field preserved unchanged from the preceding request",
            )


def _prefix(value: object, expected: object, location: str, audit: _Audit) -> bool:
    if value != expected:
        audit.add(location, "VALUE_MISMATCH", "complete preceding conversation prefix")
        return False
    return True


def _correlate(
    actual: object,
    expected: object,
    location: str,
    audit: _Audit,
    label: str,
) -> None:
    if actual != expected:
        audit.add(location, "CORRELATION", label)


def _validate_transition(
    test_id: str,
    transition: int,
    previous_request: dict,
    previous_body: dict,
    following_request: dict,
    following_body: dict,
    audit: _Audit,
) -> None:
    protocol = previous_request.get("protocol")
    if following_request.get("protocol") != protocol:
        audit.add("/protocol", "CORRELATION", "same protocol on adjacent tool-loop turns")
        return
    if not isinstance(protocol, str):
        return
    raw_response = previous_request.get("responseBody")
    if protocol == "openai_chat":
        parsed = _chat_turn(raw_response)
        if parsed is None:
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "follow-up to a streamed assistant tool call")
            return
        history, calls = parsed
        _stable_fields(previous_body, following_body, {"messages"}, audit)
        before = previous_body.get("messages")
        after = following_body.get("messages")
        if not isinstance(before, list) or not isinstance(after, list):
            return
        _prefix(after[: len(before)], before, "/requestBody/messages", audit)
        suffix = after[len(before) :]
        if len(suffix) != 1 + len(calls):
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "assistant turn followed by one result per tool call")
            return
        _prefix(suffix[0], history, f"/requestBody/messages/{len(before)}", audit)
        for index, (call, result) in enumerate(zip(calls, suffix[1:])):
            path = f"/requestBody/messages/{len(before) + 1 + index}"
            item = audit.require_mapping(result, path)
            if item is None:
                continue
            if item.get("role") != "tool":
                audit.add(f"{path}/role", "VALUE_MISMATCH", '"tool"')
            _correlate(item.get("tool_call_id"), call.get("id"), f"{path}/tool_call_id", audit, "matching streamed tool call ID")
            if audit.required(item, "content", path):
                audit.require_string(item["content"], f"{path}/content")
        return

    if protocol == "openai_responses":
        parsed = _responses_turn(raw_response)
        if parsed is None:
            audit.add("/requestBody/previous_response_id", "VALUE_MISMATCH", "completed streamed response with function calls")
            return
        response_id, calls = parsed
        _stable_fields(previous_body, following_body, {"input", "previous_response_id"}, audit)
        _correlate(following_body.get("previous_response_id"), response_id, "/requestBody/previous_response_id", audit, "preceding streamed response ID")
        outputs = following_body.get("input")
        if not isinstance(outputs, list):
            return
        if len(outputs) != len(calls):
            audit.add("/requestBody/input", "VALUE_MISMATCH", "one function_call_output per streamed function call")
            return
        for index, (call, output) in enumerate(zip(calls, outputs)):
            path = f"/requestBody/input/{index}"
            item = audit.require_mapping(output, path)
            if item is None:
                continue
            if item.get("type") != "function_call_output":
                audit.add(f"{path}/type", "VALUE_MISMATCH", '"function_call_output"')
            _correlate(item.get("call_id"), call.get("id"), f"{path}/call_id", audit, "matching streamed function call ID")
            if audit.required(item, "output", path):
                audit.require_string(item["output"], f"{path}/output")
        return

    if protocol == "anthropic_messages":
        parsed = _anthropic_turn(raw_response)
        if parsed is None:
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "follow-up to streamed tool_use blocks")
            return
        history, calls = parsed
        _stable_fields(previous_body, following_body, {"messages"}, audit)
        before = previous_body.get("messages")
        after = following_body.get("messages")
        if not isinstance(before, list) or not isinstance(after, list):
            return
        _prefix(after[: len(before)], before, "/requestBody/messages", audit)
        suffix = after[len(before) :]
        if len(suffix) != 2:
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "assistant tool_use turn followed by one user result turn")
            return
        _prefix(suffix[0], {"role": "assistant", "content": history}, f"/requestBody/messages/{len(before)}", audit)
        user = audit.require_mapping(suffix[1], f"/requestBody/messages/{len(before) + 1}")
        if user is None:
            return
        if user.get("role") != "user":
            audit.add(f"/requestBody/messages/{len(before) + 1}/role", "VALUE_MISMATCH", '"user"')
        results = user.get("content")
        if not isinstance(results, list) or len(results) != len(calls):
            audit.add(f"/requestBody/messages/{len(before) + 1}/content", "VALUE_MISMATCH", "one tool_result per streamed tool_use")
            return
        timeout = test_id == "049" and transition == 1
        for index, (call, result) in enumerate(zip(calls, results)):
            path = f"/requestBody/messages/{len(before) + 1}/content/{index}"
            item = audit.require_mapping(result, path)
            if item is None:
                continue
            if item.get("type") != "tool_result":
                audit.add(f"{path}/type", "VALUE_MISMATCH", '"tool_result"')
            _correlate(item.get("tool_use_id"), call.get("id"), f"{path}/tool_use_id", audit, "matching streamed tool_use ID")
            if timeout and item.get("is_error") is not True:
                kind = "MISSING_FIELD" if "is_error" not in item else "VALUE_MISMATCH"
                audit.add(f"{path}/is_error", kind, "true for the timeout result")
            if not timeout and "is_error" in item and item.get("is_error") is not False:
                audit.add(f"{path}/is_error", "VALUE_MISMATCH", "absent or false for a successful result")
        return

    if protocol == "gemini_generate_content":
        parsed = _gemini_turn(raw_response)
        if parsed is None:
            audit.add("/requestBody/contents", "VALUE_MISMATCH", "follow-up to streamed functionCall parts")
            return
        history, calls = parsed
        _stable_fields(previous_body, following_body, {"contents"}, audit)
        before = previous_body.get("contents")
        after = following_body.get("contents")
        if not isinstance(before, list) or not isinstance(after, list):
            return
        _prefix(after[: len(before)], before, "/requestBody/contents", audit)
        suffix = after[len(before) :]
        if len(suffix) != 2:
            audit.add("/requestBody/contents", "VALUE_MISMATCH", "model functionCall content followed by one user response content")
            return
        _prefix(suffix[0], history, f"/requestBody/contents/{len(before)}", audit)
        user = audit.require_mapping(suffix[1], f"/requestBody/contents/{len(before) + 1}")
        if user is None:
            return
        parts = user.get("parts")
        if user.get("role") != "user" or not isinstance(parts, list) or len(parts) != len(calls):
            audit.add(f"/requestBody/contents/{len(before) + 1}", "VALUE_MISMATCH", "user content with one functionResponse per call")
            return
        for index, (call, part) in enumerate(zip(calls, parts)):
            path = f"/requestBody/contents/{len(before) + 1}/parts/{index}/functionResponse"
            item = part.get("functionResponse") if isinstance(part, dict) else None
            response = audit.require_mapping(item, path)
            if response is None:
                continue
            _correlate(response.get("name"), call.get("name"), f"{path}/name", audit, "matching streamed function name")
            if call.get("id") is not None:
                _correlate(response.get("id"), call.get("id"), f"{path}/id", audit, "matching optional streamed function ID")
            elif "id" in response:
                audit.add(f"{path}/id", "UNEXPECTED_FIELD", "absent when the streamed call has no ID")
            if not isinstance(response.get("response"), dict):
                audit.add(f"{path}/response", "TYPE_MISMATCH", "object")
        return

    if protocol == "ollama_chat":
        parsed = _ollama_turn(raw_response)
        if parsed is None:
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "follow-up to streamed Ollama tool_calls")
            return
        history, calls = parsed
        _stable_fields(previous_body, following_body, {"messages"}, audit)
        before = previous_body.get("messages")
        after = following_body.get("messages")
        if not isinstance(before, list) or not isinstance(after, list):
            return
        _prefix(after[: len(before)], before, "/requestBody/messages", audit)
        suffix = after[len(before) :]
        if len(suffix) != 1 + len(calls):
            audit.add("/requestBody/messages", "VALUE_MISMATCH", "assistant turn followed by one native tool result per call")
            return
        _prefix(suffix[0], history, f"/requestBody/messages/{len(before)}", audit)
        for index, (call, result) in enumerate(zip(calls, suffix[1:])):
            path = f"/requestBody/messages/{len(before) + 1 + index}"
            item = audit.require_mapping(result, path)
            if item is None:
                continue
            if "tool_call_id" in item:
                audit.add(f"{path}/tool_call_id", "UNEXPECTED_FIELD", "native Ollama tool_name correlation")
            _correlate(item.get("tool_name"), call.get("name"), f"{path}/tool_name", audit, "matching streamed function name")
            if audit.required(item, "content", path):
                audit.require_string(item["content"], f"{path}/content")


def _append_unique(target: list[dict], additions: list[dict]) -> None:
    for difference in additions:
        if difference not in target:
            target.append(difference)


def validate_tool_loop_transitions(
    parsed: Mapping[str, object],
) -> dict[str, list[dict[str, str]]]:
    """Return bounded official-shape differences keyed by follow-up request ID."""

    try:
        if contract_key(parsed.get("run")) != V3_CONTRACT:
            return {}
    except ValueError:
        return {}
    tests = parsed.get("tests")
    requests = parsed.get("requests")
    if not isinstance(tests, dict) or not isinstance(requests, dict):
        return {}
    differences: dict[str, list[dict[str, str]]] = {}
    for test_id in sorted(V3_TOOL_TEST_IDS):
        manifest = tests.get(test_id)
        if not isinstance(manifest, dict):
            continue
        refs = manifest.get("requestRefs")
        if not isinstance(refs, list) or not all(isinstance(ref, str) for ref in refs):
            continue
        decoded: dict[str, Optional[dict]] = {}
        for request_id in refs:
            request = requests.get(request_id)
            body, body_differences = _validate_request_body(request_id, request)
            decoded[request_id] = body
            if body_differences:
                _append_unique(differences.setdefault(request_id, []), body_differences)
        for transition, (previous_id, following_id) in enumerate(zip(refs, refs[1:]), start=1):
            previous_request = requests.get(previous_id)
            following_request = requests.get(following_id)
            previous_body = decoded.get(previous_id)
            following_body = decoded.get(following_id)
            if not all(
                isinstance(value, dict)
                for value in (previous_request, following_request, previous_body, following_body)
            ):
                continue
            assert isinstance(previous_request, dict)
            assert isinstance(following_request, dict)
            assert isinstance(previous_body, dict)
            assert isinstance(following_body, dict)
            following_protocol = following_request.get("protocol")
            audit = _Audit(
                following_id,
                following_protocol if isinstance(following_protocol, str) else None,
            )
            _validate_transition(
                test_id,
                transition,
                previous_request,
                previous_body,
                following_request,
                following_body,
                audit,
            )
            if audit.differences:
                _append_unique(differences.setdefault(following_id, []), audit.differences)
    return differences


def _validate_v3_tool_pass_requests(test_id: str, requests: object) -> list[str]:
    errors: list[str] = []
    if not isinstance(requests, list):
        return [f"Test {test_id} PASS requests must be an array"]
    minimum = V3_MIN_TOOL_TURNS[test_id]
    if len(requests) < minimum:
        errors.append(f"Test {test_id} PASS requires at least {minimum} tool-loop turns")
    for turn, request in enumerate(requests, start=1):
        expected_id = f"test-{test_id}-turn-{turn}"
        if not isinstance(request, dict):
            errors.append(f"Test {test_id} turn {turn} request must be an object")
            continue
        if request.get("request_id") != expected_id:
            errors.append(f"Test {test_id} turn {turn} must reference {expected_id}")
        if request.get("tool_loop_turn") != str(turn):
            errors.append(f"Test {test_id} request {expected_id} has mismatched tool_loop_turn")
        if request.get("stream_termination") != "completed":
            errors.append(f"Test {test_id} request {expected_id} did not complete its stream")
        if request.get("transport_outcome") != "completed_eof":
            errors.append(
                f"Test {test_id} request {expected_id} did not reach clean transport EOF"
            )
        expected_signal = {
            "openai_chat": "[DONE]",
            "openai_responses": "response.completed",
            "anthropic_messages": "message_stop",
            "gemini_generate_content": "finishReason:STOP",
            "ollama_chat": "done:true",
        }.get(request.get("protocol"))
        if expected_signal is None or request.get("stream_end_signal") != expected_signal:
            errors.append(
                f"Test {test_id} request {expected_id} has an invalid terminal signal"
            )
        if request.get("tool_contract_status") != "conformant":
            errors.append(f"Test {test_id} request {expected_id} is not contract-conformant")
        expected_outcome = "completed" if turn == len(requests) else "continued"
        if request.get("tool_loop_outcome") != expected_outcome:
            errors.append(
                f"Test {test_id} request {expected_id} must have tool_loop_outcome={expected_outcome}"
            )
    return errors


def _validate_v3_tool_pass_review(
    parsed: dict,
    test_id: str,
    review: object,
) -> list[str]:
    if not isinstance(review, dict):
        return [f"Test {test_id} review must be an object"]
    if review.get("reviewedStatus") != "PASS" or test_id not in V3_TOOL_TEST_IDS:
        return []
    try:
        contract = contract_key(parsed.get("run"))
    except ValueError as error:
        return [f"Test {test_id} PASS has an invalid run contract: {error}"]
    if contract != V3_CONTRACT:
        return []
    tests = parsed.get("tests", {})
    request_map = parsed.get("requests", {})
    if not isinstance(tests, dict) or not isinstance(request_map, dict):
        return [f"Test {test_id} PASS has invalid parsed evidence maps"]
    manifest = tests.get(test_id, {})
    if not isinstance(manifest, dict):
        return [f"Test {test_id} PASS manifest must be an object"]
    refs = manifest.get("requestRefs")
    if not isinstance(refs, list) or not all(isinstance(ref, str) for ref in refs):
        return [f"Test {test_id} PASS requestRefs must be a string array"]
    expected_refs = [f"test-{test_id}-turn-{turn}" for turn in range(1, len(refs) + 1)]
    errors: list[str] = []
    if refs != expected_refs:
        errors.append(f"Test {test_id} PASS requestRefs must be contiguous ordered turns")
    missing = [request_id for request_id in refs if request_id not in request_map]
    if missing:
        errors.append(f"Test {test_id} PASS references missing request {missing[0]}")
        return errors
    mapped = [request_map[request_id] for request_id in refs]
    for request_id, request in zip(refs, mapped):
        if isinstance(request, dict) and request.get("request_id") != request_id:
            errors.append(
                f"Test {test_id} request map key {request_id} does not match request_id"
            )
    errors.extend(_validate_v3_tool_pass_requests(test_id, mapped))
    return errors
