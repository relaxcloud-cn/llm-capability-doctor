"""Audit collected responses against pinned official protocol shapes.

The module deliberately has no I/O.  It consumes parsed Model Doctor evidence
and returns a deterministic, self-validating report suitable for embedding in
an assessment.
"""

from __future__ import annotations

from collections import Counter, defaultdict
from typing import Dict, List, Mapping, Optional, Sequence, Tuple

from model_doctor_json import JSON_LOAD_ERRORS, strict_json_loads
from model_doctor_tool_loop_conformance import validate_tool_loop_transitions


RULE_SET_VERSION = "official-protocol-conformance.2026-08-18"
BASELINE_DATE = "2026-08-18"
SUPPORTED_PROTOCOLS = (
    "openai_chat",
    "openai_responses",
    "anthropic_messages",
    "gemini_generate_content",
    "ollama_chat",
)
DIFFERENCE_KINDS = (
    "EVIDENCE_GAP",
    "FRAMING",
    "INVALID_JSON",
    "MISSING_FIELD",
    "UNEXPECTED_FIELD",
    "TYPE_MISMATCH",
    "ENUM_MISMATCH",
    "VALUE_MISMATCH",
    "SEQUENCE",
    "CORRELATION",
)


_OPENAI_SOURCE = (
    "https://github.com/openai/openai-openapi/blob/"
    "2186421dca0cca7c1e67caa7739005e8b1ccc4dd/openapi.json"
)
_BASELINES = (
    {
        "protocol": "openai_chat",
        "officialVersion": "OpenAPI 2.3.0, commit 2186421dca0cca7c1e67caa7739005e8b1ccc4dd",
        "referenceDate": BASELINE_DATE,
        "sourceUrl": _OPENAI_SOURCE,
        "supportingReferences": [
            "https://platform.openai.com/docs/api-reference/chat/object",
            "https://platform.openai.com/docs/api-reference/chat/streaming",
            "https://platform.openai.com/docs/guides/error-codes/api-errors",
        ],
    },
    {
        "protocol": "openai_responses",
        "officialVersion": "OpenAPI 2.3.0, commit 2186421dca0cca7c1e67caa7739005e8b1ccc4dd",
        "referenceDate": BASELINE_DATE,
        "sourceUrl": _OPENAI_SOURCE,
        "supportingReferences": [
            "https://platform.openai.com/docs/api-reference/responses/object",
            "https://platform.openai.com/docs/api-reference/responses-streaming",
            "https://platform.openai.com/docs/guides/error-codes/api-errors",
        ],
    },
    {
        "protocol": "anthropic_messages",
        "officialVersion": "anthropic-version: 2023-06-01",
        "referenceDate": BASELINE_DATE,
        "sourceUrl": "https://platform.claude.com/docs/en/api/messages/create",
        "supportingReferences": [
            "https://platform.claude.com/docs/en/api/messages-streaming",
            "https://platform.claude.com/docs/en/api/errors",
        ],
    },
    {
        "protocol": "gemini_generate_content",
        "officialVersion": "v1beta Discovery revision 20260816",
        "referenceDate": BASELINE_DATE,
        "sourceUrl": "https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta",
        "supportingReferences": [
            "https://ai.google.dev/api/generate-content",
            "https://ai.google.dev/api/rest/generativelanguage",
        ],
    },
    {
        "protocol": "ollama_chat",
        "officialVersion": "API types commit d67ad83426633195089509347ffd4fe795120198",
        "referenceDate": BASELINE_DATE,
        "sourceUrl": (
            "https://github.com/ollama/ollama/blob/"
            "d67ad83426633195089509347ffd4fe795120198/api/types.go"
        ),
        "supportingReferences": [
            "https://github.com/ollama/ollama/blob/d67ad83426633195089509347ffd4fe795120198/docs/openapi.yaml",
            "https://docs.ollama.com/api/chat",
            "https://docs.ollama.com/api/errors",
        ],
    },
)
_REFERENCES = {item["protocol"]: item["sourceUrl"] for item in _BASELINES}
_EVIDENCE_REFERENCE = "https://www.rfc-editor.org/rfc/rfc8259"

_REPORT_FIELDS = {
    "ruleSetVersion",
    "baselineDate",
    "baselines",
    "summary",
    "results",
}
_BASELINE_FIELDS = {
    "protocol",
    "officialVersion",
    "referenceDate",
    "sourceUrl",
    "supportingReferences",
}
_SUMMARY_FIELDS = {
    "totalRequests",
    "checkedRequests",
    "consistentRequests",
    "differentRequests",
    "byProtocol",
    "byCheck",
    "byDifferenceKind",
}
_GROUP_FIELDS = {
    "protocol": {"protocol", "totalRequests", "consistentRequests", "differentRequests"},
    "check": {"checkId", "totalRequests", "consistentRequests", "differentRequests"},
    "difference": {"differenceKind", "count"},
}
_RESULT_FIELDS = {
    "requestId",
    "protocol",
    "checkIds",
    "stream",
    "httpStatus",
    "status",
    "differences",
}
_DIFFERENCE_FIELDS = {
    "requestId",
    "protocol",
    "location",
    "differenceKind",
    "expected",
    "actual",
    "officialReference",
}


def _json_pointer_token(value: object) -> str:
    return str(value).replace("~", "~0").replace("/", "~1")


def _path(parent: str, field: object) -> str:
    return f"{parent}/{_json_pointer_token(field)}" if parent else f"/{_json_pointer_token(field)}"


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


def _is_number(value: object) -> bool:
    return (
        isinstance(value, (int, float))
        and not isinstance(value, bool)
    )


def _reference(protocol: object) -> str:
    if isinstance(protocol, str):
        return _REFERENCES.get(protocol, _EVIDENCE_REFERENCE)
    return _EVIDENCE_REFERENCE


_ACTUAL_TYPES = {
    "null",
    "boolean",
    "integer",
    "number",
    "string",
    "array",
    "object",
}
_EVIDENCE_ACTUALS = {
    "empty or missing",
    "invalid exit code",
    "invalid status",
    "missing",
    "non-zero exit code",
    "profile validation deferred",
    "required evidence unavailable or invalid",
    "unknown protocol",
}


def _bounded_actual(kind: str, actual: object) -> str:
    """Return a useful category without copying provider-controlled values."""

    if kind == "MISSING_FIELD":
        return "missing"
    if kind == "UNEXPECTED_FIELD":
        return "present"
    if kind == "ENUM_MISMATCH":
        return "value outside allowed enum"
    if kind == "VALUE_MISMATCH":
        return "value differs from required value"
    if kind == "CORRELATION":
        return "correlated values differ"
    if kind == "SEQUENCE":
        return "event sequence differs from required lifecycle"
    if kind == "FRAMING":
        return "stream framing differs from required format"
    if kind == "INVALID_JSON":
        return "invalid JSON"
    if kind == "TYPE_MISMATCH":
        if isinstance(actual, str) and actual in _ACTUAL_TYPES:
            return actual
        prefix = "string containing "
        if isinstance(actual, str) and actual.startswith(prefix):
            contained_type = actual[len(prefix) :]
            if contained_type in _ACTUAL_TYPES:
                return actual
        return "value of unexpected type"
    if kind == "EVIDENCE_GAP":
        if isinstance(actual, str) and (
            actual in _ACTUAL_TYPES or actual in _EVIDENCE_ACTUALS
        ):
            return actual
        return "required evidence unavailable or invalid"
    return "value differs from required structure"


def _difference(
    request_id: str,
    protocol: Optional[str],
    location: str,
    kind: str,
    expected: str,
    actual: str,
) -> dict:
    return {
        "requestId": request_id,
        "protocol": protocol,
        "location": location or "/responseBody",
        "differenceKind": kind,
        "expected": expected,
        "actual": _bounded_actual(kind, actual),
        "officialReference": _reference(protocol),
    }


