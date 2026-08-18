"""Validate evidence.v3 native tool-loop structure and transitions."""

from __future__ import annotations

from copy import deepcopy
from typing import Mapping, Optional

from model_doctor_contracts import V3_CONTRACT, contract_key
from model_doctor_json import JSON_LOAD_ERRORS, strict_json_loads


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
        "d67ad83426633195089509347ffd4fe795120198/api/types.go"
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
) -> dict[str, object]:
    actual = {
        "CORRELATION": "correlated values differ",
        "MISSING_FIELD": "missing",
        "TYPE_MISMATCH": "value of unexpected type",
        "UNEXPECTED_FIELD": "present",
        "VALUE_MISMATCH": "value differs from required value",
    }[kind]
    return {
        "requestId": request_id,
        "protocol": protocol,
        "location": location,
        "differenceKind": kind,
        "expected": expected,
        "actual": actual,
        "officialReference": _OFFICIAL_REFERENCES.get(
            protocol, "https://www.rfc-editor.org/rfc/rfc8259"
        ),
    }


class _Audit:
    def __init__(self, request_id: str, protocol: Optional[str]) -> None:
        self.request_id = request_id
        self.protocol = protocol
        self.differences: list[dict[str, object]] = []

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
        value = strict_json_loads(raw)
    except JSON_LOAD_ERRORS:
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
    if not calls:
        audit.add(location, "VALUE_MISMATCH", "non-empty tool-call array")
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
        elif protocol == "ollama_chat":
            if "type" in call:
                audit.add(
                    f"{path}/type",
                    "UNEXPECTED_FIELD",
                    "absent from the native Ollama ToolCall",
                )
            if "id" in call:
                audit.require_string(call["id"], f"{path}/id", non_empty=True)
            if "index" in call:
                audit.add(
                    f"{path}/index",
                    "UNEXPECTED_FIELD",
                    "function.index instead of a top-level index",
                )
            if "index" in function:
                native_index = function["index"]
                if not isinstance(native_index, int) or isinstance(native_index, bool):
                    audit.add(
                        f"{path}/function/index",
                        "TYPE_MISMATCH",
                        "non-negative integer",
                    )
                elif native_index < 0:
                    audit.add(
                        f"{path}/function/index",
                        "VALUE_MISMATCH",
                        "non-negative integer",
                    )


def _validate_chat_content_part(
    value: object,
    location: str,
    audit: _Audit,
    *,
    allowed_types: set[str],
) -> None:
    part = audit.require_mapping(value, location)
    if part is None:
        return
    kind = part.get("type")
    if not isinstance(kind, str):
        difference_kind = "MISSING_FIELD" if "type" not in part else "TYPE_MISMATCH"
        audit.add(f"{location}/type", difference_kind, "content-part type string")
        return
    if kind not in allowed_types:
        audit.add(
            f"{location}/type",
            "VALUE_MISMATCH",
            "role-compatible official content-part type",
        )
        return
    if kind in {"text", "refusal"}:
        field = kind
        if audit.required(part, field, location):
            audit.require_string(part[field], f"{location}/{field}")
        return
    if kind == "image_url":
        if not audit.required(part, "image_url", location):
            return
        image = audit.require_mapping(part["image_url"], f"{location}/image_url")
        if image is None:
            return
        if audit.required(image, "url", f"{location}/image_url"):
            audit.require_string(
                image["url"],
                f"{location}/image_url/url",
                non_empty=True,
            )
        if "detail" in image:
            detail = image["detail"]
            if detail not in {"auto", "low", "high"}:
                audit.add(
                    f"{location}/image_url/detail",
                    "VALUE_MISMATCH",
                    '"auto", "low", or "high"',
                )
        return
    if kind == "input_audio":
        if not audit.required(part, "input_audio", location):
            return
        audio = audit.require_mapping(part["input_audio"], f"{location}/input_audio")
        if audio is None:
            return
        if audit.required(audio, "data", f"{location}/input_audio"):
            audit.require_string(
                audio["data"],
                f"{location}/input_audio/data",
                non_empty=True,
            )
        if audit.required(audio, "format", f"{location}/input_audio"):
            audio_format = audio["format"]
            if audio_format not in {"wav", "mp3"}:
                audit.add(
                    f"{location}/input_audio/format",
                    "VALUE_MISMATCH",
                    '"wav" or "mp3"',
                )
        return
    if not audit.required(part, "file", location):
        return
    file_value = audit.require_mapping(part["file"], f"{location}/file")
    if file_value is None:
        return
    for field in ("filename", "file_data", "file_id"):
        if field in file_value:
            audit.require_string(file_value[field], f"{location}/file/{field}")


