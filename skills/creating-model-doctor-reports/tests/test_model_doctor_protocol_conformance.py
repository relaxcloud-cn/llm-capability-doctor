from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path


SKILL_DIR = Path(__file__).resolve().parents[1]
SCRIPT_DIR = SKILL_DIR / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

from model_doctor_protocol_conformance import (  # noqa: E402
    BASELINE_DATE,
    DIFFERENCE_KINDS,
    RULE_SET_VERSION,
    SUPPORTED_PROTOCOLS,
    analyze_protocol_conformance,
    validate_protocol_conformance,
)
from model_doctor_assessment import (  # noqa: E402
    CAPABILITY_SCOPE_BOUNDARY,
    validate_reviews,
)


class ProtocolConformanceTests(unittest.TestCase):
    def _request(
        self,
        request_id: str,
        protocol: str | None,
        response: object = None,
        *,
        raw_response: str | None = None,
        request_body: object | str | None = None,
        http_status: object = "200",
        curl_exit_code: object = "0",
        stream: object = "0",
        response_headers: str = "content-type: application/json; charset=utf-8",
    ) -> dict:
        request = {
            "request_id": request_id,
            "stream": stream,
            "requestBody": (
                request_body
                if isinstance(request_body, str)
                else json.dumps(request_body or {"model": "fixture"})
            ),
            "metrics": {
                "http_status": http_status,
                "curl_exit_code": curl_exit_code,
            },
            "responseHeaders": response_headers,
            "stderr": "",
            "responseBody": (
                raw_response
                if raw_response is not None
                else json.dumps(response)
            ),
        }
        if protocol is not None:
            request["protocol"] = protocol
        return request

    def _parsed(self, requests: dict, tests: dict | None = None) -> dict:
        return {
            "requests": requests,
            "tests": tests or {},
        }

    def _chat(self) -> dict:
        return {
            "id": "chatcmpl-fixture",
            "object": "chat.completion",
            "created": 1,
            "model": "gpt-fixture",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "ok",
                    },
                    "finish_reason": "stop",
                    "logprobs": None,
                }
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2,
            },
        }

    def _responses(self) -> dict:
        return {
            "id": "resp_fixture",
            "object": "response",
            "created_at": 1,
            "error": None,
            "incomplete_details": None,
            "instructions": None,
            "model": "gpt-fixture",
            "tools": [],
            "output": [
                {
                    "id": "msg_fixture",
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        {
                            "type": "output_text",
                            "text": "ok",
                            "annotations": [],
                        }
                    ],
                    "status": "completed",
                }
            ],
            "parallel_tool_calls": True,
            "metadata": {},
            "tool_choice": "auto",
            "temperature": 1.0,
            "top_p": 1.0,
            "status": "completed",
        }

    def _anthropic(self) -> dict:
        return {
            "id": "msg_fixture",
            "type": "message",
            "role": "assistant",
            "model": "claude-fixture",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "stop_sequence": None,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        }

    def _gemini(self) -> dict:
        return {
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [{"text": "ok"}],
                    },
                    "finishReason": "STOP",
                    "index": 0,
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 1,
                "candidatesTokenCount": 1,
                "totalTokenCount": 2,
            },
            "modelVersion": "gemini-fixture",
            "responseId": "resp-fixture",
        }

    def _ollama(self) -> dict:
        return {
            "model": "ollama-fixture",
            "created_at": "2026-08-18T00:00:00Z",
            "message": {"role": "assistant", "content": "ok"},
            "done": True,
            "done_reason": "stop",
            "total_duration": 10,
            "load_duration": 2,
            "prompt_eval_count": 1,
            "prompt_eval_duration": 3,
            "eval_count": 1,
            "eval_duration": 4,
        }

    @staticmethod
    def _encode_sse(
        events: list[tuple[str | None, object]],
        *,
        trailing_blank: bool = True,
    ) -> str:
        frames = []
        for event_name, data in events:
            lines = []
            if event_name is not None:
                lines.append(f"event: {event_name}")
            payload = (
                data
                if isinstance(data, str)
                else json.dumps(data, separators=(",", ":"))
            )
            lines.append(f"data: {payload}")
            frames.append("\n".join(lines))
        body = "\n\n".join(frames)
        return body + ("\n\n" if trailing_blank else "")

    @staticmethod
    def _encode_ndjson(records: list[object]) -> str:
        lines = [
            record
            if isinstance(record, str)
            else json.dumps(record, separators=(",", ":"))
            for record in records
        ]
        return "\n".join(lines) + "\n"

    def _chat_stream_events(self) -> list[tuple[str | None, object]]:
        return [
            (
                None,
                {
                    "id": "chatcmpl-stream",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "gpt-fixture",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"role": "assistant", "content": "o"},
                            "finish_reason": None,
                            "logprobs": None,
                        }
                    ],
                },
            ),
            (
                None,
                {
                    "id": "chatcmpl-stream",
                    "object": "chat.completion.chunk",
                    "created": 1,
                    "model": "gpt-fixture",
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": "k"},
                            "finish_reason": "stop",
                            "logprobs": None,
                        }
                    ],
                },
            ),
            (None, "[DONE]"),
        ]

    def _responses_stream_events(self) -> list[tuple[str | None, object]]:
        started = self._responses()
        started["status"] = "in_progress"
        started["completed_at"] = None
        started["output"] = []
        started["usage"] = None
        in_progress = copy.deepcopy(started)
        completed = self._responses()
        completed["completed_at"] = 2

        item_started = {
            "id": "msg_fixture",
            "type": "message",
            "role": "assistant",
            "content": [],
            "status": "in_progress",
        }
        part_started = {
            "type": "output_text",
            "text": "",
            "annotations": [],
        }
        part_completed = {
            "type": "output_text",
            "text": "ok",
            "annotations": [],
        }
        item_completed = completed["output"][0]

        return [
            (
                "response.created",
                {
                    "type": "response.created",
                    "response": started,
                    "sequence_number": 0,
                },
            ),
            (
                "response.in_progress",
                {
                    "type": "response.in_progress",
                    "response": in_progress,
                    "sequence_number": 1,
                },
            ),
            (
                "response.output_item.added",
                {
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": item_started,
                    "sequence_number": 2,
                },
            ),
            (
                "response.content_part.added",
                {
                    "type": "response.content_part.added",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "part": part_started,
                    "sequence_number": 3,
                },
            ),
            (
                "response.output_text.delta",
                {
                    "type": "response.output_text.delta",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "delta": "ok",
                    "logprobs": [],
                    "sequence_number": 4,
                },
            ),
            (
                "response.output_text.done",
                {
                    "type": "response.output_text.done",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "text": "ok",
                    "logprobs": [],
                    "sequence_number": 5,
                },
            ),
            (
                "response.content_part.done",
                {
                    "type": "response.content_part.done",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "part": part_completed,
                    "sequence_number": 6,
                },
            ),
            (
                "response.output_item.done",
                {
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": item_completed,
                    "sequence_number": 7,
                },
            ),
            (
                "response.completed",
                {
                    "type": "response.completed",
                    "response": completed,
                    "sequence_number": 8,
                },
            ),
        ]

    def _responses_function_stream_events(
        self,
    ) -> list[tuple[str | None, object]]:
        started = self._responses()
        started["status"] = "in_progress"
        started["completed_at"] = None
        started["output"] = []
        started["usage"] = None
        in_progress = copy.deepcopy(started)

        item_started = {
            "id": "fc_fixture",
            "type": "function_call",
            "call_id": "call_fixture",
            "name": "get_weather",
            "arguments": "",
            "status": "in_progress",
        }
        item_completed = {
            **item_started,
            "arguments": '{"city":"Beijing"}',
            "status": "completed",
        }
        completed = self._responses()
        completed["completed_at"] = 2
        completed["output"] = [copy.deepcopy(item_completed)]

        return [
            (
                "response.created",
                {
                    "type": "response.created",
                    "response": started,
                    "sequence_number": 0,
                },
            ),
            (
                "response.in_progress",
                {
                    "type": "response.in_progress",
                    "response": in_progress,
                    "sequence_number": 1,
                },
            ),
            (
                "response.output_item.added",
                {
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": item_started,
                    "sequence_number": 2,
                },
            ),
            (
                "response.function_call_arguments.delta",
                {
                    "type": "response.function_call_arguments.delta",
                    "item_id": "fc_fixture",
                    "output_index": 0,
                    "delta": '{"city":',
                    "sequence_number": 3,
                },
            ),
            (
                "response.function_call_arguments.done",
                {
                    "type": "response.function_call_arguments.done",
                    "item_id": "fc_fixture",
                    "output_index": 0,
                    "name": "get_weather",
                    "arguments": '{"city":"Beijing"}',
                    "sequence_number": 4,
                },
            ),
            (
                "response.output_item.done",
                {
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": item_completed,
                    "sequence_number": 5,
                },
            ),
            (
                "response.completed",
                {
                    "type": "response.completed",
                    "response": completed,
                    "sequence_number": 6,
                },
            ),
        ]

    def _responses_item_stream_events(
        self,
        item_started: dict,
        item_completed: dict,
        middle_events: list[tuple[str, dict]],
    ) -> list[tuple[str | None, object]]:
        started = self._responses()
        started["status"] = "in_progress"
        started["completed_at"] = None
        started["output"] = []
        started["usage"] = None
        completed = self._responses()
        completed["completed_at"] = 2
        completed["output"] = [copy.deepcopy(item_completed)]
        events: list[tuple[str | None, object]] = [
            (
                "response.created",
                {"type": "response.created", "response": started},
            ),
            (
                "response.in_progress",
                {
                    "type": "response.in_progress",
                    "response": copy.deepcopy(started),
                },
            ),
            (
                "response.output_item.added",
                {
                    "type": "response.output_item.added",
                    "output_index": 0,
                    "item": copy.deepcopy(item_started),
                },
            ),
            *copy.deepcopy(middle_events),
            (
                "response.output_item.done",
                {
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": copy.deepcopy(item_completed),
                },
            ),
            (
                "response.completed",
                {"type": "response.completed", "response": completed},
            ),
        ]
        for sequence_number, (_, payload) in enumerate(events):
            assert isinstance(payload, dict)
            payload["sequence_number"] = sequence_number
        return events

    def _anthropic_stream_events(self) -> list[tuple[str | None, object]]:
        return [
            (
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_stream",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-fixture",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {"input_tokens": 1, "output_tokens": 1},
                    },
                },
            ),
            (
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "get_weather",
                        "input": {},
                    },
                },
            ),
            (
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": '{"city":',
                    },
                },
            ),
            (
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": '"Beijing"}',
                    },
                },
            ),
            (
                "content_block_stop",
                {"type": "content_block_stop", "index": 0},
            ),
            (
                "message_delta",
                {
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": "tool_use",
                        "stop_sequence": None,
                    },
                    "usage": {"output_tokens": 3},
                },
            ),
            ("message_stop", {"type": "message_stop"}),
        ]

    def _gemini_stream_events(self) -> list[tuple[str | None, object]]:
        return [
            (
                None,
                {
                    "candidates": [
                        {
                            "content": {
                                "role": "model",
                                "parts": [{"text": "o"}],
                            },
                            "index": 0,
                        }
                    ],
                    "modelVersion": "gemini-fixture",
                    "responseId": "resp-fixture",
                },
            ),
            (
                None,
                {
                    "candidates": [
                        {
                            "content": {
                                "role": "model",
                                "parts": [{"text": "k"}],
                            },
                            "finishReason": "STOP",
                            "index": 0,
                        }
                    ],
                    "usageMetadata": {
                        "promptTokenCount": 1,
                        "candidatesTokenCount": 1,
                        "totalTokenCount": 2,
                    },
                    "modelVersion": "gemini-fixture",
                    "responseId": "resp-fixture",
                },
            ),
        ]

    def _ollama_stream_records(self) -> list[object]:
        return [
            {
                "model": "ollama-fixture",
                "created_at": "2026-08-18T00:00:00Z",
                "message": {"role": "assistant", "content": "o"},
                "done": False,
            },
            {
                "model": "ollama-fixture",
                "created_at": "2026-08-18T00:00:01Z",
                "message": {"role": "assistant", "content": "k"},
                "done": True,
                "done_reason": "stop",
                "total_duration": 10,
                "load_duration": 2,
                "prompt_eval_count": 1,
                "prompt_eval_duration": 3,
                "eval_count": 1,
                "eval_duration": 4,
            },
        ]

    def _valid_stream_fixtures(self) -> dict[str, tuple[str, str]]:
        return {
            "openai_chat": (
                self._encode_sse(self._chat_stream_events()),
                "content-type: text/event-stream; charset=utf-8",
            ),
            "openai_responses": (
                self._encode_sse(self._responses_stream_events()),
                "content-type: text/event-stream",
            ),
            "anthropic_messages": (
                self._encode_sse(self._anthropic_stream_events()),
                "content-type: text/event-stream",
            ),
            "gemini_generate_content": (
                self._encode_sse(
                    self._gemini_stream_events(),
                    trailing_blank=False,
                ),
                "content-type: text/event-stream",
            ),
            "ollama_chat": (
                self._encode_ndjson(self._ollama_stream_records()),
                "content-type: application/x-ndjson",
            ),
        }

    @staticmethod
    def _fixture_body(name: str) -> str:
        return (SKILL_DIR.parents[1] / "src/protocol/fixtures" / name).read_text(
            encoding="utf-8"
        )

    @staticmethod
    def _tool_schema(*, closed: bool) -> dict:
        schema = {
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        }
        if closed:
            schema["additionalProperties"] = False
        return schema

    def _tool_loop_bodies(self, protocol: str) -> tuple[dict, dict, str, str]:
        prompt = "MODEL_DOCTOR_CASE_046"
        output = "WEATHER_SUNNY"
        if protocol == "openai_chat":
            tool = {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather.",
                    "strict": True,
                    "parameters": self._tool_schema(closed=True),
                },
            }
            initial = {
                "model": "gpt-test",
                "messages": [{"role": "user", "content": prompt}],
                "tools": [tool],
                "tool_choice": "auto",
                "parallel_tool_calls": False,
                "stream": True,
            }
            follow = copy.deepcopy(initial)
            follow["messages"].extend([
                {
                    "role": "assistant",
                    "content": None,
                    "tool_calls": [{
                        "id": "call_weather_046",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": '{"city":"Beijing"}',
                        },
                    }],
                },
                {"role": "tool", "tool_call_id": "call_weather_046", "content": output},
            ])
            return initial, follow, "openai_chat_tool.sse", "openai_chat_final.sse"
        if protocol == "openai_responses":
            tool = {
                "type": "function",
                "name": "get_weather",
                "description": "Get weather.",
                "parameters": self._tool_schema(closed=True),
                "strict": True,
            }
            initial = {
                "model": "gpt-test",
                "input": prompt,
                "tools": [tool],
                "tool_choice": "auto",
                "parallel_tool_calls": False,
                "store": True,
                "stream": True,
            }
            follow = {
                **copy.deepcopy(initial),
                "input": [{
                    "type": "function_call_output",
                    "call_id": "call_fixture",
                    "output": output,
                }],
                "previous_response_id": "resp_fixture",
            }
            return initial, follow, "openai_responses_tool.sse", "openai_responses_final.sse"
        if protocol == "anthropic_messages":
            tool = {
                "name": "get_weather",
                "description": "Get weather.",
                "input_schema": self._tool_schema(closed=False),
            }
            initial = {
                "model": "claude-test",
                "max_tokens": 1024,
                "messages": [{"role": "user", "content": prompt}],
                "tools": [tool],
                "stream": True,
            }
            follow = copy.deepcopy(initial)
            follow["messages"].extend([
                {
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "get_weather",
                        "input": {"city": "Beijing"},
                    }],
                },
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": output,
                    }],
                },
            ])
            return initial, follow, "anthropic_tool.sse", "anthropic_final.sse"
        if protocol == "gemini_generate_content":
            tool = {
                "functionDeclarations": [{
                    "name": "get_weather",
                    "description": "Get weather.",
                    "parameters": self._tool_schema(closed=False),
                }]
            }
            initial = {
                "contents": [{"role": "user", "parts": [{"text": prompt}]}],
                "tools": [tool],
            }
            follow = copy.deepcopy(initial)
            follow["contents"].extend([
                {
                    "role": "model",
                    "parts": [
                        {
                            "text": "private reasoning",
                            "thought": True,
                            "thoughtSignature": "sig-thought",
                        },
                        {
                            "functionCall": {
                                "id": "call_weather_046",
                                "name": "get_weather",
                                "args": {"city": "Beijing"},
                            },
                            "thoughtSignature": "sig-call",
                        },
                        {"text": "", "thoughtSignature": "sig-empty"},
                    ],
                },
                {
                    "role": "user",
                    "parts": [{
                        "functionResponse": {
                            "id": "call_weather_046",
                            "name": "get_weather",
                            "response": {"result": output},
                        }
                    }],
                },
            ])
            return initial, follow, "gemini_tool.sse", "gemini_final.sse"
        if protocol == "ollama_chat":
            tool = {
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather.",
                    "parameters": self._tool_schema(closed=False),
                },
            }
            initial = {
                "model": "qwen3",
                "messages": [{"role": "user", "content": prompt}],
                "tools": [tool],
                "stream": True,
            }
            follow = copy.deepcopy(initial)
            follow["messages"].extend([
                {
                    "role": "assistant",
                    "thinking": "Need weather.",
                    "content": "",
                    "tool_calls": [
                        {
                            "function": {
                                "name": "get_time",
                                "arguments": {"zone": "UTC"},
                            },
                        },
                        {
                            "function": {
                                "name": "get_weather",
                                "arguments": {"city": "Beijing"},
                            },
                        },
                    ],
                },
                {"role": "tool", "tool_name": "get_time", "content": "TIME_UTC_00:00"},
                {"role": "tool", "tool_name": "get_weather", "content": output},
            ])
            return initial, follow, "ollama_tool.ndjson", "ollama_final.ndjson"
        raise AssertionError(protocol)

    def _tool_loop_parsed(self, protocol: str) -> dict:
        initial, follow, tool_fixture, final_fixture = self._tool_loop_bodies(protocol)
        content_type = (
            "content-type: application/x-ndjson"
            if protocol == "ollama_chat"
            else "content-type: text/event-stream"
        )
        tool_response = self._fixture_body(tool_fixture)
        final_response = self._fixture_body(final_fixture)
        if protocol == "openai_responses":
            tool_response = self._encode_sse(self._responses_function_stream_events())
            final_response = self._encode_sse(self._responses_stream_events())
        elif protocol == "anthropic_messages":
            tool_response = self._encode_sse(self._anthropic_stream_events())
        elif protocol == "ollama_chat":
            tool_response = self._encode_ndjson([
                {
                    "model": "ollama-fixture",
                    "created_at": "2026-08-18T00:00:00Z",
                    "message": {
                        "role": "assistant",
                        "thinking": "Need weather.",
                        "content": "",
                        "tool_calls": [
                            {"function": {"name": "get_time", "arguments": {"zone": "UTC"}}},
                            {"function": {"name": "get_weather", "arguments": {"city": "Beijing"}}},
                        ],
                    },
                    "done": False,
                },
                {
                    "model": "ollama-fixture",
                    "created_at": "2026-08-18T00:00:01Z",
                    "message": {"role": "assistant", "content": ""},
                    "done": True,
                    "done_reason": "stop",
                    "total_duration": 10,
                    "load_duration": 2,
                    "prompt_eval_count": 1,
                    "prompt_eval_duration": 3,
                    "eval_count": 1,
                    "eval_duration": 4,
                },
            ])
            final_response = self._encode_ndjson(self._ollama_stream_records())
        requests = {}
        for turn, (body, response) in enumerate(
            ((initial, tool_response), (follow, final_response)), start=1
        ):
            request_id = f"test-046-turn-{turn}"
            requests[request_id] = self._request(
                request_id,
                protocol,
                request_body=body,
                raw_response=response,
                stream="1",
                response_headers=content_type,
            )
        return {
            "run": {
                "log_schema": "llm-capability-doctor.evidence.v3",
                "script_version": "0.11.0",
            },
            "requests": requests,
            "tests": {
                "046": {
                    "requestRefs": ["test-046-turn-1", "test-046-turn-2"],
                }
            },
        }

    def _ollama_tool_loop_with_ids(self) -> dict:
        parsed = self._tool_loop_parsed("ollama_chat")
        first = parsed["requests"]["test-046-turn-1"]
        records = [
            json.loads(line)
            for line in first["responseBody"].splitlines()
            if line.strip()
        ]
        calls = records[0]["message"]["tool_calls"]
        calls[0]["id"] = "call-time"
        calls[0]["function"]["index"] = 0
        calls[1]["id"] = "call-weather"
        calls[1]["function"]["index"] = 1
        first["responseBody"] = self._encode_ndjson(records)

        follow = parsed["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        body["messages"][1]["tool_calls"] = copy.deepcopy(calls)
        body["messages"][2]["tool_call_id"] = "call-time"
        body["messages"][3]["tool_call_id"] = "call-weather"
        follow["requestBody"] = json.dumps(body)
        return parsed

    def _v3_pass_gate_fixture(self, protocol: str) -> tuple[dict, dict]:
        parsed = self._tool_loop_parsed(protocol)
        parsed["tests"]["046"].update(
            {"name": "Official tool loop", "category": "Core"}
        )
        end_signal = {
            "openai_chat": "[DONE]",
            "openai_responses": "response.completed",
            "anthropic_messages": "message_stop",
            "gemini_generate_content": "finishReason:STOP",
            "ollama_chat": "done:true",
        }[protocol]
        for turn in (1, 2):
            request = parsed["requests"][f"test-046-turn-{turn}"]
            request.update({
                "transport_outcome": "completed_eof",
                "stream_termination": "completed",
                "stream_end_signal": end_signal,
                "model_stop_reason": "stop" if turn == 2 else "tool_calls",
                "stream_event_count": "1",
                "tool_contract_status": "conformant",
                "tool_contract_errors_json": "[]",
                "tool_loop_turn": str(turn),
                "tool_loop_outcome": "completed" if turn == 2 else "continued",
            })
        reviews = {
            "schemaVersion": "llm-capability-doctor.reviews.v2",
            "tests": {
                "046": {
                    "testId": "046",
                    "reviewedStatus": "PASS",
                    "conclusion": "The complete official tool loop passed.",
                    "logic": {
                        "purpose": "Check a complete tool loop.",
                        "method": "Inspect every ordered request.",
                        "passCriteria": ["Every official turn completes."],
                        "failCriteria": ["Any official turn is invalid."],
                        "capabilityBoundary": "This conclusion covers this run.",
                    },
                    "evidenceRefs": [
                        "request:test-046-turn-1",
                        "request:test-046-turn-2",
                    ],
                    "evidenceExcerpts": ["The official loop completed."],
                    "limitations": [],
                    "retestInstructions": [],
                }
            },
            "capabilitySummary": {
                "headline": "The collected capability evidence was reviewed.",
                "verifiedFacts": {
                    "interfaceProtocol": {
                        "evidenceState": "NOT_COLLECTED",
                        "family": "UNKNOWN",
                        "requestFormat": "No interface request format was collected.",
                        "responseFormat": "No interface response format was collected.",
                        "statement": "No interface protocol fact was collected.",
                        "evidenceRefs": [],
                        "boundary": "No protocol can be inferred without evidence.",
                    },
                    "contextWindow": {
                        "evidenceState": "NOT_COLLECTED",
                        "highestVerifiedTier": None,
                        "highestVerifiedInputTokens": None,
                        "firstFailedTier": None,
                        "firstFailedInputTokens": None,
                        "statement": "No context tier evidence was collected.",
                        "evidenceRefs": [],
                        "boundary": "No context limit can be inferred.",
                    },
                    "concurrency": {
                        "evidenceState": "NOT_COLLECTED",
                        "highestVerifiedConcurrentRequests": None,
                        "statement": "No concurrency wave was collected.",
                        "evidenceRefs": [],
                        "boundary": "No concurrency limit can be inferred.",
                    },
                },
                "issues": [],
                "scopeBoundary": CAPABILITY_SCOPE_BOUNDARY,
            },
        }
        return parsed, reviews

    def _single_result(self, request: dict) -> dict:
        report = analyze_protocol_conformance(
            self._parsed({request["request_id"]: request})
        )
        self.assertEqual([], validate_protocol_conformance(report))
        return report["results"][0]

    def _assert_one_difference(
        self,
        result: dict,
        location: str,
        difference_kind: str,
    ) -> None:
        self.assertEqual("DIFFERENT", result["status"])
        self.assertEqual(1, len(result["differences"]), result["differences"])
        self.assertEqual(location, result["differences"][0]["location"])
        self.assertEqual(
            difference_kind,
            result["differences"][0]["differenceKind"],
        )

    def _assert_difference(
        self,
        result: dict,
        location: str,
        difference_kind: str,
    ) -> None:
        self.assertEqual("DIFFERENT", result["status"])
        self.assertIn(
            (location, difference_kind),
            [
                (difference["location"], difference["differenceKind"])
                for difference in result["differences"]
            ],
            result["differences"],
        )

    def test_v3_tool_transition_matrix_accepts_all_five_official_follow_ups(self) -> None:
        for protocol in SUPPORTED_PROTOCOLS:
            with self.subTest(protocol=protocol):
                report = analyze_protocol_conformance(self._tool_loop_parsed(protocol))
                self.assertEqual([], validate_protocol_conformance(report))
                results = {item["requestId"]: item for item in report["results"]}
                self.assertEqual(
                    "CONSISTENT",
                    results["test-046-turn-2"]["status"],
                    results["test-046-turn-2"]["differences"],
                )

    def test_v3_ollama_optional_ids_follow_the_pinned_native_contract(self) -> None:
        parsed = self._ollama_tool_loop_with_ids()
        report = analyze_protocol_conformance(parsed)
        self.assertEqual([], validate_protocol_conformance(report))
        self.assertTrue(
            all(result["status"] == "CONSISTENT" for result in report["results"]),
            report["results"],
        )

        mismatched = copy.deepcopy(parsed)
        follow = mismatched["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        body["messages"][2]["tool_call_id"] = "wrong-call"
        follow["requestBody"] = json.dumps(body)
        mismatch_report = analyze_protocol_conformance(mismatched)
        mismatch_result = next(
            item for item in mismatch_report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self._assert_difference(
            mismatch_result,
            "/requestBody/messages/2/tool_call_id",
            "CORRELATION",
        )

        invented = self._tool_loop_parsed("ollama_chat")
        follow = invented["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        body["messages"][2]["tool_call_id"] = "invented-call"
        follow["requestBody"] = json.dumps(body)
        invented_report = analyze_protocol_conformance(invented)
        invented_result = next(
            item for item in invented_report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self._assert_difference(
            invented_result,
            "/requestBody/messages/2/tool_call_id",
            "UNEXPECTED_FIELD",
        )

    def test_v3_ollama_accepts_native_id_and_index_but_rejects_call_type(self) -> None:
        parsed = self._ollama_tool_loop_with_ids()
        first = parsed["requests"]["test-046-turn-1"]
        records = [
            json.loads(line)
            for line in first["responseBody"].splitlines()
            if line.strip()
        ]
        records[0]["message"]["tool_calls"][0]["type"] = "function"
        first["responseBody"] = self._encode_ndjson(records)
        follow = parsed["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        body["messages"][1]["tool_calls"][0]["type"] = "function"
        follow["requestBody"] = json.dumps(body)

        report = analyze_protocol_conformance(parsed)
        self.assertEqual([], validate_protocol_conformance(report))
        first_result = next(
            item for item in report["results"]
            if item["requestId"] == "test-046-turn-1"
        )
        self._assert_difference(
            first_result,
            "/records/0/message/tool_calls/0/type",
            "UNEXPECTED_FIELD",
        )

    def test_v3_tool_transition_matrix_rejects_all_five_correlation_mutations(self) -> None:
        mutations = {
            "openai_chat": lambda body: body["messages"][-1].__setitem__(
                "tool_call_id", "wrong-call"
            ),
            "openai_responses": lambda body: body.__setitem__(
                "previous_response_id", "wrong-response"
            ),
            "anthropic_messages": lambda body: body["messages"][-1]["content"][0].__setitem__(
                "tool_use_id", "wrong-tool"
            ),
            "gemini_generate_content": lambda body: body["contents"][-1]["parts"][0][
                "functionResponse"
            ].__setitem__("id", "wrong-call"),
            "ollama_chat": lambda body: body["messages"][-1].__setitem__(
                "tool_name", "wrong_tool"
            ),
        }
        for protocol, mutate in mutations.items():
            with self.subTest(protocol=protocol):
                parsed = self._tool_loop_parsed(protocol)
                follow = parsed["requests"]["test-046-turn-2"]
                body = json.loads(follow["requestBody"])
                mutate(body)
                follow["requestBody"] = json.dumps(body)

                report = analyze_protocol_conformance(parsed)
                self.assertEqual([], validate_protocol_conformance(report))
                result = next(
                    item for item in report["results"]
                    if item["requestId"] == "test-046-turn-2"
                )

                self.assertEqual("DIFFERENT", result["status"], result)
                self.assertTrue(
                    any(
                        difference["differenceKind"] == "CORRELATION"
                        for difference in result["differences"]
                    ),
                    result["differences"],
                )

    def test_v3_tool_transition_rejects_foreign_initial_fields(self) -> None:
        mutations = {
            "openai_chat": ("contents", []),
            "openai_responses": ("messages", []),
            "anthropic_messages": ("previous_response_id", "resp-foreign"),
            "gemini_generate_content": ("stream", True),
            "ollama_chat": ("tool_choice", "auto"),
        }
        for protocol, (field, value) in mutations.items():
            with self.subTest(protocol=protocol):
                parsed = self._tool_loop_parsed(protocol)
                initial = parsed["requests"]["test-046-turn-1"]
                body = json.loads(initial["requestBody"])
                body[field] = value
                initial["requestBody"] = json.dumps(body)

                report = analyze_protocol_conformance(parsed)
                self.assertEqual([], validate_protocol_conformance(report))
                result = next(
                    item
                    for item in report["results"]
                    if item["requestId"] == "test-046-turn-1"
                )
                self.assertEqual("DIFFERENT", result["status"])
                self.assertTrue(
                    any(
                        difference["differenceKind"] == "UNEXPECTED_FIELD"
                        and difference["location"] == f"/requestBody/{field}"
                        for difference in result["differences"]
                    ),
                    result["differences"],
                )

    def test_v3_pass_gate_rejects_empty_initial_prompt_for_all_protocols(self) -> None:
        def clear_prompt(protocol: str, body: dict) -> None:
            if protocol in {"openai_chat", "anthropic_messages", "ollama_chat"}:
                body["messages"][0]["content"] = ""
            elif protocol == "openai_responses":
                body["input"] = ""
            else:
                body["contents"][0]["parts"] = []

        for protocol in SUPPORTED_PROTOCOLS:
            with self.subTest(protocol=protocol):
                parsed, reviews = self._v3_pass_gate_fixture(protocol)
                self.assertEqual([], validate_reviews(parsed, reviews))
                for request in parsed["requests"].values():
                    body = json.loads(request["requestBody"])
                    clear_prompt(protocol, body)
                    request["requestBody"] = json.dumps(body)

                errors = validate_reviews(parsed, reviews)

                self.assertTrue(
                    any("official protocol" in error for error in errors),
                    errors,
                )

    def test_v3_pass_gate_rejects_invalid_provider_native_message_unions(self) -> None:
        cases = (
            ("openai_chat", "user-object"),
            ("ollama_chat", "user-object"),
            ("anthropic_messages", "tool-result-object"),
            ("gemini_generate_content", "empty-part"),
        )
        for protocol, mutation in cases:
            with self.subTest(protocol=protocol, mutation=mutation):
                parsed, reviews = self._v3_pass_gate_fixture(protocol)
                request_id = (
                    "test-046-turn-2"
                    if mutation == "tool-result-object"
                    else "test-046-turn-1"
                )
                request = parsed["requests"][request_id]
                body = json.loads(request["requestBody"])
                if mutation == "user-object":
                    body["messages"][0]["content"] = {"invalid": True}
                elif mutation == "tool-result-object":
                    body["messages"][-1]["content"][0]["content"] = {
                        "invalid": True
                    }
                else:
                    body["contents"][0]["parts"].append({})
                request["requestBody"] = json.dumps(body)

                errors = validate_reviews(parsed, reviews)
                self.assertTrue(
                    any("official protocol" in error for error in errors),
                    errors,
                )

    def test_v3_pass_gate_rejects_invalid_required_and_control_fields(self) -> None:
        cases = (
            ("anthropic_messages", "missing-max-tokens"),
            ("anthropic_messages", "boolean-max-tokens"),
            ("anthropic_messages", "zero-max-tokens"),
            ("openai_chat", "numeric-parallel"),
            ("openai_responses", "numeric-parallel"),
            ("openai_chat", "boolean-tool-choice"),
            ("openai_responses", "boolean-tool-choice"),
        )
        for protocol, mutation in cases:
            with self.subTest(protocol=protocol, mutation=mutation):
                parsed, reviews = self._v3_pass_gate_fixture(protocol)
                request = parsed["requests"]["test-046-turn-1"]
                body = json.loads(request["requestBody"])
                if mutation == "missing-max-tokens":
                    del body["max_tokens"]
                elif mutation == "boolean-max-tokens":
                    body["max_tokens"] = True
                elif mutation == "zero-max-tokens":
                    body["max_tokens"] = 0
                elif mutation == "numeric-parallel":
                    body["parallel_tool_calls"] = 1
                else:
                    body["tool_choice"] = True
                request["requestBody"] = json.dumps(body)

                errors = validate_reviews(parsed, reviews)
                self.assertTrue(
                    any("official protocol" in error for error in errors),
                    errors,
                )

    def test_v3_json_decoding_rejects_nonfinite_constants(self) -> None:
        parsed, reviews = self._v3_pass_gate_fixture("openai_chat")
        for request in parsed["requests"].values():
            body = json.loads(request["requestBody"])
            body["nonfinite"] = float("inf")
            request["requestBody"] = json.dumps(body)

        errors = validate_reviews(parsed, reviews)
        self.assertTrue(
            any("official protocol" in error for error in errors),
            errors,
        )

    def test_v3_json_decoding_is_total_for_deep_request_sse_and_ndjson(self) -> None:
        deep_json = "[" * 1200 + "0" + "]" * 1200
        cases = (
            ("openai_chat", "request"),
            ("openai_chat", "sse"),
            ("ollama_chat", "ndjson"),
        )
        for protocol, carrier in cases:
            with self.subTest(protocol=protocol, carrier=carrier):
                parsed = self._tool_loop_parsed(protocol)
                first = parsed["requests"]["test-046-turn-1"]
                if carrier == "request":
                    first["requestBody"] = deep_json
                elif carrier == "sse":
                    first["responseBody"] = f"data: {deep_json}\n\n"
                else:
                    first["responseBody"] = f"{deep_json}\n"

                report = analyze_protocol_conformance(parsed)
                self.assertEqual([], validate_protocol_conformance(report))
                self.assertTrue(
                    any(result["status"] == "DIFFERENT" for result in report["results"]),
                    report["results"],
                )

    def test_v3_malformed_protocol_differences_keep_result_ownership(self) -> None:
        for mutation in ("missing", "null", "boolean", "request-null"):
            with self.subTest(mutation=mutation):
                parsed = self._tool_loop_parsed("openai_chat")
                request_id = "test-046-turn-1"
                if mutation == "request-null":
                    parsed["requests"][request_id] = None
                elif mutation == "missing":
                    del parsed["requests"][request_id]["protocol"]
                elif mutation == "null":
                    parsed["requests"][request_id]["protocol"] = None
                else:
                    parsed["requests"][request_id]["protocol"] = True

                report = analyze_protocol_conformance(parsed)
                self.assertEqual([], validate_protocol_conformance(report))
                result = next(
                    item for item in report["results"]
                    if item["requestId"] == request_id
                )
                self.assertTrue(result["differences"], result)
                self.assertTrue(
                    all(
                        difference["protocol"] == result["protocol"]
                        for difference in result["differences"]
                    ),
                    result,
                )

    def test_v3_transition_deduplicates_diagnostics_by_identity(self) -> None:
        parsed = self._tool_loop_parsed("openai_chat")
        follow = parsed["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        body["stream"] = False
        follow["requestBody"] = json.dumps(body)

        report = analyze_protocol_conformance(parsed)
        self.assertEqual([], validate_protocol_conformance(report))
        result = next(
            item for item in report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        matching = [
            difference
            for difference in result["differences"]
            if difference["location"] == "/requestBody/stream"
            and difference["differenceKind"] == "VALUE_MISMATCH"
        ]
        self.assertEqual(1, len(matching), result["differences"])

    def test_v3_tool_transition_rejects_adjacent_protocol_switch(self) -> None:
        parsed = self._tool_loop_parsed("openai_chat")
        parsed["requests"]["test-046-turn-2"]["protocol"] = "ollama_chat"

        report = analyze_protocol_conformance(parsed)

        self.assertEqual([], validate_protocol_conformance(report))
        result = next(
            item for item in report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self.assertTrue(
            any(
                difference["location"] == "/protocol"
                and difference["differenceKind"] == "CORRELATION"
                for difference in result["differences"]
            ),
            result["differences"],
        )

    def test_anthropic_timeout_transition_requires_is_error(self) -> None:
        parsed = self._tool_loop_parsed("anthropic_messages")
        renamed = {}
        for turn in (1, 2):
            old_id = f"test-046-turn-{turn}"
            new_id = f"test-049-turn-{turn}"
            request = parsed["requests"][old_id]
            request["request_id"] = new_id
            renamed[new_id] = request
        parsed["requests"] = renamed
        parsed["tests"] = {
            "049": {"requestRefs": ["test-049-turn-1", "test-049-turn-2"]}
        }
        follow = parsed["requests"]["test-049-turn-2"]
        body = json.loads(follow["requestBody"])
        body["messages"][-1]["content"][0]["content"] = "ERROR: timeout"
        follow["requestBody"] = json.dumps(body)

        missing_flag = analyze_protocol_conformance(parsed)
        result = next(
            item for item in missing_flag["results"]
            if item["requestId"] == "test-049-turn-2"
        )
        self.assertTrue(
            any(
                difference["location"].endswith("/is_error")
                for difference in result["differences"]
            ),
            result["differences"],
        )

        body["messages"][-1]["content"][0]["is_error"] = True
        follow["requestBody"] = json.dumps(body)
        complete = analyze_protocol_conformance(parsed)
        result = next(
            item for item in complete["results"]
            if item["requestId"] == "test-049-turn-2"
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_anthropic_transition_allows_text_only_after_tool_results(self) -> None:
        parsed = self._tool_loop_parsed("anthropic_messages")
        follow = parsed["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        content = body["messages"][-1]["content"]
        content.append({"type": "text", "text": "continue"})
        follow["requestBody"] = json.dumps(body)

        accepted = analyze_protocol_conformance(parsed)
        result = next(
            item for item in accepted["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        content.reverse()
        follow["requestBody"] = json.dumps(body)
        rejected = analyze_protocol_conformance(parsed)
        result = next(
            item for item in rejected["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self.assertEqual("DIFFERENT", result["status"])
        self.assertTrue(
            any(
                difference["differenceKind"] == "VALUE_MISMATCH"
                for difference in result["differences"]
            ),
            result["differences"],
        )

    def test_gemini_transition_accepts_omitted_optional_call_id(self) -> None:
        parsed = self._tool_loop_parsed("gemini_generate_content")
        initial = parsed["requests"]["test-046-turn-1"]
        initial["responseBody"] = initial["responseBody"].replace(
            '"id":"call_weather_046",',
            "",
        )
        follow = parsed["requests"]["test-046-turn-2"]
        body = json.loads(follow["requestBody"])
        del body["contents"][-2]["parts"][1]["functionCall"]["id"]
        del body["contents"][-1]["parts"][0]["functionResponse"]["id"]
        follow["requestBody"] = json.dumps(body)

        report = analyze_protocol_conformance(parsed)

        self.assertEqual([], validate_protocol_conformance(report))
        result = next(
            item for item in report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_ollama_transition_preserves_extensions_and_uses_function_index_order(self) -> None:
        parsed = self._tool_loop_parsed("ollama_chat")
        initial = parsed["requests"]["test-046-turn-1"]
        initial["responseBody"] = self._fixture_body("ollama_tool.ndjson")
        initial_body = json.loads(initial["requestBody"])
        initial_body["tools"].append({
            "type": "function",
            "function": {
                "name": "get_time",
                "description": "Get time.",
                "parameters": {
                    "type": "object",
                    "properties": {"zone": {"type": "string"}},
                    "required": ["zone"],
                },
            },
        })
        initial["requestBody"] = json.dumps(initial_body)

        follow = parsed["requests"]["test-046-turn-2"]
        follow_body = copy.deepcopy(initial_body)
        follow_body["messages"].extend([
            {
                "role": "assistant",
                "content": "Calling tools.",
                "thinking": "Need weather. ",
                "future_message": {"trace": 1},
                "tool_calls": [
                    {
                        "id": "call_weather_046",
                        "type": "function",
                        "function": {
                            "index": 1,
                            "name": "get_weather",
                            "arguments": {
                                "city": "Beijing",
                                "units": {"temperature": "celsius"},
                            },
                            "future_function": "keep",
                        },
                        "future_call": "keep",
                    },
                    {
                        "id": "call_time_046",
                        "type": "function",
                        "function": {
                            "index": 0,
                            "name": "get_time",
                            "arguments": {"zone": "UTC"},
                        },
                    },
                ],
            },
            {"role": "tool", "tool_name": "get_time", "content": "TIME_UTC_00:00"},
            {"role": "tool", "tool_name": "get_weather", "content": "WEATHER_SUNNY"},
        ])
        follow["requestBody"] = json.dumps(follow_body)

        report = analyze_protocol_conformance(parsed)

        self.assertEqual([], validate_protocol_conformance(report))
        result = next(
            item for item in report["results"]
            if item["requestId"] == "test-046-turn-2"
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_public_constants_and_pinned_baselines(self) -> None:
        self.assertEqual(
            "official-protocol-conformance.2026-08-18",
            RULE_SET_VERSION,
        )
        self.assertEqual("2026-08-18", BASELINE_DATE)
        self.assertEqual(
            (
                "openai_chat",
                "openai_responses",
                "anthropic_messages",
                "gemini_generate_content",
                "ollama_chat",
            ),
            SUPPORTED_PROTOCOLS,
        )
        self.assertEqual(
            (
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
            ),
            DIFFERENCE_KINDS,
        )

        report = analyze_protocol_conformance(self._parsed({}))
        self.assertEqual(list(SUPPORTED_PROTOCOLS), [
            item["protocol"] for item in report["baselines"]
        ])
        self.assertTrue(all(
            item["referenceDate"] == BASELINE_DATE
            and item["sourceUrl"].startswith("https://")
            and item["supportingReferences"]
            for item in report["baselines"]
        ))
        self.assertEqual([], validate_protocol_conformance(report))

    def test_each_nonstream_success_profile_is_consistent(self) -> None:
        fixtures = {
            "openai_chat": self._chat(),
            "openai_responses": self._responses(),
            "anthropic_messages": self._anthropic(),
            "gemini_generate_content": self._gemini(),
            "ollama_chat": self._ollama(),
        }

        for protocol, response in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(f"req-{protocol}", protocol, response)
                )
                self.assertEqual("CONSISTENT", result["status"])
                self.assertEqual([], result["differences"])

    def test_chat_function_arguments_must_be_json_object_string(self) -> None:
        response = self._chat()
        response["choices"][0]["message"] = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": {"city": "Beijing"},
                    },
                }
            ],
        }
        response["choices"][0]["finish_reason"] = "tool_calls"
        result = self._single_result(
            self._request("req-chat-tool", "openai_chat", response)
        )
        self._assert_one_difference(
            result,
            "/choices/0/message/tool_calls/0/function/arguments",
            "TYPE_MISMATCH",
        )

    def test_responses_function_arguments_must_be_json_object_string(self) -> None:
        response = self._responses()
        response["output"] = [
            {
                "id": "fc_fixture",
                "type": "function_call",
                "call_id": "call_1",
                "name": "get_weather",
                "arguments": {"city": "Beijing"},
                "status": "completed",
            }
        ]
        result = self._single_result(
            self._request("req-responses-tool", "openai_responses", response)
        )
        self._assert_one_difference(
            result,
            "/output/0/arguments",
            "TYPE_MISMATCH",
        )

    def test_anthropic_tool_input_must_be_object(self) -> None:
        response = self._anthropic()
        response["content"] = [
            {
                "type": "tool_use",
                "id": "toolu_1",
                "name": "get_weather",
                "input": '{"city":"Beijing"}',
            }
        ]
        response["stop_reason"] = "tool_use"
        result = self._single_result(
            self._request("req-anthropic-tool", "anthropic_messages", response)
        )
        self._assert_one_difference(
            result,
            "/content/0/input",
            "TYPE_MISMATCH",
        )

    def test_gemini_required_function_args_must_be_object(self) -> None:
        response = self._gemini()
        response["candidates"][0]["content"]["parts"] = [
            {
                "functionCall": {
                    "name": "get_weather",
                    "args": '{"city":"Beijing"}',
                }
            }
        ]
        request_body = {
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "get_weather",
                            "parameters": {
                                "type": "OBJECT",
                                "properties": {"city": {"type": "STRING"}},
                                "required": ["city"],
                            },
                        }
                    ]
                }
            ]
        }
        result = self._single_result(
            self._request(
                "req-gemini-tool",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self._assert_one_difference(
            result,
            "/candidates/0/content/parts/0/functionCall/args",
            "TYPE_MISMATCH",
        )

    def test_ollama_function_arguments_must_be_object(self) -> None:
        response = self._ollama()
        response["message"]["content"] = ""
        response["message"]["tool_calls"] = [
            {
                "function": {
                    "name": "get_weather",
                    "arguments": '{"city":"Beijing"}',
                }
            }
        ]
        result = self._single_result(
            self._request("req-ollama-tool", "ollama_chat", response)
        )
        self._assert_one_difference(
            result,
            "/message/tool_calls/0/function/arguments",
            "TYPE_MISMATCH",
        )

    def test_optional_omission_and_missing_content_type_pass(self) -> None:
        response = self._chat()
        response.pop("usage")
        response["choices"][0].pop("logprobs")
        result = self._single_result(
            self._request(
                "req-optional",
                "openai_chat",
                response,
                response_headers="x-request-id: req-fixture",
            )
        )
        self.assertEqual("CONSISTENT", result["status"])

    def test_undocumented_field_is_rejected(self) -> None:
        response = self._chat()
        response["undocumented"] = "do not echo this"
        result = self._single_result(
            self._request("req-extra", "openai_chat", response)
        )
        self._assert_one_difference(
            result,
            "/undocumented",
            "UNEXPECTED_FIELD",
        )
        self.assertEqual("present", result["differences"][0]["actual"])

    def test_invalid_json_is_rejected_without_echoing_body(self) -> None:
        result = self._single_result(
            self._request(
                "req-json",
                "openai_chat",
                raw_response='{"id":',
            )
        )
        self._assert_one_difference(result, "/responseBody", "INVALID_JSON")
        self.assertNotIn('{"id":', json.dumps(result))

    def test_valid_error_envelopes_for_all_protocols(self) -> None:
        fixtures = {
            "openai_chat": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "openai_responses": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "anthropic_messages": {
                "type": "error",
                "error": {"type": "invalid_request_error", "message": "bad"},
                "request_id": "req_fixture",
            },
            "gemini_generate_content": {
                "error": {"code": 400, "message": "bad", "status": "INVALID_ARGUMENT"}
            },
            "ollama_chat": {"error": "bad request"},
        }
        for protocol, response in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-error-{protocol}",
                        protocol,
                        response,
                        http_status="400",
                    )
                )
                self.assertEqual("CONSISTENT", result["status"])

    def test_invalid_error_envelopes_for_all_four_families(self) -> None:
        fixtures = {
            "openai_chat": (
                {"error": {"type": "bad", "message": "bad", "code": None}},
                "/error/param",
                "MISSING_FIELD",
            ),
            "anthropic_messages": (
                {"type": "error", "error": {"type": "bad", "message": 1}},
                "/error/message",
                "TYPE_MISMATCH",
            ),
            "gemini_generate_content": (
                {"error": {"code": 401, "message": "bad"}},
                "/error/code",
                "VALUE_MISMATCH",
            ),
            "ollama_chat": (
                {"error": {"message": "bad"}},
                "/error",
                "TYPE_MISMATCH",
            ),
        }
        for protocol, (response, location, kind) in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-invalid-error-{protocol}",
                        protocol,
                        response,
                        http_status="400",
                    )
                )
                self._assert_one_difference(result, location, kind)

    def test_unknown_and_missing_protocol_are_evidence_gaps(self) -> None:
        for protocol in ("custom_protocol", None):
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(f"req-{protocol}", protocol, self._chat())
                )
                self._assert_one_difference(result, "/protocol", "EVIDENCE_GAP")

    def test_transport_missing_body_and_invalid_status_are_evidence_gaps(self) -> None:
        missing_body = self._request("req-missing-body", "openai_chat", self._chat())
        missing_body.pop("responseBody")
        fixtures = (
            self._request(
                "req-transport",
                "openai_chat",
                raw_response="",
                curl_exit_code="7",
            ),
            missing_body,
            self._request(
                "req-status",
                "openai_chat",
                self._chat(),
                http_status="not-a-status",
            ),
        )
        locations = (
            "/metrics/curl_exit_code",
            "/responseBody",
            "/metrics/http_status",
        )
        for request, location in zip(fixtures, locations):
            with self.subTest(request_id=request["request_id"]):
                result = self._single_result(request)
                self._assert_one_difference(result, location, "EVIDENCE_GAP")

    def test_recorded_empty_body_is_a_protocol_difference(self) -> None:
        nonstream = self._single_result(
            self._request("req-empty-json", "openai_chat", raw_response="")
        )
        self._assert_one_difference(nonstream, "/responseBody", "INVALID_JSON")

        stream = self._single_result(
            self._request(
                "req-empty-stream",
                "openai_chat",
                raw_response="",
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_one_difference(stream, "/events/0", "SEQUENCE")

    def test_bool_is_not_an_integer(self) -> None:
        response = self._chat()
        response["created"] = True
        result = self._single_result(
            self._request("req-bool", "openai_chat", response)
        )
        self._assert_one_difference(result, "/created", "TYPE_MISMATCH")

    def test_root_json_type_mismatches_remain_self_validating(self) -> None:
        for protocol in SUPPORTED_PROTOCOLS:
            for http_status in ("200", "400"):
                with self.subTest(protocol=protocol, http_status=http_status):
                    request = self._request(
                        f"req-root-{protocol}-{http_status}",
                        protocol,
                        raw_response="[]",
                        http_status=http_status,
                    )
                    report = analyze_protocol_conformance(
                        self._parsed({request["request_id"]: request})
                    )
                    self.assertEqual([], validate_protocol_conformance(report))
                    self._assert_one_difference(
                        report["results"][0],
                        "/responseBody",
                        "TYPE_MISMATCH",
                    )

    def test_difference_actual_does_not_echo_untrusted_enum_values(self) -> None:
        marker = "SECRET_MARKER_42"
        response = self._chat()
        response["choices"][0]["finish_reason"] = marker
        report = analyze_protocol_conformance(
            self._parsed(
                {
                    "req-bounded-actual": self._request(
                        "req-bounded-actual",
                        "openai_chat",
                        response,
                    )
                }
            )
        )
        self.assertEqual([], validate_protocol_conformance(report))
        self.assertNotIn(marker, json.dumps(report))
        self._assert_one_difference(
            report["results"][0],
            "/choices/0/finish_reason",
            "ENUM_MISMATCH",
        )

    def test_gemini_args_are_required_only_for_required_parameters(self) -> None:
        response = self._gemini()
        response["candidates"][0]["content"]["parts"] = [
            {"functionCall": {"name": "optional_lookup"}}
        ]
        request_body = {
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "optional_lookup",
                            "parameters": {
                                "type": "OBJECT",
                                "properties": {"query": {"type": "STRING"}},
                            },
                        }
                    ]
                }
            ]
        }
        result = self._single_result(
            self._request(
                "req-gemini-optional-args",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self.assertEqual("CONSISTENT", result["status"])

        request_body["tools"][0]["functionDeclarations"][0]["parameters"][
            "required"
        ] = ["query"]
        result = self._single_result(
            self._request(
                "req-gemini-required-args",
                "gemini_generate_content",
                response,
                request_body=request_body,
            )
        )
        self._assert_one_difference(
            result,
            "/candidates/0/content/parts/0/functionCall/args",
            "MISSING_FIELD",
        )

    def test_gemini_missing_args_need_valid_request_evidence(self) -> None:
        for request_body in ("{bad", "[]"):
            with self.subTest(request_body=request_body):
                response = self._gemini()
                response["candidates"][0]["content"]["parts"] = [
                    {"functionCall": {"name": "get_weather"}}
                ]
                result = self._single_result(
                    self._request(
                        f"req-gemini-invalid-request-{len(request_body)}",
                        "gemini_generate_content",
                        response,
                        request_body=request_body,
                    )
                )
                self._assert_one_difference(
                    result,
                    "/requestBody",
                    "EVIDENCE_GAP",
                )

    def test_request_identity_mismatch_is_reported(self) -> None:
        request = self._request("inner-id", "openai_chat", self._chat())
        report = analyze_protocol_conformance(
            self._parsed({"map-key": request})
        )
        self.assertEqual([], validate_protocol_conformance(report))
        self._assert_one_difference(
            report["results"][0],
            "/request_id",
            "CORRELATION",
        )

    def test_validator_rejects_unpinned_difference_reference(self) -> None:
        response = self._chat()
        response["created"] = "one"
        report = analyze_protocol_conformance(
            self._parsed(
                {"req-reference": self._request("req-reference", "openai_chat", response)}
            )
        )
        report["results"][0]["differences"][0]["officialReference"] = (
            "https://example.invalid/not-the-pinned-reference"
        )
        errors = validate_protocol_conformance(report)
        self.assertTrue(
            any("officialReference" in error for error in errors),
            errors,
        )

    def test_responses_nested_collections_validate_each_item(self) -> None:
        mutations = (
            ("tools", "/tools/0"),
            ("annotations", "/output/0/content/0/annotations/0"),
            ("logprobs", "/output/0/content/0/logprobs/0"),
        )
        for field, location in mutations:
            with self.subTest(field=field):
                response = self._responses()
                if field == "tools":
                    response["tools"] = ["not-an-object"]
                else:
                    response["output"][0]["content"][0][field] = [
                        "not-an-object"
                    ]
                result = self._single_result(
                    self._request(
                        f"req-responses-nested-{field}",
                        "openai_responses",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "TYPE_MISMATCH")

    def test_responses_nested_required_fields_report_without_crashing(self) -> None:
        fixtures = []

        missing_tool_name = self._responses()
        missing_tool_name["tools"] = [
            {
                "type": "function",
                "strict": True,
                "parameters": {},
            }
        ]
        fixtures.append((missing_tool_name, "/tools/0/name"))

        missing_logprob_token = self._responses()
        missing_logprob_token["output"][0]["content"][0]["logprobs"] = [
            {
                "logprob": -0.1,
                "top_logprobs": [],
            }
        ]
        fixtures.append(
            (
                missing_logprob_token,
                "/output/0/content/0/logprobs/0/token",
            )
        )

        missing_logprob_value = self._responses()
        missing_logprob_value["output"][0]["content"][0]["logprobs"] = [
            {
                "token": "o",
                "top_logprobs": [],
            }
        ]
        fixtures.append(
            (
                missing_logprob_value,
                "/output/0/content/0/logprobs/0/logprob",
            )
        )

        for index, (response, location) in enumerate(fixtures):
            with self.subTest(location=location):
                result = self._single_result(
                    self._request(
                        f"req-responses-missing-nested-{index}",
                        "openai_responses",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "MISSING_FIELD")

    def test_responses_logprob_optional_fields_match_frozen_schema(self) -> None:
        response = self._responses()
        response["output"][0]["content"][0]["logprobs"] = [
            {"token": "o", "logprob": -0.1},
            {
                "token": "k",
                "logprob": -0.2,
                "top_logprobs": [{}, {"token": "x"}, {"logprob": -1.0}],
            },
        ]
        result = self._single_result(
            self._request("req-responses-optional-logprobs", "openai_responses", response)
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_responses_logprob_rejects_undocumented_bytes_fields(self) -> None:
        response = self._responses()
        response["output"][0]["content"][0]["logprobs"] = [
            {
                "token": "o",
                "logprob": -0.1,
                "bytes": [111],
                "top_logprobs": [{"bytes": [120]}],
            }
        ]
        result = self._single_result(
            self._request("req-responses-logprob-bytes", "openai_responses", response)
        )
        self._assert_difference(
            result,
            "/output/0/content/0/logprobs/0/bytes",
            "UNEXPECTED_FIELD",
        )
        self._assert_difference(
            result,
            "/output/0/content/0/logprobs/0/top_logprobs/0/bytes",
            "UNEXPECTED_FIELD",
        )

    def test_anthropic_citations_validate_each_item(self) -> None:
        response = self._anthropic()
        response["content"][0]["citations"] = ["not-an-object"]
        result = self._single_result(
            self._request("req-anthropic-citation", "anthropic_messages", response)
        )
        self._assert_one_difference(
            result,
            "/content/0/citations/0",
            "TYPE_MISMATCH",
        )

    def test_anthropic_missing_block_fields_report_without_crashing(self) -> None:
        fixtures = (
            ({"type": "text"}, "text"),
            ({"type": "thinking", "signature": "sig"}, "thinking"),
            ({"type": "thinking", "thinking": "trace"}, "signature"),
            ({"type": "redacted_thinking"}, "data"),
            (
                {"type": "tool_use", "name": "tool", "input": {}},
                "id",
            ),
            (
                {"type": "tool_use", "id": "toolu_1", "input": {}},
                "name",
            ),
            (
                {"type": "tool_use", "id": "toolu_1", "name": "tool"},
                "input",
            ),
        )
        for index, (block, missing_field) in enumerate(fixtures):
            with self.subTest(block_type=block["type"], missing=missing_field):
                response = self._anthropic()
                response["content"] = [block]
                if block["type"] == "tool_use":
                    response["stop_reason"] = "tool_use"
                result = self._single_result(
                    self._request(
                        f"req-anthropic-missing-block-{index}",
                        "anthropic_messages",
                        response,
                    )
                )
                self._assert_one_difference(
                    result,
                    f"/content/0/{missing_field}",
                    "MISSING_FIELD",
                )

    def test_gemini_nested_objects_validate_their_fields(self) -> None:
        mutations = (
            (
                "citationMetadata",
                {"citationSources": "not-an-array"},
                "/candidates/0/citationMetadata/citationSources",
            ),
            (
                "executableCode",
                {"language": "PYTHON", "code": 1},
                "/candidates/0/content/parts/0/executableCode/code",
            ),
        )
        for field, value, location in mutations:
            with self.subTest(field=field):
                response = self._gemini()
                if field == "citationMetadata":
                    response["candidates"][0][field] = value
                else:
                    response["candidates"][0]["content"]["parts"] = [
                        {field: value}
                    ]
                result = self._single_result(
                    self._request(
                        f"req-gemini-nested-{field}",
                        "gemini_generate_content",
                        response,
                    )
                )
                self._assert_one_difference(result, location, "TYPE_MISMATCH")

    def test_invalid_stream_evidence_cannot_pass_as_nonstream(self) -> None:
        invalid_values = (None, "", "wat", [], {})
        for index, stream_value in enumerate(invalid_values):
            with self.subTest(stream=stream_value):
                request = self._request(
                    f"req-invalid-stream-{index}",
                    "openai_chat",
                    self._chat(),
                )
                if stream_value is None:
                    request.pop("stream")
                else:
                    request["stream"] = stream_value
                result = self._single_result(request)
                self.assertFalse(result["stream"])
                self._assert_one_difference(result, "/stream", "EVIDENCE_GAP")

    def test_each_stream_success_profile_is_consistent(self) -> None:
        for protocol, (body, headers) in self._valid_stream_fixtures().items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-stream-{protocol}",
                        protocol,
                        raw_response=body,
                        stream="1",
                        response_headers=headers,
                    )
                )
                self.assertTrue(result["stream"])
                self.assertEqual("CONSISTENT", result["status"])
                self.assertEqual([], result["differences"])

    def test_stream_request_http_errors_use_json_error_envelopes(self) -> None:
        fixtures = {
            "openai_chat": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "openai_responses": {
                "error": {
                    "type": "invalid_request_error",
                    "message": "invalid request",
                    "param": None,
                    "code": None,
                }
            },
            "anthropic_messages": {
                "type": "error",
                "error": {"type": "invalid_request_error", "message": "bad"},
                "request_id": "req_fixture",
            },
            "gemini_generate_content": {
                "error": {
                    "code": 400,
                    "message": "bad",
                    "status": "INVALID_ARGUMENT",
                }
            },
            "ollama_chat": {"error": "bad request"},
        }
        for protocol, response in fixtures.items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-stream-http-error-{protocol}",
                        protocol,
                        response,
                        http_status="400",
                        stream="1",
                        response_headers="content-type: application/json",
                    )
                )
                self.assertEqual("CONSISTENT", result["status"])
                self.assertEqual([], result["differences"])

    def test_stream_content_type_when_present_matches_protocol(self) -> None:
        wrong_headers = {
            "openai_chat": "content-type: application/json",
            "openai_responses": "content-type: application/json",
            "anthropic_messages": "content-type: application/json",
            "gemini_generate_content": "content-type: application/json",
            "ollama_chat": "content-type: text/event-stream",
        }
        for protocol, (body, _) in self._valid_stream_fixtures().items():
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-stream-content-type-{protocol}",
                        protocol,
                        raw_response=body,
                        stream="1",
                        response_headers=wrong_headers[protocol],
                    )
                )
                self._assert_difference(
                    result,
                    "/responseHeaders/content-type",
                    "VALUE_MISMATCH",
                )

    def test_stream_framing_is_protocol_specific(self) -> None:
        chat = self._chat_stream_events()
        chat[0] = ("chat.completion.chunk", chat[0][1])

        responses = self._responses_stream_events()
        responses[0] = (None, responses[0][1])

        anthropic = self._anthropic_stream_events()
        anthropic[0] = (None, anthropic[0][1])

        gemini_named = self._gemini_stream_events()
        gemini_named[0] = ("message", gemini_named[0][1])

        gemini_done = self._gemini_stream_events()
        gemini_done.append((None, "[DONE]"))

        ollama_sse = [
            (None, record) for record in self._ollama_stream_records()
        ]

        fixtures = (
            (
                "openai_chat",
                self._encode_sse(chat),
                "/events/0/event",
            ),
            (
                "openai_responses",
                self._encode_sse(responses),
                "/events/0/event",
            ),
            (
                "anthropic_messages",
                self._encode_sse(anthropic),
                "/events/0/event",
            ),
            (
                "gemini_generate_content",
                self._encode_sse(gemini_named, trailing_blank=False),
                "/events/0/event",
            ),
            (
                "gemini_generate_content",
                self._encode_sse(gemini_done),
                "/events/2/data",
            ),
            (
                "ollama_chat",
                self._encode_sse(ollama_sse),
                "/records/0",
            ),
        )
        for index, (protocol, body, location) in enumerate(fixtures):
            with self.subTest(protocol=protocol, index=index):
                headers = (
                    "content-type: application/x-ndjson"
                    if protocol == "ollama_chat"
                    else "content-type: text/event-stream"
                )
                result = self._single_result(
                    self._request(
                        f"req-stream-framing-{protocol}-{index}",
                        protocol,
                        raw_response=body,
                        stream="1",
                        response_headers=headers,
                    )
                )
                self._assert_difference(result, location, "FRAMING")

    def test_stream_payload_invalid_json_is_precise_and_not_echoed(self) -> None:
        fixtures = (
            (
                "openai_chat",
                self._encode_sse([(None, '{"secret":"SSE_STREAM_SECRET"')]),
                "content-type: text/event-stream",
                "/events/0/data",
                "SSE_STREAM_SECRET",
            ),
            (
                "ollama_chat",
                self._encode_ndjson(['{"secret":"NDJSON_STREAM_SECRET"']),
                "content-type: application/x-ndjson",
                "/records/0",
                "NDJSON_STREAM_SECRET",
            ),
        )
        for protocol, body, headers, location, marker in fixtures:
            with self.subTest(protocol=protocol):
                result = self._single_result(
                    self._request(
                        f"req-stream-invalid-json-{protocol}",
                        protocol,
                        raw_response=body,
                        stream="1",
                        response_headers=headers,
                    )
                )
                self._assert_difference(result, location, "INVALID_JSON")
                self.assertNotIn(marker, json.dumps(result))

    def test_chat_stream_terminal_and_cross_chunk_correlation(self) -> None:
        missing_done = self._chat_stream_events()[:-1]

        changed_id = self._chat_stream_events()
        changed_id[1][1]["id"] = "chatcmpl-other"

        changed_created = self._chat_stream_events()
        changed_created[1][1]["created"] = 2

        changed_model = self._chat_stream_events()
        changed_model[1][1]["model"] = "gpt-other"

        duplicate_choice = self._chat_stream_events()
        duplicate_choice[1][1]["choices"].append(
            copy.deepcopy(duplicate_choice[1][1]["choices"][0])
        )

        fixtures = (
            (missing_done, "/events/2", "SEQUENCE"),
            (changed_id, "/events/1/data/id", "CORRELATION"),
            (changed_created, "/events/1/data/created", "CORRELATION"),
            (changed_model, "/events/1/data/model", "CORRELATION"),
            (
                duplicate_choice,
                "/events/1/data/choices/1/index",
                "CORRELATION",
            ),
        )
        for index, (events, location, kind) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-chat-stream-correlation-{index}",
                        "openai_chat",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, kind)

    def test_chat_stream_choice_indexes_follow_request_n_across_chunks(self) -> None:
        two_choice_events = self._chat_stream_events()
        for event_index in (0, 1):
            second_choice = copy.deepcopy(
                two_choice_events[event_index][1]["choices"][0]
            )
            second_choice["index"] = 1
            two_choice_events[event_index][1]["choices"].append(second_choice)
        two_choice_result = self._single_result(
            self._request(
                "req-chat-stream-two-choices",
                "openai_chat",
                raw_response=self._encode_sse(two_choice_events),
                request_body={"model": "fixture", "n": 2},
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", two_choice_result["status"])

        fixtures = []
        default_n_events = self._chat_stream_events()
        default_n_events[1][1]["choices"][0]["index"] = 1
        fixtures.append((default_n_events, {"model": "fixture"}))

        out_of_range_events = self._chat_stream_events()
        out_of_range_events[1][1]["choices"][0]["index"] = 2
        fixtures.append((out_of_range_events, {"model": "fixture", "n": 2}))

        for case_index, (events, request_body) in enumerate(fixtures):
            with self.subTest(case=case_index):
                result = self._single_result(
                    self._request(
                        f"req-chat-stream-choice-range-{case_index}",
                        "openai_chat",
                        raw_response=self._encode_sse(events),
                        request_body=request_body,
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(
                    result,
                    f"/events/1/data/choices/0/index",
                    "CORRELATION",
                )

    def test_chat_stream_done_marker_requires_a_response_chunk(self) -> None:
        result = self._single_result(
            self._request(
                "req-chat-stream-done-only",
                "openai_chat",
                raw_response=self._encode_sse([(None, "[DONE]")]),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(result, "/events/0/data", "SEQUENCE")

    def test_responses_stream_event_sequence_lifecycle_and_terminal(self) -> None:
        event_mismatch = self._responses_stream_events()
        event_mismatch[4] = ("response.output_text.done", event_mismatch[4][1])

        sequence_repeated = self._responses_stream_events()
        sequence_repeated[4][1]["sequence_number"] = 3

        item_mismatch = self._responses_stream_events()
        item_mismatch[4][1]["item_id"] = "msg_other"

        output_index_mismatch = self._responses_stream_events()
        output_index_mismatch[4][1]["output_index"] = 1

        content_index_mismatch = self._responses_stream_events()
        content_index_mismatch[5][1]["content_index"] = 1

        open_item_at_terminal = self._responses_stream_events()
        del open_item_at_terminal[7]

        missing_terminal = self._responses_stream_events()[:-1]

        terminal_identity_mismatch = self._responses_stream_events()
        terminal_identity_mismatch[8][1]["response"]["id"] = "resp_other"

        fixtures = (
            (event_mismatch, "/events/4/event", "VALUE_MISMATCH"),
            (
                sequence_repeated,
                "/events/4/data/sequence_number",
                "SEQUENCE",
            ),
            (item_mismatch, "/events/4/data/item_id", "CORRELATION"),
            (
                output_index_mismatch,
                "/events/4/data/output_index",
                "CORRELATION",
            ),
            (
                content_index_mismatch,
                "/events/5/data/content_index",
                "CORRELATION",
            ),
            (open_item_at_terminal, "/events/7/data/type", "SEQUENCE"),
            (missing_terminal, "/events/8", "SEQUENCE"),
            (
                terminal_identity_mismatch,
                "/events/8/data/response/id",
                "CORRELATION",
            ),
        )
        for index, (events, location, kind) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-responses-stream-lifecycle-{index}",
                        "openai_responses",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, kind)

        error_result = self._single_result(
            self._request(
                "req-responses-stream-error",
                "openai_responses",
                raw_response=self._encode_sse(
                    [
                        (
                            "error",
                            {
                                "type": "error",
                                "code": "server_error",
                                "message": "generation failed",
                                "param": None,
                                "sequence_number": 0,
                            },
                        )
                    ]
                ),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", error_result["status"])

    def test_responses_stream_function_call_events_follow_official_shape(self) -> None:
        result = self._single_result(
            self._request(
                "req-responses-stream-function-call",
                "openai_responses",
                raw_response=self._encode_sse(
                    self._responses_function_stream_events()
                ),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_responses_stream_accepts_output_text_annotation_event(self) -> None:
        events = self._responses_stream_events()
        events.insert(
            5,
            (
                "response.output_text.annotation.added",
                {
                    "type": "response.output_text.annotation.added",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "annotation_index": 0,
                    "annotation": {
                        "type": "file_citation",
                        "file_id": "file_fixture",
                        "index": 0,
                        "filename": "fixture.txt",
                    },
                    "sequence_number": 5,
                },
            ),
        )
        for index, (_, payload) in enumerate(events):
            payload["sequence_number"] = index
        result = self._single_result(
            self._request(
                "req-responses-stream-output-text-annotation",
                "openai_responses",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_responses_stream_sequence_numbers_are_strictly_increasing(self) -> None:
        monotonic = self._responses_stream_events()
        for index, (_, payload) in enumerate(monotonic):
            payload["sequence_number"] = 10 + index * 2
        monotonic_result = self._single_result(
            self._request(
                "req-responses-stream-monotonic-sequence",
                "openai_responses",
                raw_response=self._encode_sse(monotonic),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual(
            "CONSISTENT",
            monotonic_result["status"],
            monotonic_result["differences"],
        )

        for name, value in (("repeat", 3), ("decrease", 2)):
            with self.subTest(case=name):
                events = self._responses_stream_events()
                events[4][1]["sequence_number"] = value
                result = self._single_result(
                    self._request(
                        f"req-responses-stream-sequence-{name}",
                        "openai_responses",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(
                    result,
                    "/events/4/data/sequence_number",
                    "SEQUENCE",
                )

    def test_responses_stream_function_item_fields_correlate(self) -> None:
        fixtures = []

        changed_item_id = self._responses_function_stream_events()
        changed_item_id[3][1]["item_id"] = "fc_other"
        fixtures.append((changed_item_id, "/events/3/data/item_id"))

        changed_output_index = self._responses_function_stream_events()
        changed_output_index[3][1]["output_index"] = 1
        fixtures.append((changed_output_index, "/events/3/data/output_index"))

        changed_call_id = self._responses_function_stream_events()
        changed_call_id[5][1]["item"]["call_id"] = "call_other"
        fixtures.append((changed_call_id, "/events/5/data/item/call_id"))

        changed_final_call_id = self._responses_function_stream_events()
        changed_final_call_id[6][1]["response"]["output"][0][
            "call_id"
        ] = "call_other"
        fixtures.append(
            (
                changed_final_call_id,
                "/events/6/data/response/output/0/call_id",
            )
        )

        for index, (events, location) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-responses-stream-function-correlation-{index}",
                        "openai_responses",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, "CORRELATION")

    def test_responses_audio_events_require_official_response_id_fields(self) -> None:
        audio_events = (
            (
                "response.audio.delta",
                {"type": "response.audio.delta", "delta": "AA=="},
            ),
            (
                "response.audio.done",
                {"type": "response.audio.done", "response_id": "resp_fixture"},
            ),
            (
                "response.audio.transcript.delta",
                {
                    "type": "response.audio.transcript.delta",
                    "response_id": "resp_fixture",
                    "delta": "ok",
                },
            ),
            (
                "response.audio.transcript.done",
                {
                    "type": "response.audio.transcript.done",
                    "response_id": "resp_fixture",
                },
            ),
        )
        events = self._responses_stream_events()
        for offset, audio_event in enumerate(audio_events):
            events.insert(2 + offset, copy.deepcopy(audio_event))
        for sequence_number, (_, payload) in enumerate(events):
            payload["sequence_number"] = sequence_number
        result = self._single_result(
            self._request(
                "req-responses-audio-events",
                "openai_responses",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        for event_name in (
            "response.audio.done",
            "response.audio.transcript.delta",
            "response.audio.transcript.done",
        ):
            with self.subTest(event=event_name):
                missing = copy.deepcopy(events)
                event_index = next(
                    index for index, event in enumerate(missing) if event[0] == event_name
                )
                missing[event_index][1].pop("response_id")
                invalid = self._single_result(
                    self._request(
                        f"req-responses-audio-missing-{event_index}",
                        "openai_responses",
                        raw_response=self._encode_sse(missing),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(
                    invalid,
                    f"/events/{event_index}/data/response_id",
                    "MISSING_FIELD",
                )

    def test_responses_custom_tool_call_terminal_and_event_correlation(self) -> None:
        started = {
            "type": "custom_tool_call",
            "id": "ctc_fixture",
            "call_id": "call_fixture",
            "name": "run_fixture",
            "input": "",
        }
        completed = {**started, "input": "fixture input"}
        middle = [
            (
                "response.custom_tool_call_input.delta",
                {
                    "type": "response.custom_tool_call_input.delta",
                    "item_id": "ctc_fixture",
                    "output_index": 0,
                    "delta": "fixture ",
                },
            ),
            (
                "response.custom_tool_call_input.done",
                {
                    "type": "response.custom_tool_call_input.done",
                    "item_id": "ctc_fixture",
                    "output_index": 0,
                    "input": "fixture input",
                },
            ),
        ]
        events = self._responses_item_stream_events(started, completed, middle)
        result = self._single_result(
            self._request(
                "req-responses-custom-tool",
                "openai_responses",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        missing_input = copy.deepcopy(events)
        missing_input[2][1]["item"].pop("input")
        invalid_item = self._single_result(
            self._request(
                "req-responses-custom-tool-missing-input",
                "openai_responses",
                raw_response=self._encode_sse(missing_input),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_item,
            "/events/2/data/item/input",
            "MISSING_FIELD",
        )

        reasoning_started = {
            "id": "rs_fixture",
            "type": "reasoning",
            "summary": [],
            "status": "in_progress",
        }
        reasoning_completed = {**reasoning_started, "status": "completed"}
        wrong_reference = self._responses_item_stream_events(
            reasoning_started,
            reasoning_completed,
            [
                (
                    "response.custom_tool_call_input.delta",
                    {
                        "type": "response.custom_tool_call_input.delta",
                        "item_id": "rs_fixture",
                        "output_index": 0,
                        "delta": "wrong",
                    },
                )
            ],
        )
        invalid_reference = self._single_result(
            self._request(
                "req-responses-custom-tool-wrong-item",
                "openai_responses",
                raw_response=self._encode_sse(wrong_reference),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_reference,
            "/events/3/data/item_id",
            "CORRELATION",
        )

    def test_responses_reasoning_events_reference_reasoning_items(self) -> None:
        reasoning_started = {
            "id": "rs_fixture",
            "type": "reasoning",
            "summary": [],
            "status": "in_progress",
        }
        reasoning_completed = {
            **reasoning_started,
            "summary": [{"type": "summary_text", "text": "ok"}],
            "status": "completed",
        }
        middle = [
            (
                "response.reasoning_summary_text.delta",
                {
                    "type": "response.reasoning_summary_text.delta",
                    "item_id": "rs_fixture",
                    "output_index": 0,
                    "summary_index": 0,
                    "delta": "ok",
                },
            ),
            (
                "response.reasoning_summary_text.done",
                {
                    "type": "response.reasoning_summary_text.done",
                    "item_id": "rs_fixture",
                    "output_index": 0,
                    "summary_index": 0,
                    "text": "ok",
                },
            ),
        ]
        legal = self._responses_item_stream_events(
            reasoning_started, reasoning_completed, middle
        )
        result = self._single_result(
            self._request(
                "req-responses-reasoning-events",
                "openai_responses",
                raw_response=self._encode_sse(legal),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        custom_item = {
            "type": "custom_tool_call",
            "id": "ctc_fixture",
            "call_id": "call_fixture",
            "name": "fixture",
            "input": "ok",
        }
        wrong = self._responses_item_stream_events(custom_item, custom_item, middle)
        for _, payload in wrong[3:-2]:
            payload["item_id"] = "ctc_fixture"
        invalid = self._single_result(
            self._request(
                "req-responses-reasoning-wrong-item",
                "openai_responses",
                raw_response=self._encode_sse(wrong),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid,
            "/events/3/data/item_id",
            "CORRELATION",
        )

    def test_responses_shell_call_shape_and_event_correlation(self) -> None:
        started = {
            "type": "shell_call",
            "id": "sh_fixture",
            "call_id": "call_fixture",
            "action": {
                "commands": ["echo ok"],
                "timeout_ms": None,
                "max_output_length": None,
            },
            "status": "in_progress",
            "environment": {"type": "local"},
        }
        completed = {**started, "status": "completed"}
        middle = [
            (
                "response.shell_call_command.added",
                {
                    "type": "response.shell_call_command.added",
                    "output_index": 0,
                    "command_index": 0,
                    "command": "",
                },
            ),
            (
                "response.shell_call_command.delta",
                {
                    "type": "response.shell_call_command.delta",
                    "output_index": 0,
                    "command_index": 0,
                    "delta": "echo ok",
                },
            ),
            (
                "response.shell_call_command.done",
                {
                    "type": "response.shell_call_command.done",
                    "output_index": 0,
                    "command_index": 0,
                    "command": "echo ok",
                },
            ),
            (
                "response.shell_call_output_content.delta",
                {
                    "type": "response.shell_call_output_content.delta",
                    "item_id": "sh_fixture",
                    "output_index": 0,
                    "command_index": 0,
                    "delta": {"stdout": "ok\n"},
                },
            ),
            (
                "response.shell_call_output_content.done",
                {
                    "type": "response.shell_call_output_content.done",
                    "item_id": "sh_fixture",
                    "output_index": 0,
                    "command_index": 0,
                    "output": [
                        {
                            "stdout": "ok\n",
                            "stderr": "",
                            "outcome": {"type": "exit", "exit_code": 0},
                        }
                    ],
                },
            ),
        ]
        events = self._responses_item_stream_events(started, completed, middle)
        result = self._single_result(
            self._request(
                "req-responses-shell-call",
                "openai_responses",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        missing_action_field = copy.deepcopy(events)
        missing_action_field[2][1]["item"]["action"].pop("max_output_length")
        invalid_shape = self._single_result(
            self._request(
                "req-responses-shell-missing-action-field",
                "openai_responses",
                raw_response=self._encode_sse(missing_action_field),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_shape,
            "/events/2/data/item/action/max_output_length",
            "MISSING_FIELD",
        )

        custom_item = {
            "type": "custom_tool_call",
            "id": "ctc_fixture",
            "call_id": "call_fixture",
            "name": "fixture",
            "input": "ok",
        }
        wrong = self._responses_item_stream_events(custom_item, custom_item, middle)
        for _, payload in wrong[3:-2]:
            if "item_id" in payload:
                payload["item_id"] = "ctc_fixture"
        invalid_reference = self._single_result(
            self._request(
                "req-responses-shell-wrong-item",
                "openai_responses",
                raw_response=self._encode_sse(wrong),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_reference,
            "/events/3/data/output_index",
            "CORRELATION",
        )
        self._assert_difference(
            invalid_reference,
            "/events/6/data/item_id",
            "CORRELATION",
        )

    def test_responses_annotation_is_nullable_and_targets_output_text(self) -> None:
        events = self._responses_stream_events()
        events.insert(
            5,
            (
                "response.output_text.annotation.added",
                {
                    "type": "response.output_text.annotation.added",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "annotation_index": 0,
                    "annotation": None,
                },
            ),
        )
        for sequence_number, (_, payload) in enumerate(events):
            payload["sequence_number"] = sequence_number
        result = self._single_result(
            self._request(
                "req-responses-null-annotation",
                "openai_responses",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

        missing = copy.deepcopy(events)
        missing[5][1].pop("annotation")
        invalid_missing = self._single_result(
            self._request(
                "req-responses-missing-annotation",
                "openai_responses",
                raw_response=self._encode_sse(missing),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_missing,
            "/events/5/data/annotation",
            "MISSING_FIELD",
        )

        refusal = self._responses_stream_events()
        refusal[3][1]["part"] = {"type": "refusal", "refusal": ""}
        del refusal[4:6]
        refusal.insert(
            4,
            (
                "response.output_text.annotation.added",
                {
                    "type": "response.output_text.annotation.added",
                    "item_id": "msg_fixture",
                    "output_index": 0,
                    "content_index": 0,
                    "annotation_index": 0,
                    "annotation": None,
                },
            ),
        )
        refusal[5][1]["part"] = {"type": "refusal", "refusal": "no"}
        refusal_item = copy.deepcopy(refusal[6][1]["item"])
        refusal_item["content"] = [{"type": "refusal", "refusal": "no"}]
        refusal[6][1]["item"] = refusal_item
        refusal[7][1]["response"]["output"] = [copy.deepcopy(refusal_item)]
        for sequence_number, (_, payload) in enumerate(refusal):
            payload["sequence_number"] = sequence_number
        invalid_part = self._single_result(
            self._request(
                "req-responses-annotation-refusal",
                "openai_responses",
                raw_response=self._encode_sse(refusal),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            invalid_part,
            "/events/4/data/content_index",
            "CORRELATION",
        )

    def test_responses_stream_terminal_event_matches_response_state(self) -> None:
        completed_with_failed_status = self._responses_stream_events()
        completed_with_failed_status[-1][1]["response"]["status"] = "failed"

        failed_without_error = self._responses_stream_events()
        failed_without_error[-1] = (
            "response.failed",
            {
                **failed_without_error[-1][1],
                "type": "response.failed",
            },
        )
        failed_without_error[-1][1]["response"]["status"] = "failed"

        fixtures = (
            (
                completed_with_failed_status,
                "/events/8/data/response/status",
            ),
            (failed_without_error, "/events/8/data/response/error"),
        )
        for index, (events, location) in enumerate(fixtures):
            with self.subTest(index=index):
                result = self._single_result(
                    self._request(
                        f"req-responses-stream-terminal-state-{index}",
                        "openai_responses",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, "CORRELATION")

    def test_anthropic_stream_block_tool_and_message_lifecycle(self) -> None:
        event_mismatch = self._anthropic_stream_events()
        event_mismatch[2] = ("content_block_stop", event_mismatch[2][1])

        block_index_mismatch = self._anthropic_stream_events()
        block_index_mismatch[4][1]["index"] = 1

        invalid_tool_json = self._anthropic_stream_events()
        invalid_tool_json[3][1]["delta"]["partial_json"] = '"Beijing"'

        stop_reason_mismatch = self._anthropic_stream_events()
        stop_reason_mismatch[5][1]["delta"]["stop_reason"] = "end_turn"

        missing_message_delta = self._anthropic_stream_events()
        del missing_message_delta[5]

        missing_message_stop = self._anthropic_stream_events()[:-1]

        fixtures = (
            (event_mismatch, "/events/2/event", "VALUE_MISMATCH"),
            (block_index_mismatch, "/events/4/data/index", "CORRELATION"),
            (invalid_tool_json, "/events/4/data", "INVALID_JSON"),
            (
                stop_reason_mismatch,
                "/events/5/data/delta/stop_reason",
                "CORRELATION",
            ),
            (missing_message_delta, "/events/5/data/type", "SEQUENCE"),
            (missing_message_stop, "/events/6", "SEQUENCE"),
        )
        for index, (events, location, kind) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-anthropic-stream-lifecycle-{index}",
                        "anthropic_messages",
                        raw_response=self._encode_sse(events),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, kind)

        error_result = self._single_result(
            self._request(
                "req-anthropic-stream-error",
                "anthropic_messages",
                raw_response=self._encode_sse(
                    [
                        (
                            "error",
                            {
                                "type": "error",
                                "error": {
                                    "type": "overloaded_error",
                                    "message": "overloaded",
                                },
                            },
                        )
                    ]
                ),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", error_result["status"])

    def test_anthropic_stream_content_blocks_precede_message_delta(self) -> None:
        events = self._anthropic_stream_events()
        message_delta = events.pop(5)
        events.insert(1, message_delta)
        result = self._single_result(
            self._request(
                "req-anthropic-stream-block-after-message-delta",
                "anthropic_messages",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            result,
            "/events/2/data/type",
            "SEQUENCE",
        )

    def test_anthropic_stream_allows_multiple_partial_message_deltas(self) -> None:
        events = self._anthropic_stream_events()
        events.insert(
            5,
            (
                "message_delta",
                {
                    "type": "message_delta",
                    "delta": {},
                    "usage": {"output_tokens": 1},
                },
            ),
        )
        result = self._single_result(
            self._request(
                "req-anthropic-partial-message-deltas",
                "anthropic_messages",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", result["status"], result["differences"])

    def test_anthropic_stream_delta_type_matches_open_block(self) -> None:
        events = self._anthropic_stream_events()
        events[2][1]["delta"] = {"type": "text_delta", "text": "wrong"}
        del events[3]
        result = self._single_result(
            self._request(
                "req-anthropic-stream-tool-text-delta",
                "anthropic_messages",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            result,
            "/events/2/data/delta/type",
            "CORRELATION",
        )

    def test_anthropic_stream_block_indexes_are_contiguous(self) -> None:
        events = self._anthropic_stream_events()
        for event_index in (1, 2, 3, 4):
            events[event_index][1]["index"] = 1
        result = self._single_result(
            self._request(
                "req-anthropic-stream-noncontiguous-block",
                "anthropic_messages",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            result,
            "/events/1/data/index",
            "SEQUENCE",
        )

    def test_anthropic_stream_reports_each_extra_field_once(self) -> None:
        events = self._anthropic_stream_events()
        events[-1][1]["extra"] = True
        result = self._single_result(
            self._request(
                "req-anthropic-stream-extra-field",
                "anthropic_messages",
                raw_response=self._encode_sse(events),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        matching = [
            difference
            for difference in result["differences"]
            if difference["location"] == "/events/6/data/extra"
            and difference["differenceKind"] == "UNEXPECTED_FIELD"
        ]
        self.assertEqual(1, len(matching), result["differences"])

    def test_gemini_stream_eof_and_declared_tool_arguments(self) -> None:
        final_event = self._gemini_stream_events()[-1:]
        eof_result = self._single_result(
            self._request(
                "req-gemini-stream-eof",
                "gemini_generate_content",
                raw_response=self._encode_sse(
                    final_event,
                    trailing_blank=False,
                ),
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self.assertEqual("CONSISTENT", eof_result["status"])

        response = self._gemini()
        response["candidates"][0]["content"]["parts"] = [
            {
                "functionCall": {
                    "name": "get_weather",
                    "args": '{"city":"Beijing"}',
                }
            }
        ]
        request_body = {
            "tools": [
                {
                    "functionDeclarations": [
                        {
                            "name": "get_weather",
                            "parameters": {
                                "type": "OBJECT",
                                "properties": {"city": {"type": "STRING"}},
                                "required": ["city"],
                            },
                        }
                    ]
                }
            ]
        }
        args_result = self._single_result(
            self._request(
                "req-gemini-stream-tool-args",
                "gemini_generate_content",
                raw_response=self._encode_sse(
                    [(None, response)],
                    trailing_blank=False,
                ),
                request_body=request_body,
                stream="1",
                response_headers="content-type: text/event-stream",
            )
        )
        self._assert_difference(
            args_result,
            "/events/0/data/candidates/0/content/parts/0/functionCall/args",
            "TYPE_MISMATCH",
        )

    def test_gemini_stream_identity_is_stable_across_frames(self) -> None:
        fixtures = []

        changed_response_id = self._gemini_stream_events()
        changed_response_id[1][1]["responseId"] = "resp-other"
        fixtures.append((changed_response_id, "/events/1/data/responseId"))

        changed_model_version = self._gemini_stream_events()
        changed_model_version[1][1]["modelVersion"] = "gemini-other"
        fixtures.append((changed_model_version, "/events/1/data/modelVersion"))

        for index, (events, location) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-gemini-stream-identity-{index}",
                        "gemini_generate_content",
                        raw_response=self._encode_sse(
                            events,
                            trailing_blank=False,
                        ),
                        stream="1",
                        response_headers="content-type: text/event-stream",
                    )
                )
                self._assert_difference(result, location, "CORRELATION")

    def test_ollama_stream_terminal_usage_and_inband_error(self) -> None:
        missing_terminal = self._ollama_stream_records()[:1]

        duplicate_terminal = self._ollama_stream_records()
        duplicate_terminal.append(copy.deepcopy(duplicate_terminal[-1]))

        invalid_usage = self._ollama_stream_records()
        invalid_usage[1]["eval_count"] = "1"

        invalid_error = self._ollama_stream_records()[:1]
        invalid_error.append({"error": {"message": "generation failed"}})

        fixtures = (
            (missing_terminal, "/records/1", "SEQUENCE"),
            (duplicate_terminal, "/records/2/done", "SEQUENCE"),
            (invalid_usage, "/records/1/eval_count", "TYPE_MISMATCH"),
            (invalid_error, "/records/1/error", "TYPE_MISMATCH"),
        )
        for index, (records, location, kind) in enumerate(fixtures):
            with self.subTest(index=index, location=location):
                result = self._single_result(
                    self._request(
                        f"req-ollama-stream-lifecycle-{index}",
                        "ollama_chat",
                        raw_response=self._encode_ndjson(records),
                        stream="1",
                        response_headers="content-type: application/x-ndjson",
                    )
                )
                self._assert_difference(result, location, kind)

        inband_error = self._ollama_stream_records()[:1]
        inband_error.append({"error": "generation failed"})
        error_result = self._single_result(
            self._request(
                "req-ollama-stream-error",
                "ollama_chat",
                raw_response=self._encode_ndjson(inband_error),
                stream="1",
                response_headers="content-type: application/x-ndjson",
            )
        )
        self.assertEqual("CONSISTENT", error_result["status"])

    def test_ollama_stream_final_record_requires_usage_fields(self) -> None:
        usage_fields = (
            "total_duration",
            "load_duration",
            "prompt_eval_count",
            "prompt_eval_duration",
            "eval_count",
            "eval_duration",
        )
        for field in usage_fields:
            with self.subTest(field=field):
                records = self._ollama_stream_records()
                del records[-1][field]
                result = self._single_result(
                    self._request(
                        f"req-ollama-stream-missing-final-{field}",
                        "ollama_chat",
                        raw_response=self._encode_ndjson(records),
                        stream="1",
                        response_headers="content-type: application/x-ndjson",
                    )
                )
                self._assert_difference(
                    result,
                    f"/records/1/{field}",
                    "MISSING_FIELD",
                )

    def test_ollama_stream_nonfinal_record_rejects_final_usage_fields(self) -> None:
        records = self._ollama_stream_records()
        records[0]["eval_count"] = 1
        result = self._single_result(
            self._request(
                "req-ollama-stream-nonfinal-usage",
                "ollama_chat",
                raw_response=self._encode_ndjson(records),
                stream="1",
                response_headers="content-type: application/x-ndjson",
            )
        )
        self._assert_difference(
            result,
            "/records/0/eval_count",
            "SEQUENCE",
        )

    def test_content_type_is_validated_only_when_present(self) -> None:
        result = self._single_result(
            self._request(
                "req-content-type",
                "openai_chat",
                self._chat(),
                response_headers="content-type: text/plain",
            )
        )
        self._assert_one_difference(
            result,
            "/responseHeaders/content-type",
            "VALUE_MISMATCH",
        )

    def test_all_requests_are_reported_once_with_check_associations(self) -> None:
        requests = {
            "req-z": self._request("req-z", "openai_chat", self._chat()),
            "req-unreferenced": self._request(
                "req-unreferenced", "custom_protocol", self._chat()
            ),
            "req-a": self._request("req-a", "openai_chat", self._chat()),
        }
        tests = {
            "010": {"requestRefs": ["req-z"]},
            "002": {"requestRefs": ["req-z", "req-a"]},
        }
        report = analyze_protocol_conformance(self._parsed(requests, tests))

        self.assertEqual(
            ["req-z", "req-unreferenced", "req-a"],
            [item["requestId"] for item in report["results"]],
        )
        self.assertEqual(["002", "010"], report["results"][0]["checkIds"])
        self.assertEqual([], report["results"][1]["checkIds"])
        self.assertEqual(["002"], report["results"][2]["checkIds"])
        self.assertEqual(
            {
                "totalRequests": 3,
                "checkedRequests": 3,
                "consistentRequests": 2,
                "differentRequests": 1,
            },
            {key: report["summary"][key] for key in (
                "totalRequests",
                "checkedRequests",
                "consistentRequests",
                "differentRequests",
            )},
        )
        self.assertEqual(
            ["custom_protocol", "openai_chat"],
            [item["protocol"] for item in report["summary"]["byProtocol"]],
        )
        self.assertEqual(
            ["002", "010"],
            [item["checkId"] for item in report["summary"]["byCheck"]],
        )
        self.assertEqual(
            [{"differenceKind": "EVIDENCE_GAP", "count": 1}],
            report["summary"]["byDifferenceKind"],
        )
        self.assertEqual([], validate_protocol_conformance(report))

    def test_validator_rejects_tampered_shape_counts_and_status(self) -> None:
        report = analyze_protocol_conformance(
            self._parsed(
                {"req-chat": self._request("req-chat", "openai_chat", self._chat())}
            )
        )
        mutations = []

        extra_field = copy.deepcopy(report)
        extra_field["extra"] = True
        mutations.append((extra_field, "field extra"))

        wrong_count = copy.deepcopy(report)
        wrong_count["summary"]["totalRequests"] = 2
        mutations.append((wrong_count, "totalRequests"))

        impossible_status = copy.deepcopy(report)
        impossible_status["results"][0]["status"] = "DIFFERENT"
        mutations.append((impossible_status, "differences"))

        wrong_group = copy.deepcopy(report)
        wrong_group["summary"]["byProtocol"][0]["consistentRequests"] = 0
        mutations.append((wrong_group, "byProtocol"))

        for value, expected_error in mutations:
            with self.subTest(expected_error=expected_error):
                errors = validate_protocol_conformance(value)
                self.assertTrue(errors)
                self.assertTrue(
                    any(expected_error in error for error in errors),
                    errors,
                )

    def test_analyzer_treats_empty_protocol_as_missing_metadata(self) -> None:
        report = analyze_protocol_conformance(
            self._parsed(
                {"req-empty-protocol": self._request("req-empty-protocol", "", self._chat())}
            )
        )
        self.assertEqual([], validate_protocol_conformance(report))
        result = report["results"][0]
        self.assertIsNone(result["protocol"])
        self._assert_one_difference(result, "/protocol", "EVIDENCE_GAP")
        self.assertIsNone(result["differences"][0]["protocol"])


if __name__ == "__main__":
    unittest.main()