class _ChatValidator:
    """Collect strict Chat Completion differences without cascading failures."""

    def __init__(self, request_id: str, protocol: str) -> None:
        self.request_id = request_id
        self.protocol = protocol
        self.differences: List[dict] = []

    def add(
        self,
        location: str,
        kind: str,
        expected: str,
        actual: str,
    ) -> None:
        self.differences.append(
            _difference(
                self.request_id,
                self.protocol,
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
        required: Sequence[str],
        optional: Sequence[str] = (),
    ) -> bool:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return False
        fields = set(value)
        for field in sorted(set(required) - fields):
            self.add(
                _path(location, field),
                "MISSING_FIELD",
                "present",
                "missing",
            )
        for field in sorted(fields - set(required) - set(optional)):
            self.add(
                _path(location, field),
                "UNEXPECTED_FIELD",
                "absent from the pinned official object",
                "present",
            )
        return True

    def typed(
        self,
        value: object,
        location: str,
        expected: str,
        predicate: object,
    ) -> bool:
        if not predicate(value):
            self.add(location, "TYPE_MISMATCH", expected, _actual_type(value))
            return False
        return True

    def string(self, value: object, location: str) -> bool:
        return self.typed(value, location, "string", lambda item: isinstance(item, str))

    def nullable_string(self, value: object, location: str) -> bool:
        return self.typed(
            value,
            location,
            "string or null",
            lambda item: item is None or isinstance(item, str),
        )

    def integer(self, value: object, location: str) -> bool:
        return self.typed(value, location, "integer", _is_integer)

    def enum(self, value: object, location: str, allowed: Sequence[str]) -> bool:
        if value not in allowed or not isinstance(value, str):
            actual = value if isinstance(value, str) and len(value) <= 80 else _actual_type(value)
            self.add(
                location,
                "ENUM_MISMATCH",
                "one of " + ", ".join(allowed),
                actual,
            )
            return False
        return True

    def validate_success(self, value: object) -> List[dict]:
        if not self.object_shape(
            value,
            "",
            ("id", "object", "created", "model", "choices"),
            ("service_tier", "system_fingerprint", "usage"),
        ):
            return self.differences
        assert isinstance(value, dict)

        if "id" in value:
            self.string(value["id"], "/id")
        if "object" in value:
            self.enum(value["object"], "/object", ("chat.completion",))
        if "created" in value:
            self.integer(value["created"], "/created")
        if "model" in value:
            self.string(value["model"], "/model")
        if "service_tier" in value:
            self.nullable_string(value["service_tier"], "/service_tier")
        if "system_fingerprint" in value:
            self.nullable_string(value["system_fingerprint"], "/system_fingerprint")
        if "choices" in value:
            choices = value["choices"]
            if self.typed(choices, "/choices", "array", lambda item: isinstance(item, list)):
                for index, choice in enumerate(choices):
                    self.validate_choice(choice, f"/choices/{index}")
        if "usage" in value:
            self.validate_usage(value["usage"], "/usage")
        return self.differences

    def validate_choice(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("index", "message", "finish_reason"),
            ("logprobs",),
        ):
            return
        assert isinstance(value, dict)
        if "index" in value:
            self.integer(value["index"], _path(location, "index"))
        if "message" in value:
            self.validate_message(value["message"], _path(location, "message"))
        if "finish_reason" in value:
            self.enum(
                value["finish_reason"],
                _path(location, "finish_reason"),
                ("stop", "length", "tool_calls", "content_filter", "function_call"),
            )
        if "logprobs" in value and value["logprobs"] is not None:
            self.validate_logprobs(value["logprobs"], _path(location, "logprobs"))

    def validate_message(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("role", "content"),
            ("refusal", "annotations", "tool_calls", "audio", "function_call"),
        ):
            return
        assert isinstance(value, dict)
        if "role" in value:
            self.enum(value["role"], _path(location, "role"), ("assistant",))
        if "content" in value:
            self.nullable_string(value["content"], _path(location, "content"))
        if "refusal" in value:
            self.nullable_string(value["refusal"], _path(location, "refusal"))
        if "annotations" in value:
            annotations = value["annotations"]
            annotations_path = _path(location, "annotations")
            if self.typed(annotations, annotations_path, "array", lambda item: isinstance(item, list)):
                for index, annotation in enumerate(annotations):
                    self.validate_annotation(annotation, f"{annotations_path}/{index}")
        if "tool_calls" in value:
            tool_calls = value["tool_calls"]
            tool_path = _path(location, "tool_calls")
            if self.typed(tool_calls, tool_path, "array", lambda item: isinstance(item, list)):
                for index, tool_call in enumerate(tool_calls):
                    self.validate_tool_call(tool_call, f"{tool_path}/{index}")
        if "audio" in value and value["audio"] is not None:
            self.validate_audio(value["audio"], _path(location, "audio"))
        if "function_call" in value:
            self.validate_deprecated_function_call(
                value["function_call"], _path(location, "function_call")
            )

    def validate_tool_call(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("id", "type", "function")):
            return
        assert isinstance(value, dict)
        if "id" in value:
            self.string(value["id"], _path(location, "id"))
        function_variant = False
        if "type" in value:
            function_variant = self.enum(
                value["type"], _path(location, "type"), ("function",)
            )
        if "function" in value and (function_variant or value.get("type") == "function"):
            function = value["function"]
            function_path = _path(location, "function")
            if not self.object_shape(function, function_path, ("name", "arguments")):
                return
            assert isinstance(function, dict)
            if "name" in function:
                self.string(function["name"], _path(function_path, "name"))
            if "arguments" in function:
                self.validate_arguments(
                    function["arguments"], _path(function_path, "arguments")
                )

    def validate_arguments(self, value: object, location: str) -> None:
        if not isinstance(value, str):
            self.add(
                location,
                "TYPE_MISMATCH",
                "string containing one JSON object",
                _actual_type(value),
            )
            return
        try:
            parsed = strict_json_loads(value)
        except JSON_LOAD_ERRORS:
            self.add(
                location,
                "INVALID_JSON",
                "string containing one JSON object",
                "string containing invalid JSON",
            )
            return
        if not isinstance(parsed, dict):
            self.add(
                location,
                "TYPE_MISMATCH",
                "string containing one JSON object",
                f"string containing {_actual_type(parsed)}",
            )

    def validate_deprecated_function_call(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("name", "arguments")):
            return
        assert isinstance(value, dict)
        if "name" in value:
            self.string(value["name"], _path(location, "name"))
        if "arguments" in value:
            self.typed(
                value["arguments"],
                _path(location, "arguments"),
                "string",
                lambda item: isinstance(item, str),
            )

    def validate_annotation(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("type", "url_citation")):
            return
        assert isinstance(value, dict)
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("url_citation",))
        if "url_citation" not in value:
            return
        citation = value["url_citation"]
        citation_path = _path(location, "url_citation")
        if not self.object_shape(
            citation,
            citation_path,
            ("end_index", "start_index", "title", "url"),
        ):
            return
        assert isinstance(citation, dict)
        for field in ("end_index", "start_index"):
            if field in citation:
                self.integer(citation[field], _path(citation_path, field))
        for field in ("title", "url"):
            if field in citation:
                self.string(citation[field], _path(citation_path, field))

    def validate_audio(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("id", "data", "expires_at", "transcript"),
        ):
            return
        assert isinstance(value, dict)
        for field in ("id", "data", "transcript"):
            if field in value:
                self.string(value[field], _path(location, field))
        if "expires_at" in value:
            self.integer(value["expires_at"], _path(location, "expires_at"))

    def validate_logprobs(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("content", "refusal")):
            return
        assert isinstance(value, dict)
        for field in ("content", "refusal"):
            if field not in value or value[field] is None:
                continue
            items = value[field]
            item_path = _path(location, field)
            if self.typed(items, item_path, "array or null", lambda item: isinstance(item, list)):
                for index, item in enumerate(items):
                    self.validate_token_logprob(item, f"{item_path}/{index}")

    def validate_token_logprob(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("token", "logprob", "bytes", "top_logprobs"),
        ):
            return
        assert isinstance(value, dict)
        if "token" in value:
            self.string(value["token"], _path(location, "token"))
        if "logprob" in value:
            self.typed(value["logprob"], _path(location, "logprob"), "number", _is_number)
        if "bytes" in value and value["bytes"] is not None:
            bytes_value = value["bytes"]
            bytes_path = _path(location, "bytes")
            if self.typed(bytes_value, bytes_path, "integer array or null", lambda item: isinstance(item, list)):
                for index, byte in enumerate(bytes_value):
                    self.integer(byte, f"{bytes_path}/{index}")
        if "top_logprobs" in value:
            top = value["top_logprobs"]
            top_path = _path(location, "top_logprobs")
            if self.typed(top, top_path, "array", lambda item: isinstance(item, list)):
                for index, item in enumerate(top):
                    self.validate_top_logprob(item, f"{top_path}/{index}")

    def validate_top_logprob(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("token", "logprob", "bytes")):
            return
        assert isinstance(value, dict)
        if "token" in value:
            self.string(value["token"], _path(location, "token"))
        if "logprob" in value:
            self.typed(value["logprob"], _path(location, "logprob"), "number", _is_number)
        if "bytes" in value and value["bytes"] is not None:
            bytes_value = value["bytes"]
            bytes_path = _path(location, "bytes")
            if self.typed(bytes_value, bytes_path, "integer array or null", lambda item: isinstance(item, list)):
                for index, byte in enumerate(bytes_value):
                    self.integer(byte, f"{bytes_path}/{index}")

    def validate_usage(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("prompt_tokens", "completion_tokens", "total_tokens"),
            ("prompt_tokens_details", "completion_tokens_details"),
        ):
            return
        assert isinstance(value, dict)
        for field in ("prompt_tokens", "completion_tokens", "total_tokens"):
            if field in value:
                self.integer(value[field], _path(location, field))
        if "prompt_tokens_details" in value:
            self.validate_integer_details(
                value["prompt_tokens_details"],
                _path(location, "prompt_tokens_details"),
                ("audio_tokens", "cached_tokens"),
            )
        if "completion_tokens_details" in value:
            self.validate_integer_details(
                value["completion_tokens_details"],
                _path(location, "completion_tokens_details"),
                (
                    "accepted_prediction_tokens",
                    "audio_tokens",
                    "reasoning_tokens",
                    "rejected_prediction_tokens",
                ),
            )

    def validate_integer_details(
        self,
        value: object,
        location: str,
        fields: Sequence[str],
    ) -> None:
        if value is None:
            return
        if not self.object_shape(value, location, (), fields):
            return
        assert isinstance(value, dict)
        for field in fields:
            if field in value:
                self.integer(value[field], _path(location, field))

    def validate_error(self, value: object) -> List[dict]:
        if not self.object_shape(value, "", ("error",), ("request_id",)):
            return self.differences
        assert isinstance(value, dict)
        if "request_id" in value:
            self.string(value["request_id"], "/request_id")
        if "error" not in value:
            return self.differences
        error = value["error"]
        if not self.object_shape(
            error,
            "/error",
            ("type", "message", "param", "code"),
        ):
            return self.differences
        assert isinstance(error, dict)
        if "type" in error:
            self.string(error["type"], "/error/type")
        if "message" in error:
            self.string(error["message"], "/error/message")
        if "param" in error:
            self.nullable_string(error["param"], "/error/param")
        if "code" in error:
            self.nullable_string(error["code"], "/error/code")
        return self.differences