def _validate_chat_content(
    value: object,
    location: str,
    audit: _Audit,
    *,
    allowed_types: set[str],
    nullable: bool = False,
) -> None:
    if value is None and nullable:
        return
    if isinstance(value, str):
        return
    parts = audit.require_list(value, location)
    if parts is None:
        return
    if not parts:
        audit.add(location, "VALUE_MISMATCH", "non-empty content-part array")
    for index, part in enumerate(parts):
        _validate_chat_content_part(
            part,
            f"{location}/{index}",
            audit,
            allowed_types=allowed_types,
        )


def _validate_anthropic_source(
    block: Mapping[str, object],
    location: str,
    audit: _Audit,
    *,
    block_type: str,
) -> None:
    if not audit.required(block, "source", location):
        return
    source_path = f"{location}/source"
    source = audit.require_mapping(block["source"], source_path)
    if source is None:
        return
    if not audit.required(source, "type", source_path):
        return
    source_type = source["type"]
    if not audit.require_string(source_type, f"{source_path}/type", non_empty=True):
        return
    allowed_source_types = {
        "image": {"base64", "url", "file"},
        "document": {"base64", "text", "content", "url", "file"},
    }[block_type]
    if source_type not in allowed_source_types:
        audit.add(
            f"{source_path}/type",
            "VALUE_MISMATCH",
            f"official Anthropic {block_type} source type",
        )
        return
    required_fields = {
        "base64": ("media_type", "data"),
        "text": ("media_type", "data"),
        "url": ("url",),
        "file": ("file_id",),
        "content": ("content",),
    }[source_type]
    for field in required_fields:
        if not audit.required(source, field, source_path):
            continue
        field_path = f"{source_path}/{field}"
        if field == "content" and isinstance(source[field], list):
            for index, nested in enumerate(source[field]):
                _validate_anthropic_result_block(
                    nested,
                    f"{field_path}/{index}",
                    audit,
                )
        else:
            audit.require_string(source[field], field_path, non_empty=True)