class _ResponsesValidator(_ChatValidator):
    """Validate non-stream Responses API objects against the pinned profile."""

    _ITEM_STATUSES = ("in_progress", "completed", "incomplete")

    def validate_success(self, value: object) -> List[dict]:
        required = (
            "id",
            "object",
            "created_at",
            "error",
            "incomplete_details",
            "instructions",
            "model",
            "tools",
            "output",
            "parallel_tool_calls",
            "metadata",
            "tool_choice",
            "temperature",
            "top_p",
        )
        optional = (
            "status",
            "completed_at",
            "usage",
            "previous_response_id",
            "reasoning",
            "text",
            "truncation",
            "max_output_tokens",
            "store",
            "service_tier",
            "user",
        )
        if not self.object_shape(value, "", required, optional):
            return self.differences
        assert isinstance(value, dict)

        if "id" in value:
            self.string(value["id"], "/id")
        if "object" in value:
            self.enum(value["object"], "/object", ("response",))
        if "created_at" in value:
            self.integer(value["created_at"], "/created_at")
        if "error" in value:
            self.validate_response_error(value["error"], "/error")
        if "incomplete_details" in value:
            self.validate_incomplete_details(
                value["incomplete_details"], "/incomplete_details"
            )
        if "instructions" in value:
            self.nullable_string(value["instructions"], "/instructions")
        if "model" in value:
            self.string(value["model"], "/model")
        if "tools" in value:
            tools = value["tools"]
            if self.typed(tools, "/tools", "array", lambda item: isinstance(item, list)):
                for index, tool in enumerate(tools):
                    self.validate_response_tool(tool, f"/tools/{index}")
        if "output" in value:
            output = value["output"]
            if self.typed(output, "/output", "array", lambda item: isinstance(item, list)):
                for index, item in enumerate(output):
                    self.validate_output_item(item, f"/output/{index}")
        if "parallel_tool_calls" in value:
            self.typed(
                value["parallel_tool_calls"],
                "/parallel_tool_calls",
                "boolean",
                lambda item: isinstance(item, bool),
            )
        if "metadata" in value:
            self.typed(
                value["metadata"],
                "/metadata",
                "object",
                lambda item: isinstance(item, dict),
            )
        if "tool_choice" in value:
            self.validate_tool_choice(value["tool_choice"], "/tool_choice")
        for field in ("temperature", "top_p"):
            if field in value:
                self.typed(
                    value[field],
                    _path("", field),
                    "number or null",
                    lambda item: item is None or _is_number(item),
                )
        if "status" in value:
            self.enum(
                value["status"],
                "/status",
                ("completed", "failed", "in_progress", "cancelled", "queued", "incomplete"),
            )
        if "completed_at" in value:
            self.typed(
                value["completed_at"],
                "/completed_at",
                "integer or null",
                lambda item: item is None or _is_integer(item),
            )
        if "usage" in value:
            self.validate_usage(value["usage"], "/usage")
        if "previous_response_id" in value:
            self.nullable_string(value["previous_response_id"], "/previous_response_id")
        if "reasoning" in value:
            self.validate_reasoning_config(value["reasoning"], "/reasoning")
        if "text" in value:
            self.validate_text_config(value["text"], "/text")
        if "truncation" in value:
            self.enum(value["truncation"], "/truncation", ("auto", "disabled"))
        if "max_output_tokens" in value:
            self.typed(
                value["max_output_tokens"],
                "/max_output_tokens",
                "integer or null",
                lambda item: item is None or _is_integer(item),
            )
        if "store" in value:
            self.typed(value["store"], "/store", "boolean", lambda item: isinstance(item, bool))
        if "service_tier" in value:
            self.nullable_string(value["service_tier"], "/service_tier")
        if "user" in value:
            self.string(value["user"], "/user")
        return self.differences

    def validate_response_error(self, value: object, location: str) -> None:
        if value is None:
            return
        if not self.object_shape(value, location, ("code", "message")):
            return
        assert isinstance(value, dict)
        if "code" in value:
            self.string(value["code"], _path(location, "code"))
        if "message" in value:
            self.string(value["message"], _path(location, "message"))

    def validate_incomplete_details(self, value: object, location: str) -> None:
        if value is None:
            return
        if not self.object_shape(value, location, ("reason",)):
            return
        assert isinstance(value, dict)
        if "reason" in value:
            self.enum(
                value["reason"],
                _path(location, "reason"),
                ("max_output_tokens", "content_filter"),
            )

    def validate_output_item(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        item_type = value["type"]
        if not self.enum(
            item_type,
            _path(location, "type"),
            (
                "custom_tool_call",
                "function_call",
                "message",
                "reasoning",
                "shell_call",
            ),
        ):
            return
        if item_type == "function_call":
            self.validate_function_call(value, location)
        elif item_type == "custom_tool_call":
            self.validate_custom_tool_call(value, location)
        elif item_type == "message":
            self.validate_response_message(value, location)
        elif item_type == "reasoning":
            self.validate_reasoning_item(value, location)
        else:
            self.validate_shell_call(value, location)

    def validate_tool_call_caller(self, value: object, location: str) -> None:
        if value is None:
            return
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object or null", _actual_type(value))
            return
        caller_type = value.get("type")
        if caller_type == "direct":
            self.object_shape(value, location, ("type",))
        elif caller_type == "program":
            if self.object_shape(value, location, ("type", "caller_id")):
                if "caller_id" in value:
                    self.string(value["caller_id"], _path(location, "caller_id"))
        elif "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
        else:
            self.add(
                _path(location, "type"),
                "ENUM_MISMATCH",
                "one of direct, program",
                "value outside allowed enum",
            )

    def validate_custom_tool_call(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("type", "call_id", "name", "input"),
            ("id", "caller", "namespace"),
        ):
            return
        assert isinstance(value, dict)
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("custom_tool_call",))
        for field in ("id", "call_id", "namespace", "name", "input"):
            if field in value:
                self.string(value[field], _path(location, field))
        if "caller" in value:
            self.validate_tool_call_caller(value["caller"], _path(location, "caller"))

    def validate_shell_call(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("type", "id", "call_id", "action", "status", "environment"),
            ("caller", "created_by"),
        ):
            return
        assert isinstance(value, dict)
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("shell_call",))
        for field in ("id", "call_id", "created_by"):
            if field in value:
                self.string(value[field], _path(location, field))
        if "status" in value:
            self.enum(value["status"], _path(location, "status"), self._ITEM_STATUSES)
        if "caller" in value:
            self.validate_tool_call_caller(value["caller"], _path(location, "caller"))
        if "action" in value:
            self.validate_shell_action(value["action"], _path(location, "action"))
        if "environment" in value:
            self.validate_shell_environment(
                value["environment"], _path(location, "environment")
            )

    def validate_shell_action(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("commands", "timeout_ms", "max_output_length"),
        ):
            return
        assert isinstance(value, dict)
        commands = value.get("commands")
        if self.typed(commands, _path(location, "commands"), "array", lambda item: isinstance(item, list)):
            assert isinstance(commands, list)
            for index, command in enumerate(commands):
                self.string(command, f"{_path(location, 'commands')}/{index}")
        for field in ("timeout_ms", "max_output_length"):
            if field in value and value[field] is not None:
                self.integer(value[field], _path(location, field))

    def validate_shell_environment(self, value: object, location: str) -> None:
        if value is None:
            return
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object or null", _actual_type(value))
            return
        environment_type = value.get("type")
        if environment_type == "local":
            self.object_shape(value, location, ("type",))
        elif environment_type == "container_reference":
            if self.object_shape(value, location, ("type", "container_id")):
                if "container_id" in value:
                    self.string(value["container_id"], _path(location, "container_id"))
        elif "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
        else:
            self.add(
                _path(location, "type"),
                "ENUM_MISMATCH",
                "one of local, container_reference",
                "value outside allowed enum",
            )

    def validate_function_call(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("type", "call_id", "name", "arguments"),
            ("id", "status"),
        ):
            return
        assert isinstance(value, dict)
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("function_call",))
        for field in ("call_id", "name"):
            if field in value:
                self.string(value[field], _path(location, field))
        if "arguments" in value:
            self.validate_arguments(value["arguments"], _path(location, "arguments"))
        if "id" in value:
            self.string(value["id"], _path(location, "id"))
        if "status" in value:
            self.enum(value["status"], _path(location, "status"), self._ITEM_STATUSES)

    def validate_response_message(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("id", "type", "role", "content", "status"),
            ("phase",),
        ):
            return
        assert isinstance(value, dict)
        if "id" in value:
            self.string(value["id"], _path(location, "id"))
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("message",))
        if "role" in value:
            self.enum(value["role"], _path(location, "role"), ("assistant",))
        if "content" in value:
            content = value["content"]
            content_path = _path(location, "content")
            if self.typed(content, content_path, "array", lambda item: isinstance(item, list)):
                for index, item in enumerate(content):
                    self.validate_message_content(item, f"{content_path}/{index}")
        if "status" in value:
            self.enum(value["status"], _path(location, "status"), self._ITEM_STATUSES)
        if "phase" in value:
            self.enum(
                value["phase"],
                _path(location, "phase"),
                ("commentary", "final_answer"),
            )

    def validate_message_content(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        content_type = value["type"]
        if not self.enum(
            content_type,
            _path(location, "type"),
            ("output_text", "refusal"),
        ):
            return
        if content_type == "output_text":
            if not self.object_shape(
                value,
                location,
                ("type", "text", "annotations"),
                ("logprobs",),
            ):
                return
            if "text" in value:
                self.string(value["text"], _path(location, "text"))
            if "annotations" in value:
                annotations = value["annotations"]
                annotations_path = _path(location, "annotations")
                if self.typed(
                    annotations,
                    annotations_path,
                    "array",
                    lambda item: isinstance(item, list),
                ):
                    for index, annotation in enumerate(annotations):
                        self.validate_response_annotation(
                            annotation, f"{annotations_path}/{index}"
                        )
            if "logprobs" in value:
                logprobs = value["logprobs"]
                logprobs_path = _path(location, "logprobs")
                if self.typed(
                    logprobs,
                    logprobs_path,
                    "array",
                    lambda item: isinstance(item, list),
                ):
                    for index, logprob in enumerate(logprobs):
                        self.validate_response_logprob(
                            logprob, f"{logprobs_path}/{index}"
                        )
        else:
            if not self.object_shape(value, location, ("type", "refusal")):
                return
            if "refusal" in value:
                self.string(value["refusal"], _path(location, "refusal"))

    def validate_response_tool(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        if not self.enum(value["type"], _path(location, "type"), ("function",)):
            return
        if not self.object_shape(
            value,
            location,
            ("type", "name", "strict", "parameters"),
            (
                "description",
                "output_schema",
                "defer_loading",
                "allowed_callers",
            ),
        ):
            return
        if "name" in value:
            self.string(value["name"], _path(location, "name"))
        if "strict" in value:
            self.typed(
                value["strict"],
                _path(location, "strict"),
                "boolean or null",
                lambda item: item is None or isinstance(item, bool),
            )
        if "parameters" in value:
            self.typed(
                value["parameters"],
                _path(location, "parameters"),
                "object or null",
                lambda item: item is None or isinstance(item, dict),
            )
        if "description" in value:
            self.nullable_string(value["description"], _path(location, "description"))
        if "output_schema" in value:
            self.typed(
                value["output_schema"],
                _path(location, "output_schema"),
                "object or null",
                lambda item: item is None or isinstance(item, dict),
            )
        if "defer_loading" in value:
            self.typed(
                value["defer_loading"],
                _path(location, "defer_loading"),
                "boolean",
                lambda item: isinstance(item, bool),
            )
        if "allowed_callers" in value:
            callers = value["allowed_callers"]
            callers_path = _path(location, "allowed_callers")
            if callers is None:
                return
            if self.typed(
                callers,
                callers_path,
                "array or null",
                lambda item: isinstance(item, list),
            ):
                for index, caller in enumerate(callers):
                    self.enum(
                        caller,
                        f"{callers_path}/{index}",
                        ("direct", "programmatic"),
                    )

    def validate_response_annotation(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        annotation_type = value["type"]
        if not self.enum(
            annotation_type,
            _path(location, "type"),
            (
                "file_citation",
                "url_citation",
                "container_file_citation",
                "file_path",
            ),
        ):
            return
        required_by_type = {
            "file_citation": ("type", "file_id", "index", "filename"),
            "url_citation": (
                "type",
                "url",
                "start_index",
                "end_index",
                "title",
            ),
            "container_file_citation": (
                "type",
                "container_id",
                "file_id",
                "start_index",
                "end_index",
                "filename",
            ),
            "file_path": ("type", "file_id", "index"),
        }
        if not self.object_shape(value, location, required_by_type[annotation_type]):
            return
        for field in (
            "container_id",
            "file_id",
            "filename",
            "title",
            "url",
        ):
            if field in value:
                self.string(value[field], _path(location, field))
        for field in ("index", "start_index", "end_index"):
            if field in value:
                self.integer(value[field], _path(location, field))

    def validate_response_logprob(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("token", "logprob"),
            ("top_logprobs",),
        ):
            return
        assert isinstance(value, dict)
        if "token" in value:
            self.string(value["token"], _path(location, "token"))
        if "logprob" in value:
            self.typed(
                value["logprob"],
                _path(location, "logprob"),
                "number",
                _is_number,
            )
        if "top_logprobs" not in value:
            return
        top_logprobs = value["top_logprobs"]
        top_logprobs_path = _path(location, "top_logprobs")
        if self.typed(
            top_logprobs,
            top_logprobs_path,
            "array",
            lambda item: isinstance(item, list),
        ):
            for index, top_logprob in enumerate(top_logprobs):
                item_path = f"{top_logprobs_path}/{index}"
                if not self.object_shape(
                    top_logprob,
                    item_path,
                    (),
                    ("token", "logprob"),
                ):
                    continue
                assert isinstance(top_logprob, dict)
                if "token" in top_logprob:
                    self.string(top_logprob["token"], _path(item_path, "token"))
                if "logprob" in top_logprob:
                    self.typed(
                        top_logprob["logprob"],
                        _path(item_path, "logprob"),
                        "number",
                        _is_number,
                    )

    def validate_byte_array(self, value: object, location: str) -> None:
        if self.typed(value, location, "array", lambda item: isinstance(item, list)):
            for index, byte in enumerate(value):
                self.integer(byte, f"{location}/{index}")

    def validate_reasoning_item(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("id", "type", "summary"),
            ("content", "encrypted_content", "status"),
        ):
            return
        assert isinstance(value, dict)
        if "id" in value:
            self.string(value["id"], _path(location, "id"))
        if "type" in value:
            self.enum(value["type"], _path(location, "type"), ("reasoning",))
        if "summary" in value:
            self.validate_reasoning_parts(
                value["summary"], _path(location, "summary"), "summary_text"
            )
        if "content" in value:
            self.validate_reasoning_parts(
                value["content"], _path(location, "content"), "reasoning_text"
            )
        if "encrypted_content" in value:
            self.nullable_string(
                value["encrypted_content"], _path(location, "encrypted_content")
            )
        if "status" in value:
            self.enum(value["status"], _path(location, "status"), self._ITEM_STATUSES)

    def validate_reasoning_parts(
        self,
        value: object,
        location: str,
        part_type: str,
    ) -> None:
        if not self.typed(value, location, "array", lambda item: isinstance(item, list)):
            return
        assert isinstance(value, list)
        for index, item in enumerate(value):
            item_path = f"{location}/{index}"
            if not self.object_shape(item, item_path, ("type", "text")):
                continue
            assert isinstance(item, dict)
            if "type" in item:
                self.enum(item["type"], _path(item_path, "type"), (part_type,))
            if "text" in item:
                self.string(item["text"], _path(item_path, "text"))

    def validate_usage(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            (
                "input_tokens",
                "input_tokens_details",
                "output_tokens",
                "output_tokens_details",
                "total_tokens",
            ),
        ):
            return
        assert isinstance(value, dict)
        for field in ("input_tokens", "output_tokens", "total_tokens"):
            if field in value:
                self.validate_nonnegative_integer(value[field], _path(location, field))
        if "input_tokens_details" in value:
            self.validate_token_details(
                value["input_tokens_details"],
                _path(location, "input_tokens_details"),
                ("cached_tokens",),
                ("audio_tokens",),
            )
        if "output_tokens_details" in value:
            self.validate_token_details(
                value["output_tokens_details"],
                _path(location, "output_tokens_details"),
                ("reasoning_tokens",),
                ("accepted_prediction_tokens", "rejected_prediction_tokens"),
            )

    def validate_token_details(
        self,
        value: object,
        location: str,
        required: Sequence[str],
        optional: Sequence[str],
    ) -> None:
        if not self.object_shape(value, location, required, optional):
            return
        assert isinstance(value, dict)
        for field in required + optional:
            if field in value:
                self.validate_nonnegative_integer(value[field], _path(location, field))

    def validate_nonnegative_integer(self, value: object, location: str) -> None:
        self.typed(
            value,
            location,
            "non-negative integer",
            lambda item: _is_integer(item) and item >= 0,
        )

    def validate_tool_choice(self, value: object, location: str) -> None:
        if isinstance(value, str):
            self.enum(value, location, ("none", "auto", "required"))
            return
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "documented string or object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        choice_type = value["type"]
        allowed = (
            "function",
            "file_search",
            "web_search_preview",
            "computer_use_preview",
            "code_interpreter",
            "image_generation",
            "mcp",
            "custom",
        )
        if not self.enum(choice_type, _path(location, "type"), allowed):
            return
        if choice_type in ("function", "custom"):
            required = ("type", "name")
        elif choice_type == "mcp":
            required = ("type", "server_label")
        else:
            required = ("type",)
        if not self.object_shape(value, location, required):
            return
        for field in required:
            if field != "type" and field in value:
                self.string(value[field], _path(location, field))

    def validate_reasoning_config(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, (), ("effort", "summary")):
            return
        assert isinstance(value, dict)
        if "effort" in value:
            self.typed(
                value["effort"],
                _path(location, "effort"),
                "string or null",
                lambda item: item is None or isinstance(item, str),
            )
        if "summary" in value:
            self.typed(
                value["summary"],
                _path(location, "summary"),
                "string or null",
                lambda item: item is None or isinstance(item, str),
            )

    def validate_text_config(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("format",), ("verbosity",)):
            return
        assert isinstance(value, dict)
        if "format" in value:
            self.validate_text_format(value["format"], _path(location, "format"))
        if "verbosity" in value:
            self.enum(
                value["verbosity"],
                _path(location, "verbosity"),
                ("low", "medium", "high"),
            )

    def validate_text_format(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        format_type = value["type"]
        if not self.enum(format_type, _path(location, "type"), ("text", "json_object", "json_schema")):
            return
        if format_type == "json_schema":
            if not self.object_shape(
                value,
                location,
                ("type", "name", "schema"),
                ("description", "strict"),
            ):
                return
            if "name" in value:
                self.string(value["name"], _path(location, "name"))
            if "schema" in value:
                self.typed(
                    value["schema"],
                    _path(location, "schema"),
                    "object",
                    lambda item: isinstance(item, dict),
                )
            if "description" in value:
                self.string(value["description"], _path(location, "description"))
            if "strict" in value:
                self.typed(
                    value["strict"],
                    _path(location, "strict"),
                    "boolean or null",
                    lambda item: item is None or isinstance(item, bool),
                )
        else:
            self.object_shape(value, location, ("type",))


class _AnthropicValidator(_ChatValidator):
    """Validate non-stream Anthropic Messages against the pinned profile."""

    _STOP_REASONS = (
        "end_turn",
        "max_tokens",
        "stop_sequence",
        "tool_use",
        "pause_turn",
        "refusal",
        "model_context_window_exceeded",
    )

    def validate_success(self, value: object) -> List[dict]:
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
        optional = ("container", "context_management")
        if not self.object_shape(value, "", required, optional):
            return self.differences
        assert isinstance(value, dict)
        if "id" in value:
            self.string(value["id"], "/id")
        if "type" in value:
            self.enum(value["type"], "/type", ("message",))
        if "role" in value:
            self.enum(value["role"], "/role", ("assistant",))
        if "model" in value:
            self.string(value["model"], "/model")

        has_tool_use = False
        if "content" in value:
            content = value["content"]
            if self.typed(content, "/content", "array", lambda item: isinstance(item, list)):
                for index, block in enumerate(content):
                    if isinstance(block, dict) and block.get("type") == "tool_use":
                        has_tool_use = True
                    self.validate_content_block(block, f"/content/{index}")
        valid_stop_reason = False
        if "stop_reason" in value:
            valid_stop_reason = self.enum(
                value["stop_reason"], "/stop_reason", self._STOP_REASONS
            )
        if has_tool_use and valid_stop_reason and value.get("stop_reason") != "tool_use":
            self.add(
                "/stop_reason",
                "CORRELATION",
                "tool_use when content contains a tool_use block",
                str(value.get("stop_reason")),
            )
        if "stop_sequence" in value:
            self.nullable_string(value["stop_sequence"], "/stop_sequence")
        if "usage" in value:
            self.validate_usage(value["usage"], "/usage")
        if "container" in value:
            self.validate_container(value["container"], "/container")
        if "context_management" in value:
            self.typed(
                value["context_management"],
                "/context_management",
                "object or null",
                lambda item: item is None or isinstance(item, dict),
            )
        return self.differences

    def validate_content_block(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        block_type = value["type"]
        if not self.enum(
            block_type,
            _path(location, "type"),
            ("text", "thinking", "redacted_thinking", "tool_use"),
        ):
            return
        required_by_type = {
            "text": ("type", "text"),
            "thinking": ("type", "thinking", "signature"),
            "redacted_thinking": ("type", "data"),
            "tool_use": ("type", "id", "name", "input"),
        }
        optional_by_type = {"text": ("citations",)}
        if not self.object_shape(
            value,
            location,
            required_by_type[block_type],
            optional_by_type.get(block_type, ()),
        ):
            return
        if block_type == "text":
            if "text" in value:
                self.string(value["text"], _path(location, "text"))
            if "citations" in value:
                citations = value["citations"]
                citations_path = _path(location, "citations")
                if citations is not None and self.typed(
                    citations,
                    citations_path,
                    "array or null",
                    lambda item: isinstance(item, list),
                ):
                    for index, citation in enumerate(citations):
                        self.validate_text_citation(
                            citation, f"{citations_path}/{index}"
                        )
        elif block_type == "thinking":
            if "thinking" in value:
                self.string(value["thinking"], _path(location, "thinking"))
            if "signature" in value:
                self.string(value["signature"], _path(location, "signature"))
        elif block_type == "redacted_thinking":
            if "data" in value:
                self.string(value["data"], _path(location, "data"))
        else:
            if "id" in value:
                self.string(value["id"], _path(location, "id"))
            if "name" in value:
                self.string(value["name"], _path(location, "name"))
            if "input" in value:
                self.typed(
                    value["input"],
                    _path(location, "input"),
                    "object",
                    lambda item: isinstance(item, dict),
                )

    def validate_text_citation(self, value: object, location: str) -> None:
        if not isinstance(value, dict):
            self.add(location, "TYPE_MISMATCH", "object", _actual_type(value))
            return
        if "type" not in value:
            self.add(_path(location, "type"), "MISSING_FIELD", "present", "missing")
            return
        citation_type = value["type"]
        if not self.enum(
            citation_type,
            _path(location, "type"),
            (
                "char_location",
                "page_location",
                "content_block_location",
                "web_search_result_location",
                "search_result_location",
            ),
        ):
            return
        required_by_type = {
            "char_location": (
                "type",
                "cited_text",
                "document_index",
                "start_char_index",
                "end_char_index",
            ),
            "page_location": (
                "type",
                "cited_text",
                "document_index",
                "start_page_number",
                "end_page_number",
            ),
            "content_block_location": (
                "type",
                "cited_text",
                "document_index",
                "start_block_index",
                "end_block_index",
            ),
            "web_search_result_location": (
                "type",
                "cited_text",
                "encrypted_index",
                "url",
            ),
            "search_result_location": (
                "type",
                "cited_text",
                "start_block_index",
                "end_block_index",
                "search_result_index",
                "source",
            ),
        }
        optional_by_type = {
            "char_location": ("document_title", "file_id"),
            "page_location": ("document_title", "file_id"),
            "content_block_location": ("document_title", "file_id"),
            "web_search_result_location": ("title",),
            "search_result_location": ("title",),
        }
        if not self.object_shape(
            value,
            location,
            required_by_type[citation_type],
            optional_by_type[citation_type],
        ):
            return
        for field in (
            "cited_text",
            "encrypted_index",
            "source",
            "url",
        ):
            if field in value:
                self.string(value[field], _path(location, field))
        for field in ("document_title", "file_id", "title"):
            if field in value:
                self.nullable_string(value[field], _path(location, field))
        for field in (
            "document_index",
            "start_char_index",
            "end_char_index",
            "start_page_number",
            "end_page_number",
            "start_block_index",
            "end_block_index",
            "search_result_index",
        ):
            if field in value:
                self.integer(value[field], _path(location, field))

    def validate_usage(self, value: object, location: str) -> None:
        optional = (
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "cache_creation",
            "server_tool_use",
            "service_tier",
            "inference_geo",
        )
        if not self.object_shape(
            value,
            location,
            ("input_tokens", "output_tokens"),
            optional,
        ):
            return
        assert isinstance(value, dict)
        for field in (
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ):
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "non-negative integer",
                    lambda item: _is_integer(item) and item >= 0,
                )
        if "cache_creation" in value:
            self.validate_cache_creation(value["cache_creation"], _path(location, "cache_creation"))
        if "server_tool_use" in value:
            self.validate_server_tool_use(value["server_tool_use"], _path(location, "server_tool_use"))
        if "service_tier" in value:
            self.enum(
                value["service_tier"],
                _path(location, "service_tier"),
                ("standard", "priority", "batch"),
            )
        if "inference_geo" in value:
            self.string(value["inference_geo"], _path(location, "inference_geo"))

    def validate_cache_creation(self, value: object, location: str) -> None:
        fields = ("ephemeral_1h_input_tokens", "ephemeral_5m_input_tokens")
        if not self.object_shape(value, location, fields):
            return
        assert isinstance(value, dict)
        for field in fields:
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "non-negative integer",
                    lambda item: _is_integer(item) and item >= 0,
                )

    def validate_server_tool_use(self, value: object, location: str) -> None:
        fields = (
            "web_search_requests",
            "web_fetch_requests",
            "code_execution_requests",
        )
        if not self.object_shape(value, location, (), fields):
            return
        assert isinstance(value, dict)
        for field in fields:
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "non-negative integer",
                    lambda item: _is_integer(item) and item >= 0,
                )

    def validate_container(self, value: object, location: str) -> None:
        if value is None:
            return
        if not self.object_shape(value, location, ("id", "expires_at")):
            return
        assert isinstance(value, dict)
        if "id" in value:
            self.string(value["id"], _path(location, "id"))
        if "expires_at" in value:
            self.string(value["expires_at"], _path(location, "expires_at"))

    def validate_error(self, value: object) -> List[dict]:
        if not self.object_shape(value, "", ("type", "error"), ("request_id",)):
            return self.differences
        assert isinstance(value, dict)
        if "type" in value:
            self.enum(value["type"], "/type", ("error",))
        if "request_id" in value:
            self.string(value["request_id"], "/request_id")
        if "error" not in value:
            return self.differences
        error = value["error"]
        if not self.object_shape(error, "/error", ("type", "message")):
            return self.differences
        assert isinstance(error, dict)
        if "type" in error:
            self.string(error["type"], "/error/type")
        if "message" in error:
            self.string(error["message"], "/error/message")
        return self.differences


class _GeminiValidator(_ChatValidator):
    """Validate frozen v1beta GenerateContent non-stream responses."""

    _FINISH_REASONS = (
        "FINISH_REASON_UNSPECIFIED",
        "STOP",
        "MAX_TOKENS",
        "SAFETY",
        "RECITATION",
        "LANGUAGE",
        "OTHER",
        "BLOCKLIST",
        "PROHIBITED_CONTENT",
        "SPII",
        "MALFORMED_FUNCTION_CALL",
        "IMAGE_SAFETY",
        "IMAGE_PROHIBITED_CONTENT",
        "IMAGE_OTHER",
        "NO_IMAGE",
        "IMAGE_RECITATION",
        "UNEXPECTED_TOOL_CALL",
        "TOO_MANY_TOOL_CALLS",
        "MISSING_THOUGHT_SIGNATURE",
        "MALFORMED_RESPONSE",
        "ESCALATION",
    )
    _BLOCK_REASONS = (
        "BLOCK_REASON_UNSPECIFIED",
        "SAFETY",
        "OTHER",
        "BLOCKLIST",
        "PROHIBITED_CONTENT",
        "IMAGE_SAFETY",
    )
    _MODEL_STAGES = (
        "MODEL_STAGE_UNSPECIFIED",
        "UNSTABLE_EXPERIMENTAL",
        "EXPERIMENTAL",
        "PREVIEW",
        "STABLE",
        "LEGACY",
        "DEPRECATED",
        "RETIRED",
    )
    _HARM_PROBABILITIES = (
        "HARM_PROBABILITY_UNSPECIFIED",
        "NEGLIGIBLE",
        "LOW",
        "MEDIUM",
        "HIGH",
    )
    _HARM_CATEGORIES = (
        "HARM_CATEGORY_UNSPECIFIED",
        "HARM_CATEGORY_DEROGATORY",
        "HARM_CATEGORY_TOXICITY",
        "HARM_CATEGORY_VIOLENCE",
        "HARM_CATEGORY_SEXUAL",
        "HARM_CATEGORY_MEDICAL",
        "HARM_CATEGORY_DANGEROUS",
        "HARM_CATEGORY_HARASSMENT",
        "HARM_CATEGORY_HATE_SPEECH",
        "HARM_CATEGORY_SEXUALLY_EXPLICIT",
        "HARM_CATEGORY_DANGEROUS_CONTENT",
        "HARM_CATEGORY_CIVIC_INTEGRITY",
        "HARM_CATEGORY_JAILBREAK",
    )
    _MODALITIES = (
        "MODALITY_UNSPECIFIED",
        "TEXT",
        "IMAGE",
        "VIDEO",
        "AUDIO",
        "DOCUMENT",
    )
    _SERVER_TOOL_TYPES = (
        "TOOL_TYPE_UNSPECIFIED",
        "GOOGLE_SEARCH_WEB",
        "GOOGLE_SEARCH_IMAGE",
        "URL_CONTEXT",
        "GOOGLE_MAPS",
        "FILE_SEARCH",
    )

    def __init__(
        self,
        request_id: str,
        protocol: str,
        request_body: object,
        http_status: int,
    ) -> None:
        super().__init__(request_id, protocol)
        self.http_status = http_status
        (
            self.functions_requiring_args,
            self.request_body_valid,
        ) = self._functions_requiring_args(request_body)

    @staticmethod
    def _functions_requiring_args(request_body: object) -> Tuple[set, bool]:
        decoded = request_body
        if isinstance(request_body, str):
            try:
                decoded = strict_json_loads(request_body)
            except JSON_LOAD_ERRORS:
                return set(), False
        if not isinstance(decoded, dict):
            return set(), False

        names = set()
        tools = decoded.get("tools")
        if not isinstance(tools, list):
            return names, True
        for tool in tools:
            if not isinstance(tool, dict):
                continue
            declarations = tool.get("functionDeclarations")
            if not isinstance(declarations, list):
                continue
            for declaration in declarations:
                if not isinstance(declaration, dict):
                    continue
                name = declaration.get("name")
                schemas = (
                    declaration.get("parameters"),
                    declaration.get("parametersJsonSchema"),
                )
                has_required_parameters = any(
                    isinstance(schema, dict)
                    and isinstance(schema.get("required"), list)
                    and bool(schema["required"])
                    for schema in schemas
                )
                if isinstance(name, str) and has_required_parameters:
                    names.add(name)
        return names, True

    def validate_success(self, value: object) -> List[dict]:
        fields = (
            "modelVersion",
            "responseId",
            "modelStatus",
            "candidates",
            "promptFeedback",
            "usageMetadata",
        )
        if not self.object_shape(value, "", (), fields):
            return self.differences
        assert isinstance(value, dict)

        if "candidates" not in value and "promptFeedback" not in value:
            self.add(
                "/candidates",
                "MISSING_FIELD",
                "candidates or promptFeedback present",
                "both missing",
            )
        for field in ("modelVersion", "responseId"):
            if field in value:
                self.string(value[field], _path("", field))
        if "modelStatus" in value:
            self.validate_model_status(value["modelStatus"], "/modelStatus")
        if "candidates" in value:
            candidates = value["candidates"]
            if self.typed(
                candidates,
                "/candidates",
                "array",
                lambda item: isinstance(item, list),
            ):
                for index, candidate in enumerate(candidates):
                    self.validate_candidate(candidate, f"/candidates/{index}")
        if "promptFeedback" in value:
            self.validate_prompt_feedback(value["promptFeedback"], "/promptFeedback")
        if "usageMetadata" in value:
            self.validate_usage(value["usageMetadata"], "/usageMetadata")
        return self.differences

    def validate_candidate(self, value: object, location: str) -> None:
        fields = (
            "finishMessage",
            "citationMetadata",
            "groundingMetadata",
            "avgLogprobs",
            "content",
            "index",
            "safetyRatings",
            "groundingAttributions",
            "urlContextMetadata",
            "tokenCount",
            "finishReason",
            "logprobsResult",
        )
        if not self.object_shape(value, location, (), fields):
            return
        assert isinstance(value, dict)

        if "finishMessage" in value:
            self.string(value["finishMessage"], _path(location, "finishMessage"))
        if "avgLogprobs" in value:
            self.typed(
                value["avgLogprobs"],
                _path(location, "avgLogprobs"),
                "number",
                _is_number,
            )
        if "content" in value:
            self.validate_content(value["content"], _path(location, "content"))
        for field in ("index", "tokenCount"):
            if field in value:
                self.integer(value[field], _path(location, field))
        if "finishReason" in value:
            self.enum(
                value["finishReason"],
                _path(location, "finishReason"),
                self._FINISH_REASONS,
            )
        if "safetyRatings" in value:
            self.validate_safety_ratings(
                value["safetyRatings"], _path(location, "safetyRatings")
            )
        if "citationMetadata" in value:
            self.validate_citation_metadata(
                value["citationMetadata"], _path(location, "citationMetadata")
            )
        for field in ("groundingAttributions",):
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "array",
                    lambda item: isinstance(item, list),
                )
        for field in (
            "groundingMetadata",
            "urlContextMetadata",
            "logprobsResult",
        ):
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "object",
                    lambda item: isinstance(item, dict),
                )

    def validate_content(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("parts",), ("role",)):
            return
        assert isinstance(value, dict)
        if "role" in value:
            self.enum(value["role"], _path(location, "role"), ("model",))
        if "parts" not in value:
            return
        parts = value["parts"]
        parts_path = _path(location, "parts")
        if self.typed(parts, parts_path, "array", lambda item: isinstance(item, list)):
            for index, part in enumerate(parts):
                self.validate_part(part, f"{parts_path}/{index}")

    def validate_part(self, value: object, location: str) -> None:
        fields = (
            "thought",
            "text",
            "mediaResolution",
            "executableCode",
            "functionResponse",
            "toolResponse",
            "functionCall",
            "videoMetadata",
            "inlineData",
            "toolCall",
            "fileData",
            "codeExecutionResult",
            "thoughtSignature",
            "mediaProcessing",
            "partMetadata",
        )
        if not self.object_shape(value, location, (), fields):
            return
        assert isinstance(value, dict)

        if "thought" in value:
            self.typed(
                value["thought"],
                _path(location, "thought"),
                "boolean",
                lambda item: isinstance(item, bool),
            )
        if "text" in value:
            self.string(value["text"], _path(location, "text"))
        if "functionCall" in value:
            self.validate_function_call(
                value["functionCall"], _path(location, "functionCall")
            )
        if "toolCall" in value:
            self.validate_server_tool(
                value["toolCall"],
                _path(location, "toolCall"),
                response=False,
            )
        if "toolResponse" in value:
            self.validate_server_tool(
                value["toolResponse"],
                _path(location, "toolResponse"),
                response=True,
            )
        if "executableCode" in value:
            self.validate_executable_code(
                value["executableCode"], _path(location, "executableCode")
            )
        if "thoughtSignature" in value:
            self.string(
                value["thoughtSignature"], _path(location, "thoughtSignature")
            )
        if "mediaProcessing" in value:
            self.enum(
                value["mediaProcessing"],
                _path(location, "mediaProcessing"),
                ("MEDIA_PROCESSING_UNSPECIFIED", "STATIC", "AGENTIC"),
            )
        for field in (
            "mediaResolution",
            "functionResponse",
            "videoMetadata",
            "inlineData",
            "fileData",
            "codeExecutionResult",
            "partMetadata",
        ):
            if field in value:
                self.typed(
                    value[field],
                    _path(location, field),
                    "object",
                    lambda item: isinstance(item, dict),
                )

    def validate_server_tool(
        self,
        value: object,
        location: str,
        *,
        response: bool,
    ) -> None:
        optional = ("id", "response") if response else ("id", "toolName", "args")
        if not self.object_shape(value, location, ("toolType",), optional):
            return
        assert isinstance(value, dict)
        if "toolType" in value:
            self.enum(
                value["toolType"],
                _path(location, "toolType"),
                self._SERVER_TOOL_TYPES,
            )
        string_fields = ("id",) if response else ("id", "toolName")
        for field in string_fields:
            if field in value:
                self.string(value[field], _path(location, field))
        object_field = "response" if response else "args"
        if object_field in value:
            self.typed(
                value[object_field],
                _path(location, object_field),
                "object",
                lambda item: isinstance(item, dict),
            )

    def validate_citation_metadata(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, (), ("citationSources",)):
            return
        assert isinstance(value, dict)
        if "citationSources" not in value:
            return
        sources = value["citationSources"]
        sources_path = _path(location, "citationSources")
        if not self.typed(
            sources,
            sources_path,
            "array",
            lambda item: isinstance(item, list),
        ):
            return
        assert isinstance(sources, list)
        for index, source in enumerate(sources):
            source_path = f"{sources_path}/{index}"
            if not self.object_shape(
                source,
                source_path,
                (),
                ("startIndex", "endIndex", "uri", "license"),
            ):
                continue
            assert isinstance(source, dict)
            for field in ("startIndex", "endIndex"):
                if field in source:
                    self.integer(source[field], _path(source_path, field))
            for field in ("uri", "license"):
                if field in source:
                    self.string(source[field], _path(source_path, field))

    def validate_executable_code(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("language", "code"),
            ("id",),
        ):
            return
        assert isinstance(value, dict)
        if "language" in value:
            self.enum(
                value["language"],
                _path(location, "language"),
                ("LANGUAGE_UNSPECIFIED", "PYTHON"),
            )
        if "code" in value:
            self.string(value["code"], _path(location, "code"))
        if "id" in value:
            self.string(value["id"], _path(location, "id"))

    def validate_function_call(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("name",), ("args", "id")):
            return
        assert isinstance(value, dict)

        name = value.get("name")
        if "name" in value:
            self.string(name, _path(location, "name"))
        if isinstance(name, str) and name in self.functions_requiring_args:
            if "args" not in value:
                self.add(
                    _path(location, "args"),
                    "MISSING_FIELD",
                    "object because the request declaration includes parameters",
                    "missing",
                )
        elif "args" not in value and not self.request_body_valid:
            self.add(
                "/requestBody",
                "EVIDENCE_GAP",
                "valid request JSON object for conditional args validation",
                "required evidence unavailable or invalid",
            )
        if "args" in value:
            self.typed(
                value["args"],
                _path(location, "args"),
                "object",
                lambda item: isinstance(item, dict),
            )
        if "id" in value:
            self.string(value["id"], _path(location, "id"))

    def validate_prompt_feedback(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            (),
            ("blockReason", "safetyRatings"),
        ):
            return
        assert isinstance(value, dict)
        if "blockReason" in value:
            self.enum(
                value["blockReason"],
                _path(location, "blockReason"),
                self._BLOCK_REASONS,
            )
        if "safetyRatings" in value:
            self.validate_safety_ratings(
                value["safetyRatings"], _path(location, "safetyRatings")
            )

    def validate_safety_ratings(self, value: object, location: str) -> None:
        if not self.typed(value, location, "array", lambda item: isinstance(item, list)):
            return
        assert isinstance(value, list)
        for index, rating in enumerate(value):
            rating_path = f"{location}/{index}"
            if not self.object_shape(
                rating,
                rating_path,
                ("probability", "category"),
                ("blocked",),
            ):
                continue
            assert isinstance(rating, dict)
            if "probability" in rating:
                self.enum(
                    rating["probability"],
                    _path(rating_path, "probability"),
                    self._HARM_PROBABILITIES,
                )
            if "category" in rating:
                self.enum(
                    rating["category"],
                    _path(rating_path, "category"),
                    self._HARM_CATEGORIES,
                )
            if "blocked" in rating:
                self.typed(
                    rating["blocked"],
                    _path(rating_path, "blocked"),
                    "boolean",
                    lambda item: isinstance(item, bool),
                )

    def validate_usage(self, value: object, location: str) -> None:
        integer_fields = (
            "candidatesTokenCount",
            "totalTokenCount",
            "promptTokenCount",
            "toolUsePromptTokenCount",
            "thoughtsTokenCount",
            "cachedContentTokenCount",
        )
        detail_fields = (
            "cacheTokensDetails",
            "toolUsePromptTokensDetails",
            "candidatesTokensDetails",
            "promptTokensDetails",
        )
        if not self.object_shape(
            value,
            location,
            (),
            integer_fields + detail_fields + ("serviceTier",),
        ):
            return
        assert isinstance(value, dict)

        for field in integer_fields:
            if field in value:
                self.integer(value[field], _path(location, field))
        if "serviceTier" in value:
            self.enum(
                value["serviceTier"],
                _path(location, "serviceTier"),
                ("unspecified", "standard", "flex", "priority"),
            )
        for field in detail_fields:
            if field in value:
                self.validate_modality_counts(value[field], _path(location, field))

    def validate_modality_counts(self, value: object, location: str) -> None:
        if not self.typed(value, location, "array", lambda item: isinstance(item, list)):
            return
        assert isinstance(value, list)
        for index, detail in enumerate(value):
            detail_path = f"{location}/{index}"
            if not self.object_shape(
                detail,
                detail_path,
                (),
                ("modality", "tokenCount"),
            ):
                continue
            assert isinstance(detail, dict)
            if "modality" in detail:
                self.enum(
                    detail["modality"],
                    _path(detail_path, "modality"),
                    self._MODALITIES,
                )
            if "tokenCount" in detail:
                self.integer(detail["tokenCount"], _path(detail_path, "tokenCount"))

    def validate_model_status(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            (),
            ("modelStage", "message", "retirementTime"),
        ):
            return
        assert isinstance(value, dict)
        if "modelStage" in value:
            self.enum(
                value["modelStage"],
                _path(location, "modelStage"),
                self._MODEL_STAGES,
            )
        for field in ("message", "retirementTime"):
            if field in value:
                self.string(value[field], _path(location, field))

    def validate_error(self, value: object) -> List[dict]:
        if not self.object_shape(value, "", ("error",)):
            return self.differences
        assert isinstance(value, dict)
        if "error" not in value:
            return self.differences
        error = value["error"]
        if not self.object_shape(
            error,
            "/error",
            ("code", "message"),
            ("status", "details"),
        ):
            return self.differences
        assert isinstance(error, dict)

        if "code" in error and self.integer(error["code"], "/error/code"):
            if error["code"] != self.http_status:
                self.add(
                    "/error/code",
                    "VALUE_MISMATCH",
                    f"HTTP status {self.http_status}",
                    str(error["code"]),
                )
        if "message" in error:
            self.string(error["message"], "/error/message")
        if "status" in error:
            self.string(error["status"], "/error/status")
        if "details" in error:
            self.typed(
                error["details"],
                "/error/details",
                "array",
                lambda item: isinstance(item, list),
            )
        return self.differences


def _parse_integer(value: object) -> Optional[int]:
    if _is_integer(value):
        return value
    if isinstance(value, str):
        stripped = value.strip()
        if stripped and (stripped.isdigit() or (stripped[0] in "+-" and stripped[1:].isdigit())):
            try:
                return int(stripped)
            except ValueError:
                return None
    return None


class _OllamaValidator(_ChatValidator):
    """Validate frozen Ollama /api/chat non-stream response objects."""

    _DURATION_AND_COUNT_FIELDS = (
        "total_duration",
        "load_duration",
        "prompt_eval_count",
        "prompt_eval_duration",
        "eval_count",
        "eval_duration",
    )

    def __init__(self, request_id: str, protocol: str) -> None:
        super().__init__(request_id, protocol)
        self._index_mode: Optional[bool] = None
        self._seen_indexes: set[int] = set()
        self._seen_call_ids: set[str] = set()

    def validate_success(self, value: object) -> List[dict]:
        if not self.object_shape(
            value,
            "",
            ("model", "created_at", "message", "done"),
            (
                "done_reason",
                *self._DURATION_AND_COUNT_FIELDS,
                "logprobs",
            ),
        ):
            return self.differences
        assert isinstance(value, dict)

        for field in ("model", "created_at"):
            if field in value:
                self.string(value[field], _path("", field))
        if "message" in value:
            self.validate_message(value["message"], "/message")
        if "done" in value:
            done = value["done"]
            if self.typed(
                done,
                "/done",
                "boolean",
                lambda item: isinstance(item, bool),
            ) and done is not True:
                self.add("/done", "VALUE_MISMATCH", "true", "false")
        if "done_reason" in value:
            self.string(value["done_reason"], "/done_reason")
        for field in self._DURATION_AND_COUNT_FIELDS:
            if field in value:
                self.integer(value[field], _path("", field))
        if "logprobs" in value:
            logprobs = value["logprobs"]
            if self.typed(
                logprobs,
                "/logprobs",
                "array",
                lambda item: isinstance(item, list),
            ):
                for index, logprob in enumerate(logprobs):
                    self.validate_logprob(logprob, f"/logprobs/{index}")
        return self.differences

    def validate_message(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            ("role", "content"),
            ("thinking", "tool_calls", "images"),
        ):
            return
        assert isinstance(value, dict)

        if "role" in value:
            self.enum(value["role"], _path(location, "role"), ("assistant",))
        for field in ("content", "thinking"):
            if field in value:
                self.string(value[field], _path(location, field))
        if "tool_calls" in value:
            tool_calls = value["tool_calls"]
            tool_calls_path = _path(location, "tool_calls")
            if self.typed(
                tool_calls,
                tool_calls_path,
                "array",
                lambda item: isinstance(item, list),
            ):
                for index, tool_call in enumerate(tool_calls):
                    self.validate_tool_call(tool_call, f"{tool_calls_path}/{index}")
                self.validate_tool_call_indexes(tool_calls, tool_calls_path)
        if "images" in value:
            images = value["images"]
            images_path = _path(location, "images")
            if self.typed(
                images,
                images_path,
                "array",
                lambda item: isinstance(item, list),
            ):
                for index, image in enumerate(images):
                    self.string(image, f"{images_path}/{index}")

    def validate_tool_call(self, value: object, location: str) -> None:
        if not self.object_shape(value, location, ("function",), ("id",)):
            return
        assert isinstance(value, dict)
        if "id" in value:
            id_path = _path(location, "id")
            if self.string(value["id"], id_path) and not value["id"].strip():
                self.add(
                    id_path,
                    "VALUE_MISMATCH",
                    "non-empty string",
                    "empty string",
                )
        if "function" not in value:
            return

        function = value["function"]
        function_path = _path(location, "function")
        if not self.object_shape(
            function,
            function_path,
            ("name",),
            ("description", "arguments", "index"),
        ):
            return
        assert isinstance(function, dict)
        for field in ("name", "description"):
            if field in function:
                self.string(function[field], _path(function_path, field))
        if "arguments" in function:
            self.typed(
                function["arguments"],
                _path(function_path, "arguments"),
                "object",
                lambda item: isinstance(item, dict),
            )
        if "index" in function:
            index_path = _path(function_path, "index")
            if self.integer(function["index"], index_path) and function["index"] < 0:
                self.add(
                    index_path,
                    "VALUE_MISMATCH",
                    "non-negative integer",
                    str(function["index"]),
                )

    def validate_tool_call_indexes(self, value: list, location: str) -> None:
        for position, raw_call in enumerate(value):
            if not isinstance(raw_call, dict):
                continue
            function = raw_call.get("function")
            if not isinstance(function, dict):
                continue
            index_path = f"{location}/{position}/function/index"
            indexed = "index" in function
            if self._index_mode is None:
                self._index_mode = indexed
            elif indexed != self._index_mode:
                self.add(
                    index_path,
                    "MISSING_FIELD" if not indexed else "VALUE_MISMATCH",
                    "function.index on every call or on no calls in the turn",
                    "missing" if not indexed else "mixed index mode",
                )
            native_index = function.get("index")
            if (
                indexed
                and isinstance(native_index, int)
                and not isinstance(native_index, bool)
                and native_index >= 0
            ):
                if native_index in self._seen_indexes:
                    self.add(
                        index_path,
                        "VALUE_MISMATCH",
                        "unique function.index within the turn",
                        str(native_index),
                    )
                self._seen_indexes.add(native_index)
            call_id = raw_call.get("id")
            if isinstance(call_id, str) and call_id:
                if call_id in self._seen_call_ids:
                    self.add(
                        f"{location}/{position}/id",
                        "VALUE_MISMATCH",
                        "unique optional tool-call ID within the turn",
                        call_id,
                    )
                self._seen_call_ids.add(call_id)

    def validate_logprob(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            (),
            ("token", "logprob", "bytes", "top_logprobs"),
        ):
            return
        assert isinstance(value, dict)
        if "token" in value:
            self.string(value["token"], _path(location, "token"))
        if "logprob" in value:
            self.typed(
                value["logprob"],
                _path(location, "logprob"),
                "number",
                _is_number,
            )
        if "bytes" in value:
            self.validate_byte_array(value["bytes"], _path(location, "bytes"))
        if "top_logprobs" in value:
            top_logprobs = value["top_logprobs"]
            top_path = _path(location, "top_logprobs")
            if self.typed(
                top_logprobs,
                top_path,
                "array",
                lambda item: isinstance(item, list),
            ):
                for index, item in enumerate(top_logprobs):
                    self.validate_token_logprob(item, f"{top_path}/{index}")

    def validate_token_logprob(self, value: object, location: str) -> None:
        if not self.object_shape(
            value,
            location,
            (),
            ("token", "logprob", "bytes"),
        ):
            return
        assert isinstance(value, dict)
        if "token" in value:
            self.string(value["token"], _path(location, "token"))
        if "logprob" in value:
            self.typed(
                value["logprob"],
                _path(location, "logprob"),
                "number",
                _is_number,
            )
        if "bytes" in value:
            self.validate_byte_array(value["bytes"], _path(location, "bytes"))

    def validate_byte_array(self, value: object, location: str) -> None:
        if not self.typed(
            value,
            location,
            "array",
            lambda item: isinstance(item, list),
        ):
            return
        assert isinstance(value, list)
        for index, byte in enumerate(value):
            self.integer(byte, f"{location}/{index}")

    def validate_error(self, value: object) -> List[dict]:
        if not self.object_shape(value, "", ("error",)):
            return self.differences
        assert isinstance(value, dict)
        if "error" in value:
            self.string(value["error"], "/error")
        return self.differences