def _validate_anthropic_result_block(
    value: object,
    location: str,
    audit: _Audit,
) -> None:
    block = audit.require_mapping(value, location)
    if block is None:
        return
    block_type = block.get("type")
    if block_type == "text":
        if audit.required(block, "text", location):
            audit.require_string(block["text"], f"{location}/text")
        return
    if block_type in {"image", "document"}:
        _validate_anthropic_source(
            block,
            location,
            audit,
            block_type=block_type,
        )
        return
    if block_type == "search_result":
        for field in ("source", "title"):
            if audit.required(block, field, location):
                audit.require_string(
                    block[field],
                    f"{location}/{field}",
                    non_empty=True,
                )
        if not audit.required(block, "content", location):
            return
        content = audit.require_list(block["content"], f"{location}/content")
        if content is None:
            return
        if not content:
            audit.add(
                f"{location}/content",
                "VALUE_MISMATCH",
                "non-empty text-block array",
            )
        for index, nested in enumerate(content):
            _validate_anthropic_result_block(
                nested,
                f"{location}/content/{index}",
                audit,
            )
        return
    difference_kind = "MISSING_FIELD" if "type" not in block else "VALUE_MISMATCH"
    audit.add(
        f"{location}/type",
        difference_kind,
        "text, image, document, or search_result",
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
            if "content" in part:
                result_content = part["content"]
                if not isinstance(result_content, (str, list)):
                    audit.add(
                        f"{path}/content",
                        "TYPE_MISMATCH",
                        "string or provider-native content block array",
                    )
                elif isinstance(result_content, list):
                    for content_index, content_value in enumerate(result_content):
                        content_path = f"{path}/content/{content_index}"
                        _validate_anthropic_result_block(
                            content_value,
                            content_path,
                            audit,
                        )
        elif kind == "text":
            if audit.required(part, "text", path):
                audit.require_string(part["text"], f"{path}/text")
        elif kind in {"image", "document", "search_result"}:
            _validate_anthropic_result_block(part, path, audit)
        elif kind == "thinking":
            for field in ("thinking", "signature"):
                if audit.required(part, field, path):
                    audit.require_string(part[field], f"{path}/{field}")
        elif kind == "redacted_thinking":
            if audit.required(part, "data", path):
                audit.require_string(part["data"], f"{path}/data", non_empty=True)
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
    if protocol == "openai_chat":
        allowed_roles.add("developer")
        allowed_roles.add("function")
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
            if audit.required(message, "tool_call_id", path):
                audit.require_string(
                    message["tool_call_id"],
                    f"{path}/tool_call_id",
                    non_empty=True,
                )
            if audit.required(message, "content", path):
                _validate_chat_content(
                    message["content"],
                    f"{path}/content",
                    audit,
                    allowed_types={"text"},
                )
        if protocol == "openai_chat" and role == "assistant":
            has_content = "content" in message and message["content"] is not None
            has_calls = (
                message.get("tool_calls") not in (None, [])
                or message.get("function_call") is not None
            )
            if not has_content and not has_calls:
                audit.add(
                    f"{path}/content",
                    "MISSING_FIELD",
                    "content unless tool_calls or function_call is present",
                )
            if "content" in message:
                _validate_chat_content(
                    message["content"],
                    f"{path}/content",
                    audit,
                    allowed_types={"text", "refusal"},
                    nullable=True,
                )
                content_parts = message["content"]
                if isinstance(content_parts, list):
                    part_types = [
                        part.get("type")
                        for part in content_parts
                        if isinstance(part, dict)
                    ]
                    if "refusal" in part_types and part_types != ["refusal"]:
                        audit.add(
                            f"{path}/content",
                            "VALUE_MISMATCH",
                            "one refusal part or one or more text parts",
                        )
            if "tool_calls" in message:
                _validate_history_tool_calls(
                    protocol,
                    message["tool_calls"],
                    f"{path}/tool_calls",
                    audit,
                )
            if "function_call" in message and message["function_call"] is not None:
                function_path = f"{path}/function_call"
                function = audit.require_mapping(message["function_call"], function_path)
                if function is not None:
                    for field in ("name", "arguments"):
                        if audit.required(function, field, function_path):
                            audit.require_string(
                                function[field],
                                f"{function_path}/{field}",
                            )
            if "refusal" in message and message["refusal"] is not None:
                audit.require_string(message["refusal"], f"{path}/refusal")
        if protocol == "openai_chat" and role in {"system", "developer"}:
            if audit.required(message, "content", path):
                _validate_chat_content(
                    message["content"],
                    f"{path}/content",
                    audit,
                    allowed_types={"text"},
                )
        if protocol == "openai_chat" and role == "user":
            if audit.required(message, "content", path):
                _validate_chat_content(
                    message["content"],
                    f"{path}/content",
                    audit,
                    allowed_types={"text", "image_url", "input_audio", "file"},
                )
        if protocol == "openai_chat" and role == "function":
            if audit.required(message, "content", path):
                content = message["content"]
                if content is not None:
                    audit.require_string(content, f"{path}/content")
            if audit.required(message, "name", path):
                audit.require_string(message["name"], f"{path}/name", non_empty=True)
        if protocol == "ollama_chat":
            if "tool_call_id" in message:
                audit.require_string(
                    message["tool_call_id"],
                    f"{path}/tool_call_id",
                    non_empty=True,
                )
            if role == "tool":
                if audit.required(message, "tool_name", path):
                    audit.require_string(message["tool_name"], f"{path}/tool_name", non_empty=True)
                if audit.required(message, "content", path):
                    audit.require_string(message["content"], f"{path}/content")
            elif audit.required(message, "content", path):
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


_GEMINI_PART_PAYLOADS = {
    "text",
    "inlineData",
    "functionCall",
    "functionResponse",
    "fileData",
    "executableCode",
    "codeExecutionResult",
    "toolCall",
    "toolResponse",
}


def _validate_gemini_part(value: object, location: str, audit: _Audit) -> None:
    part = audit.require_mapping(value, location)
    if part is None:
        return
    if "thoughtSignature" in part:
        audit.require_string(
            part["thoughtSignature"],
            f"{location}/thoughtSignature",
            non_empty=True,
        )
    if "thought" in part and not isinstance(part["thought"], bool):
        audit.add(f"{location}/thought", "TYPE_MISMATCH", "boolean")
    payloads = sorted(_GEMINI_PART_PAYLOADS.intersection(part))
    if len(payloads) != 1:
        difference_kind = "MISSING_FIELD" if not payloads else "VALUE_MISMATCH"
        audit.add(
            location,
            difference_kind,
            "exactly one provider-native Part payload",
        )
    for payload in payloads:
        payload_path = f"{location}/{payload}"
        payload_value = part[payload]
        if payload == "text":
            audit.require_string(payload_value, payload_path)
        elif payload == "inlineData":
            blob = audit.require_mapping(payload_value, payload_path)
            if blob is not None:
                for field in ("mimeType", "data"):
                    if audit.required(blob, field, payload_path):
                        audit.require_string(
                            blob[field],
                            f"{payload_path}/{field}",
                            non_empty=True,
                        )
        elif payload == "fileData":
            file_data = audit.require_mapping(payload_value, payload_path)
            if file_data is not None:
                if audit.required(file_data, "fileUri", payload_path):
                    audit.require_string(
                        file_data["fileUri"],
                        f"{payload_path}/fileUri",
                        non_empty=True,
                    )
                if "mimeType" in file_data:
                    audit.require_string(
                        file_data["mimeType"],
                        f"{payload_path}/mimeType",
                        non_empty=True,
                    )
        elif payload == "executableCode":
            code = audit.require_mapping(payload_value, payload_path)
            if code is not None:
                if audit.required(code, "language", payload_path):
                    language = code["language"]
                    if language not in {"LANGUAGE_UNSPECIFIED", "PYTHON"}:
                        audit.add(
                            f"{payload_path}/language",
                            "VALUE_MISMATCH",
                            "official Gemini executable-code language",
                        )
                if audit.required(code, "code", payload_path):
                    audit.require_string(code["code"], f"{payload_path}/code")
                if "id" in code:
                    audit.require_string(code["id"], f"{payload_path}/id")
        elif payload == "codeExecutionResult":
            result = audit.require_mapping(payload_value, payload_path)
            if result is not None:
                if audit.required(result, "outcome", payload_path):
                    outcome = result["outcome"]
                    if outcome not in {
                        "OUTCOME_UNSPECIFIED",
                        "OUTCOME_OK",
                        "OUTCOME_FAILED",
                        "OUTCOME_DEADLINE_EXCEEDED",
                    }:
                        audit.add(
                            f"{payload_path}/outcome",
                            "VALUE_MISMATCH",
                            "official Gemini execution outcome",
                        )
                for field in ("id", "output"):
                    if field in result:
                        audit.require_string(result[field], f"{payload_path}/{field}")
        elif payload == "functionCall":
            call = audit.require_mapping(payload_value, payload_path)
            if call is not None:
                if audit.required(call, "name", payload_path):
                    audit.require_string(
                        call["name"],
                        f"{payload_path}/name",
                        non_empty=True,
                    )
                if "args" in call:
                    audit.require_mapping(call["args"], f"{payload_path}/args")
                if "id" in call:
                    audit.require_string(
                        call["id"],
                        f"{payload_path}/id",
                        non_empty=True,
                    )
        elif payload == "functionResponse":
            response = audit.require_mapping(payload_value, payload_path)
            if response is not None:
                if audit.required(response, "name", payload_path):
                    audit.require_string(
                        response["name"],
                        f"{payload_path}/name",
                        non_empty=True,
                    )
                if audit.required(response, "response", payload_path):
                    audit.require_mapping(
                        response["response"],
                        f"{payload_path}/response",
                    )
                if "id" in response:
                    audit.require_string(
                        response["id"],
                        f"{payload_path}/id",
                        non_empty=True,
                    )
        else:
            audit.require_mapping(payload_value, payload_path)


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
            _validate_gemini_part(
                part_value,
                f"{path}/parts/{part_index}",
                audit,
            )


def _validate_responses_output(
    value: object,
    location: str,
    audit: _Audit,
) -> None:
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
        part_type = part.get("type")
        if part_type not in {"input_text", "input_image", "input_file"}:
            difference_kind = "MISSING_FIELD" if "type" not in part else "VALUE_MISMATCH"
            audit.add(
                f"{path}/type",
                difference_kind,
                '"input_text", "input_image", or "input_file"',
            )
            continue
        if part_type == "input_text":
            if audit.required(part, "text", path):
                audit.require_string(part["text"], f"{path}/text")
            continue
        nullable_string_fields = (
            ("image_url", "file_id")
            if part_type == "input_image"
            else ("file_id", "filename", "file_data", "file_url")
        )
        for field in nullable_string_fields:
            if field in part and part[field] is not None:
                audit.require_string(part[field], f"{path}/{field}")
        if "detail" in part and part["detail"] is not None:
            allowed_details = (
                {"auto", "low", "high", "original"}
                if part_type == "input_image"
                else {"auto", "low", "high"}
            )
            if part["detail"] not in allowed_details:
                audit.add(
                    f"{path}/detail",
                    "VALUE_MISMATCH",
                    "official OpenAI input detail value",
                )


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
            _validate_responses_output(item["output"], f"{path}/output", audit)


def _non_empty_content(value: object) -> bool:
    if isinstance(value, str):
        return bool(value.strip())
    if isinstance(value, list):
        return bool(value)
    return False


def _validate_allowed_tools(
    value: object,
    location: str,
    audit: _Audit,
) -> None:
    allowed = audit.require_mapping(value, location)
    if allowed is None:
        return
    if audit.required(allowed, "mode", location):
        if allowed["mode"] not in {"auto", "required"}:
            audit.add(
                f"{location}/mode",
                "VALUE_MISMATCH",
                '"auto" or "required"',
            )
    if not audit.required(allowed, "tools", location):
        return
    tools = audit.require_list(allowed["tools"], f"{location}/tools")
    if tools is None:
        return
    for index, tool in enumerate(tools):
        audit.require_mapping(tool, f"{location}/tools/{index}")


def _validate_openai_tool_choice(
    protocol: str,
    choice: object,
    audit: _Audit,
) -> None:
    location = "/requestBody/tool_choice"
    if isinstance(choice, str):
        if choice not in {"none", "auto", "required"}:
            audit.add(
                location,
                "VALUE_MISMATCH",
                '"none", "auto", "required", or provider-native object',
            )
        return
    value = audit.require_mapping(choice, location)
    if value is None:
        return
    if not audit.required(value, "type", location):
        return
    choice_type = value["type"]
    if not audit.require_string(choice_type, f"{location}/type", non_empty=True):
        return
    if protocol == "openai_chat":
        if choice_type in {"function", "custom"}:
            field = choice_type
            if not audit.required(value, field, location):
                return
            named = audit.require_mapping(value[field], f"{location}/{field}")
            if named is not None and audit.required(named, "name", f"{location}/{field}"):
                audit.require_string(
                    named["name"],
                    f"{location}/{field}/name",
                    non_empty=True,
                )
        elif choice_type == "allowed_tools":
            if audit.required(value, "allowed_tools", location):
                _validate_allowed_tools(
                    value["allowed_tools"],
                    f"{location}/allowed_tools",
                    audit,
                )
        else:
            audit.add(
                f"{location}/type",
                "VALUE_MISMATCH",
                '"function", "custom", or "allowed_tools"',
            )
        return
    hosted_types = {
        "file_search",
        "web_search_preview",
        "computer",
        "computer_use_preview",
        "computer_use",
        "web_search_preview_2025_03_11",
        "image_generation",
        "code_interpreter",
        "programmatic_tool_calling",
        "apply_patch",
        "shell",
    }
    if choice_type in {"function", "custom"}:
        if audit.required(value, "name", location):
            audit.require_string(value["name"], f"{location}/name", non_empty=True)
    elif choice_type == "mcp":
        if audit.required(value, "server_label", location):
            audit.require_string(
                value["server_label"],
                f"{location}/server_label",
                non_empty=True,
            )
        if "name" in value and value["name"] is not None:
            audit.require_string(value["name"], f"{location}/name", non_empty=True)
    elif choice_type == "allowed_tools":
        _validate_allowed_tools(value, location, audit)
    elif choice_type not in hosted_types:
        audit.add(
            f"{location}/type",
            "VALUE_MISMATCH",
            "official OpenAI Responses tool-choice type",
        )


def _validate_openai_controls(protocol: str, body: dict, audit: _Audit) -> None:
    if "parallel_tool_calls" in body and not isinstance(
        body["parallel_tool_calls"], bool
    ):
        audit.add(
            "/requestBody/parallel_tool_calls",
            "TYPE_MISMATCH",
            "boolean",
        )
    if "tool_choice" in body:
        _validate_openai_tool_choice(protocol, body["tool_choice"], audit)


def _validate_anthropic_controls(body: dict, audit: _Audit) -> None:
    if "max_tokens" not in body:
        audit.add("/requestBody/max_tokens", "MISSING_FIELD", "positive integer")
        return
    value = body["max_tokens"]
    if not isinstance(value, int) or isinstance(value, bool):
        audit.add("/requestBody/max_tokens", "TYPE_MISMATCH", "positive integer")
    elif value <= 0:
        audit.add("/requestBody/max_tokens", "VALUE_MISMATCH", "positive integer")


def _validate_initial_prompt(protocol: str, body: dict, audit: _Audit) -> None:
    if protocol != "gemini_generate_content":
        model = body.get("model")
        if not isinstance(model, str) or not model.strip():
            kind = "MISSING_FIELD" if "model" not in body else "VALUE_MISMATCH"
            audit.add("/requestBody/model", kind, "non-empty model string")
    if protocol == "openai_responses" and "previous_response_id" in body:
        audit.add(
            "/requestBody/previous_response_id",
            "UNEXPECTED_FIELD",
            "absent from the initial request",
        )

    if protocol in {"openai_chat", "anthropic_messages", "ollama_chat"}:
        messages = body.get("messages")
        first = messages[0] if isinstance(messages, list) and messages else None
        if not (
            isinstance(first, dict)
            and first.get("role") == "user"
            and _non_empty_content(first.get("content"))
        ):
            audit.add(
                "/requestBody/messages",
                "VALUE_MISMATCH",
                "first user message with non-empty prompt content",
            )
        return
    if protocol == "openai_responses":
        if not _non_empty_content(body.get("input")):
            audit.add(
                "/requestBody/input",
                "VALUE_MISMATCH",
                "non-empty initial input prompt",
            )
        return
    if protocol == "gemini_generate_content":
        contents = body.get("contents")
        first = contents[0] if isinstance(contents, list) and contents else None
        parts = first.get("parts") if isinstance(first, dict) else None
        has_text = isinstance(parts, list) and any(
            isinstance(part, dict)
            and isinstance(part.get("text"), str)
            and bool(part["text"].strip())
            for part in parts
        )
        if not (
            isinstance(first, dict)
            and first.get("role") == "user"
            and has_text
        ):
            audit.add(
                "/requestBody/contents",
                "VALUE_MISMATCH",
                "first user content with a non-empty text part",
            )


def _validate_request_body(
    request_id: str,
    request: object,
    *,
    initial: bool,
) -> tuple[Optional[dict], list[dict]]:
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
            {"input", "messages", "parallel_tool_calls", "previous_response_id", "store", "stream", "tool_choice"},
            "/requestBody",
        )
        _validate_contents(body, audit)
    else:
        if body.get("stream") is not True:
            kind = "MISSING_FIELD" if "stream" not in body else "VALUE_MISMATCH"
            audit.add("/requestBody/stream", kind, "true")
        if protocol == "openai_chat":
            audit.reject(body, {"contents", "input", "previous_response_id"}, "/requestBody")
            _validate_openai_controls(protocol, body, audit)
            _validate_messages(protocol, body, audit)
        elif protocol == "openai_responses":
            audit.reject(body, {"contents", "messages"}, "/requestBody")
            _validate_openai_controls(protocol, body, audit)
            if body.get("store") is not True:
                kind = "MISSING_FIELD" if "store" not in body else "VALUE_MISMATCH"
                audit.add("/requestBody/store", kind, "true")
            if "input" not in body:
                audit.add("/requestBody/input", "MISSING_FIELD", "string or function_call_output array")
            else:
                _validate_responses_input(body["input"], audit)
        elif protocol == "anthropic_messages":
            audit.reject(
                body,
                {"contents", "input", "parallel_tool_calls", "previous_response_id", "store"},
                "/requestBody",
            )
            _validate_anthropic_controls(body, audit)
            _validate_messages(protocol, body, audit)
        elif protocol == "ollama_chat":
            audit.reject(
                body,
                {"contents", "input", "parallel_tool_calls", "previous_response_id", "store", "tool_choice"},
                "/requestBody",
            )
            _validate_messages(protocol, body, audit)
    _validate_tools(protocol, body, audit)
    if initial:
        _validate_initial_prompt(protocol, body, audit)
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
            decoded = strict_json_loads(joined)
        except JSON_LOAD_ERRORS:
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
            decoded = strict_json_loads(line)
        except JSON_LOAD_ERRORS:
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
                block["input"] = strict_json_loads(json_fragments[index])
            except JSON_LOAD_ERRORS:
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
                _validate_responses_output(item["output"], f"{path}/output", audit)
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
        if not isinstance(results, list) or len(results) < len(calls):
            audit.add(
                f"/requestBody/messages/{len(before) + 1}/content",
                "VALUE_MISMATCH",
                "one leading tool_result per streamed tool_use",
            )
            return
        timeout = test_id == "049" and transition == 1
        for index, (call, result) in enumerate(zip(calls, results[: len(calls)])):
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
        for index, result in enumerate(results[len(calls) :], start=len(calls)):
            path = f"/requestBody/messages/{len(before) + 1}/content/{index}"
            if not isinstance(result, dict) or result.get("type") != "text":
                audit.add(
                    path,
                    "VALUE_MISMATCH",
                    "only text blocks after all tool_result blocks",
                )
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
            call_id = call.get("id")
            if isinstance(call_id, str) and call_id:
                if "tool_call_id" in item:
                    _correlate(
                        item.get("tool_call_id"),
                        call_id,
                        f"{path}/tool_call_id",
                        audit,
                        "matching optional streamed tool call ID",
                    )
            elif "tool_call_id" in item:
                audit.add(
                    f"{path}/tool_call_id",
                    "UNEXPECTED_FIELD",
                    "absent when the streamed call has no ID",
                )
            _correlate(item.get("tool_name"), call.get("name"), f"{path}/tool_name", audit, "matching streamed function name")
            if audit.required(item, "content", path):
                audit.require_string(item["content"], f"{path}/content")


def _difference_identity(difference: Mapping[str, object]) -> tuple[object, ...]:
    return tuple(
        difference.get(field)
        for field in ("requestId", "protocol", "location", "differenceKind")
    )


def _append_unique(
    target: list[dict[str, object]],
    additions: list[dict[str, object]],
) -> None:
    identities = {_difference_identity(difference) for difference in target}
    for difference in additions:
        identity = _difference_identity(difference)
        if identity in identities:
            continue
        target.append(difference)
        identities.add(identity)


def validate_tool_loop_transitions(
    parsed: Mapping[str, object],
) -> dict[str, list[dict[str, object]]]:
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
    differences: dict[str, list[dict[str, object]]] = {}
    for test_id in sorted(V3_TOOL_TEST_IDS):
        manifest = tests.get(test_id)
        if not isinstance(manifest, dict):
            continue
        refs = manifest.get("requestRefs")
        if not isinstance(refs, list) or not all(isinstance(ref, str) for ref in refs):
            continue
        decoded: dict[str, Optional[dict]] = {}
        for position, request_id in enumerate(refs):
            request = requests.get(request_id)
            body, body_differences = _validate_request_body(
                request_id,
                request,
                initial=position == 0,
            )
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