def _parse_stream(value: object) -> Optional[bool]:
    if isinstance(value, bool):
        return value
    if _is_integer(value) and value in (0, 1):
        return bool(value)
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in {"1", "true", "yes", "on"}:
            return True
        if normalized in {"0", "false", "no", "off"}:
            return False
    return None


def _content_type(headers: object) -> Optional[str]:
    if not isinstance(headers, str):
        return None
    found = None
    for line in headers.splitlines():
        name, separator, value = line.partition(":")
        if separator and name.strip().lower() == "content-type":
            found = value.strip()
    return found


def _check_ids(parsed: Mapping[str, object]) -> Dict[str, List[str]]:
    associations: Dict[str, set] = defaultdict(set)
    tests = parsed.get("tests", {})
    if not isinstance(tests, dict):
        return {}
    for check_id, test in tests.items():
        if not isinstance(test, dict):
            continue
        refs = test.get("requestRefs", [])
        if not isinstance(refs, list):
            continue
        for request_id in refs:
            if isinstance(request_id, str):
                associations[request_id].add(str(check_id))
    return {key: sorted(value) for key, value in associations.items()}


def _analyze_request(
    request_id: str,
    request: object,
    check_ids: Sequence[str],
) -> dict:
    raw_request = request if isinstance(request, dict) else {}
    raw_protocol = raw_request.get("protocol")
    protocol = (
        raw_protocol
        if isinstance(raw_protocol, str) and raw_protocol != ""
        else None
    )
    stream_value = _parse_stream(raw_request.get("stream"))
    stream = stream_value is True
    metrics = raw_request.get("metrics")
    metrics = metrics if isinstance(metrics, dict) else {}
    http_status = _parse_integer(metrics.get("http_status"))
    curl_exit_code = _parse_integer(metrics.get("curl_exit_code"))
    response_body = raw_request.get("responseBody")
    differences: List[dict] = []

    def evidence(location: str, expected: str, actual: str) -> None:
        differences.append(
            _difference(
                request_id,
                protocol,
                location,
                "EVIDENCE_GAP",
                expected,
                actual,
            )
        )

    if isinstance(request, dict) and request.get("request_id") != request_id:
        differences.append(
            _difference(
                request_id,
                protocol,
                "/request_id",
                "CORRELATION",
                "same identifier as the enclosing requests map key",
                "identifier does not match the enclosing requests map key",
            )
        )

    if not isinstance(request, dict):
        evidence("/request", "collected request object", _actual_type(request))
    elif stream_value is None:
        evidence(
            "/stream",
            "explicit boolean stream indicator",
            "missing" if "stream" not in raw_request else _actual_type(raw_request["stream"]),
        )
    elif curl_exit_code is None:
        evidence(
            "/metrics/curl_exit_code",
            "integer transport exit code",
            "invalid exit code",
        )
    elif curl_exit_code != 0:
        evidence(
            "/metrics/curl_exit_code",
            "zero transport exit code",
            "non-zero exit code",
        )
    elif "responseBody" not in raw_request:
        evidence("/responseBody", "recorded response bytes", "missing")
    elif not isinstance(response_body, str):
        evidence("/responseBody", "recorded response bytes", _actual_type(response_body))
    elif http_status is None or not 100 <= http_status <= 599:
        evidence(
            "/metrics/http_status",
            "HTTP status integer from 100 through 599",
            "invalid status",
        )
    elif protocol not in SUPPORTED_PROTOCOLS:
        evidence(
            "/protocol",
            "one pinned supported protocol",
            "missing" if protocol is None else "unknown protocol",
        )
    else:
        if protocol == "gemini_generate_content":
            validator = _GeminiValidator(
                request_id,
                protocol,
                raw_request.get("requestBody"),
                http_status,
            )
        else:
            validator_class = {
                "openai_chat": _ChatValidator,
                "openai_responses": _ResponsesValidator,
                "anthropic_messages": _AnthropicValidator,
                "ollama_chat": _OllamaValidator,
            }.get(protocol)
            validator = (
                validator_class(request_id, protocol)
                if validator_class is not None
                else None
            )

        if stream and validator is not None:
            from model_doctor_protocol_streaming import (
                StreamEvidence,
                validate_stream_response,
            )

            differences.extend(
                validate_stream_response(
                    StreamEvidence(
                        request_id=request_id,
                        protocol=protocol,
                        response_body=response_body,
                        response_headers=raw_request.get("responseHeaders"),
                        http_status=http_status,
                        request_body=raw_request.get("requestBody"),
                        profile_validator=validator,
                    ),
                    make_difference=_difference,
                    validate_http_error=validator.validate_error,
                )
            )
            return {
                "requestId": request_id,
                "protocol": protocol,
                "checkIds": list(check_ids),
                "stream": stream,
                "httpStatus": http_status,
                "status": "DIFFERENT" if differences else "CONSISTENT",
                "differences": differences,
            }

        content_type = _content_type(raw_request.get("responseHeaders"))
        if content_type is not None:
            media_type = content_type.split(";", 1)[0].strip().lower()
            if media_type != "application/json":
                differences.append(
                    _difference(
                        request_id,
                        protocol,
                        "/responseHeaders/content-type",
                        "VALUE_MISMATCH",
                        "application/json when content-type is present",
                        media_type or "empty media type",
                    )
                )
        try:
            decoded = strict_json_loads(response_body)
        except JSON_LOAD_ERRORS:
            differences.append(
                _difference(
                    request_id,
                    protocol,
                    "/responseBody",
                    "INVALID_JSON",
                    "one JSON response object",
                    "invalid JSON",
                )
            )
        else:
            if validator is not None:
                if 200 <= http_status <= 299:
                    differences.extend(validator.validate_success(decoded))
                else:
                    differences.extend(validator.validate_error(decoded))
            else:
                evidence(
                    "/responseBody",
                    "implemented pinned non-stream protocol profile",
                    "profile validation deferred",
                )

    return {
        "requestId": request_id,
        "protocol": protocol,
        "checkIds": list(check_ids),
        "stream": stream,
        "httpStatus": http_status,
        "status": "DIFFERENT" if differences else "CONSISTENT",
        "differences": differences,
    }


def _status_group(field: str, results: Sequence[dict]) -> List[dict]:
    grouped: Dict[str, List[dict]] = defaultdict(list)
    if field == "protocol":
        for result in results:
            protocol = result.get("protocol")
            key = protocol if isinstance(protocol, str) else "unknown"
            grouped[key].append(result)
    else:
        for result in results:
            for check_id in result.get("checkIds", []):
                grouped[check_id].append(result)

    key_name = "protocol" if field == "protocol" else "checkId"
    return [
        {
            key_name: key,
            "totalRequests": len(items),
            "consistentRequests": sum(
                item.get("status") == "CONSISTENT" for item in items
            ),
            "differentRequests": sum(
                item.get("status") == "DIFFERENT" for item in items
            ),
        }
        for key, items in sorted(grouped.items())
    ]


def _make_summary(results: Sequence[dict]) -> dict:
    difference_counts = Counter(
        difference.get("differenceKind")
        for result in results
        for difference in result.get("differences", [])
    )
    difference_counts.pop(None, None)
    return {
        "totalRequests": len(results),
        "checkedRequests": len(results),
        "consistentRequests": sum(
            result.get("status") == "CONSISTENT" for result in results
        ),
        "differentRequests": sum(
            result.get("status") == "DIFFERENT" for result in results
        ),
        "byProtocol": _status_group("protocol", results),
        "byCheck": _status_group("check", results),
        "byDifferenceKind": [
            {"differenceKind": kind, "count": count}
            for kind, count in sorted(difference_counts.items())
        ],
    }


def analyze_protocol_conformance(parsed: dict) -> dict:
    """Return pinned baselines, summaries, and one result per raw request."""

    parsed_mapping: Mapping[str, object] = parsed if isinstance(parsed, dict) else {}
    requests = parsed_mapping.get("requests", {})
    requests = requests if isinstance(requests, dict) else {}
    associations = _check_ids(parsed_mapping)
    results = [
        _analyze_request(str(request_id), request, associations.get(str(request_id), []))
        for request_id, request in requests.items()
    ]
    results_by_id = {result["requestId"]: result for result in results}
    for request_id, differences in validate_tool_loop_transitions(
        parsed_mapping
    ).items():
        result = results_by_id.get(request_id)
        if result is None:
            continue
        for difference in differences:
            if difference not in result["differences"]:
                result["differences"].append(difference)
        result["status"] = "DIFFERENT" if result["differences"] else "CONSISTENT"
    return {
        "ruleSetVersion": RULE_SET_VERSION,
        "baselineDate": BASELINE_DATE,
        "baselines": [
            {
                **baseline,
                "supportingReferences": list(baseline["supportingReferences"]),
            }
            for baseline in _BASELINES
        ],
        "summary": _make_summary(results),
        "results": results,
    }


def _shape_errors(value: object, fields: set, prefix: str) -> Tuple[List[str], bool]:
    if not isinstance(value, dict):
        return [f"{prefix} must be an object"], False
    errors = [
        f"{prefix} field {field} is not allowed"
        for field in sorted(set(value) - fields)
    ]
    errors.extend(
        f"{prefix} field {field} is required"
        for field in sorted(fields - set(value))
    )
    return errors, not errors


def _non_empty_string(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _validate_group_array(
    value: object,
    name: str,
    kind: str,
    errors: List[str],
) -> None:
    if not isinstance(value, list):
        errors.append(f"summary {name} must be an array")
        return
    fields = _GROUP_FIELDS[kind]
    for index, item in enumerate(value):
        item_errors, valid = _shape_errors(item, fields, f"summary {name}[{index}]")
        errors.extend(item_errors)
        if not valid:
            continue
        assert isinstance(item, dict)
        for field in fields & {"totalRequests", "consistentRequests", "differentRequests", "count"}:
            if not _is_integer(item.get(field)) or item[field] < 0:
                errors.append(f"summary {name}[{index}] {field} must be a non-negative integer")


def validate_protocol_conformance(value: object) -> List[str]:
    """Validate the report's closed shape and deterministic cross-field counts."""

    errors, valid_report = _shape_errors(value, _REPORT_FIELDS, "protocolConformance")
    if not valid_report:
        return errors
    assert isinstance(value, dict)

    if value.get("ruleSetVersion") != RULE_SET_VERSION:
        errors.append("ruleSetVersion does not match the pinned rule set")
    if value.get("baselineDate") != BASELINE_DATE:
        errors.append("baselineDate does not match the pinned baseline date")

    baselines = value.get("baselines")
    if not isinstance(baselines, list):
        errors.append("baselines must be an array")
    else:
        for index, baseline in enumerate(baselines):
            baseline_errors, _ = _shape_errors(
                baseline, _BASELINE_FIELDS, f"baselines[{index}]"
            )
            errors.extend(baseline_errors)
        if baselines != list(_BASELINES):
            errors.append("baselines do not match pinned protocol metadata")

    results = value.get("results")
    valid_results: List[dict] = []
    seen_request_ids = set()
    if not isinstance(results, list):
        errors.append("results must be an array")
    else:
        for index, result in enumerate(results):
            prefix = f"results[{index}]"
            result_errors, valid_shape = _shape_errors(result, _RESULT_FIELDS, prefix)
            errors.extend(result_errors)
            if not valid_shape:
                continue
            assert isinstance(result, dict)
            structurally_valid = True
            request_id = result.get("requestId")
            if not _non_empty_string(request_id):
                errors.append(f"{prefix} requestId must be a non-empty string")
                structurally_valid = False
            elif request_id in seen_request_ids:
                errors.append(f"{prefix} requestId is duplicated: {request_id}")
                structurally_valid = False
            else:
                seen_request_ids.add(request_id)
            protocol = result.get("protocol")
            if protocol is not None and (
                not isinstance(protocol, str) or protocol == ""
            ):
                errors.append(f"{prefix} protocol must be a non-empty string or null")
                structurally_valid = False
            check_ids = result.get("checkIds")
            if not (
                isinstance(check_ids, list)
                and all(_non_empty_string(item) for item in check_ids)
                and check_ids == sorted(set(check_ids))
            ):
                errors.append(f"{prefix} checkIds must be a sorted unique string array")
                structurally_valid = False
            if not isinstance(result.get("stream"), bool):
                errors.append(f"{prefix} stream must be boolean")
                structurally_valid = False
            http_status = result.get("httpStatus")
            if http_status is not None and not _is_integer(http_status):
                errors.append(f"{prefix} httpStatus must be an integer or null")
                structurally_valid = False
            status = result.get("status")
            if status not in {"CONSISTENT", "DIFFERENT"}:
                errors.append(f"{prefix} status is invalid")
                structurally_valid = False
            differences = result.get("differences")
            if not isinstance(differences, list):
                errors.append(f"{prefix} differences must be an array")
                structurally_valid = False
                differences = []
            elif status == "CONSISTENT" and differences:
                errors.append(f"{prefix} CONSISTENT requires empty differences")
            elif status == "DIFFERENT" and not differences:
                errors.append(f"{prefix} DIFFERENT requires non-empty differences")

            for difference_index, difference in enumerate(differences):
                difference_prefix = f"{prefix} differences[{difference_index}]"
                difference_errors, valid_difference = _shape_errors(
                    difference, _DIFFERENCE_FIELDS, difference_prefix
                )
                errors.extend(difference_errors)
                if not valid_difference:
                    structurally_valid = False
                    continue
                assert isinstance(difference, dict)
                if difference.get("requestId") != request_id:
                    errors.append(f"{difference_prefix} requestId does not own the difference")
                if difference.get("protocol") != protocol:
                    errors.append(f"{difference_prefix} protocol does not own the difference")
                if difference.get("differenceKind") not in DIFFERENCE_KINDS:
                    errors.append(f"{difference_prefix} differenceKind is invalid")
                difference_kind = difference.get("differenceKind")
                location = difference.get("location")
                if not _non_empty_string(location) or not location.startswith("/"):
                    errors.append(f"{difference_prefix} location must be an RFC 6901 path")
                for field in ("expected", "actual", "officialReference"):
                    if not _non_empty_string(difference.get(field)):
                        errors.append(f"{difference_prefix} {field} must be a non-empty string")
                actual = difference.get("actual")
                if (
                    isinstance(difference_kind, str)
                    and isinstance(actual, str)
                    and actual != _bounded_actual(difference_kind, actual)
                ):
                    errors.append(
                        f"{difference_prefix} actual must be a bounded category"
                    )
                if difference.get("officialReference") != _reference(protocol):
                    errors.append(
                        f"{difference_prefix} officialReference does not match "
                        "the pinned protocol source"
                    )
            if structurally_valid:
                valid_results.append(result)

    summary = value.get("summary")
    summary_errors, valid_summary = _shape_errors(summary, _SUMMARY_FIELDS, "summary")
    errors.extend(summary_errors)
    if valid_summary:
        assert isinstance(summary, dict)
        for field in (
            "totalRequests",
            "checkedRequests",
            "consistentRequests",
            "differentRequests",
        ):
            if not _is_integer(summary.get(field)) or summary[field] < 0:
                errors.append(f"summary {field} must be a non-negative integer")
        _validate_group_array(summary.get("byProtocol"), "byProtocol", "protocol", errors)
        _validate_group_array(summary.get("byCheck"), "byCheck", "check", errors)
        _validate_group_array(
            summary.get("byDifferenceKind"),
            "byDifferenceKind",
            "difference",
            errors,
        )

        if isinstance(results, list) and len(valid_results) == len(results):
            expected_summary = _make_summary(valid_results)
            for field in _SUMMARY_FIELDS:
                if summary.get(field) != expected_summary[field]:
                    errors.append(f"summary {field} does not match results")

    return errors
