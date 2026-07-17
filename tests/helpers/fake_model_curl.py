#!/usr/bin/env python3
"""Minimal curl-compatible executable for Model Doctor CLI tests."""

import json
import os
import re
import sys
import time
from pathlib import Path


def argument_value(arguments, option, default=""):
    try:
        return arguments[arguments.index(option) + 1]
    except (ValueError, IndexError):
        return default


def chat(content, *, reasoning_tokens=None):
    usage = {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}
    if reasoning_tokens is not None:
        usage["completion_tokens_details"] = {"reasoning_tokens": reasoning_tokens}
    return json.dumps(
        {
            "id": "resp_fixture",
            "object": "chat.completion",
            "model": "fixture-model",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop",
                }
            ],
            "usage": usage,
        },
        separators=(",", ":"),
    )


def anthropic_message(content):
    return json.dumps(
        {
            "id": "msg_fixture",
            "type": "message",
            "role": "assistant",
            "model": "fixture-model",
            "content": [{"type": "text", "text": content}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 8, "output_tokens": 4},
        },
        separators=(",", ":"),
    )


def anthropic_tool_use():
    return json.dumps(
        {
            "id": "msg_fixture",
            "type": "message",
            "role": "assistant",
            "model": "fixture-model",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_fixture",
                    "name": "get_weather",
                    "input": {"city": "Beijing"},
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 8, "output_tokens": 4},
        },
        separators=(",", ":"),
    )


def tool_chain_probe(protocol):
    if protocol == "openai_chat":
        return chat("MODEL_DOCTOR_PROTOCOL_OK")
    if protocol == "openai_responses":
        return json.dumps(
            {
                "id": "resp_probe",
                "object": "response",
                "output": [{"type": "message", "content": []}],
                "output_text": "MODEL_DOCTOR_PROTOCOL_OK",
            },
            separators=(",", ":"),
        )
    if protocol == "anthropic_messages":
        return anthropic_message("MODEL_DOCTOR_PROTOCOL_OK")
    if protocol == "gemini_generate_content":
        return json.dumps(
            {
                "candidates": [
                    {
                        "content": {
                            "role": "model",
                            "parts": [{"text": "MODEL_DOCTOR_PROTOCOL_OK"}],
                        },
                        "finishReason": "STOP",
                    }
                ]
            },
            separators=(",", ":"),
        )
    return json.dumps(
        {
            "model": "fixture-model",
            "message": {"role": "assistant", "content": "MODEL_DOCTOR_PROTOCOL_OK"},
            "done": True,
            "done_reason": "stop",
        },
        separators=(",", ":"),
    )


def tool_chain_first(protocol, test_id):
    sentinel = f"OBSERVED_{protocol.upper()}_{test_id}"
    tool_name = "get_weather"
    if protocol == "openai_chat":
        message = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": f"call_{test_id}",
                    "type": "function",
                    "function": {
                        "name": tool_name,
                        "arguments": json.dumps(
                            {"city": "Beijing", "sentinel": sentinel},
                            separators=(",", ":"),
                        ),
                    },
                }
            ],
            "fixture_sentinel": sentinel,
        }
        payload = {
            "id": f"chatcmpl_{test_id}",
            "object": "chat.completion",
            "choices": [{"message": message, "finish_reason": "tool_calls"}],
        }
    elif protocol == "openai_responses":
        payload = {
            "id": f"resp_{test_id}_{sentinel}",
            "object": "response",
            "output": [
                {
                    "type": "function_call",
                    "id": f"fc_{test_id}",
                    "call_id": f"call_{test_id}_{sentinel}",
                    "name": tool_name,
                    "arguments": json.dumps({"city": "Beijing"}),
                }
            ],
            "output_text": "",
        }
    elif protocol == "anthropic_messages":
        payload = {
            "id": f"msg_{test_id}",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": f"toolu_{test_id}",
                    "name": tool_name,
                    "input": {"city": "Beijing", "sentinel": sentinel},
                    "fixture_sentinel": sentinel,
                }
            ],
            "stop_reason": "tool_use",
        }
    elif protocol == "gemini_generate_content":
        payload = {
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "functionCall": {
                                    "id": f"gem_call_{test_id}",
                                    "name": tool_name,
                                    "args": {"city": "Beijing", "sentinel": sentinel},
                                },
                                "fixture_sentinel": sentinel,
                            }
                        ],
                    },
                    "finishReason": "STOP",
                }
            ]
        }
    else:
        payload = {
            "model": "fixture-model",
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [
                    {
                        "function": {
                            "name": tool_name,
                            "arguments": {"city": "Beijing", "sentinel": sentinel},
                        }
                    }
                ],
                "fixture_sentinel": sentinel,
            },
            "done": True,
            "done_reason": "stop",
        }
    return json.dumps(payload, separators=(",", ":"))


def tool_chain_follow(protocol, test_id):
    marker = f"MODEL_DOCTOR_CASE_{test_id}_FOLLOW_OK"
    if protocol == "openai_responses":
        return json.dumps(
            {
                "id": f"resp_{test_id}_follow",
                "object": "response",
                "output": [],
                "output_text": marker,
            },
            separators=(",", ":"),
        )
    if protocol == "anthropic_messages":
        return anthropic_message(marker)
    if protocol == "gemini_generate_content":
        return json.dumps(
            {
                "candidates": [
                    {
                        "content": {"role": "model", "parts": [{"text": marker}]},
                        "finishReason": "STOP",
                    }
                ]
            },
            separators=(",", ":"),
        )
    if protocol == "ollama_chat":
        return json.dumps(
            {
                "model": "fixture-model",
                "message": {"role": "assistant", "content": marker},
                "done": True,
                "done_reason": "stop",
            },
            separators=(",", ":"),
        )
    return chat(marker)


def truncated_chat(content):
    return '{"choices":[{"message":{"content":' + json.dumps(content)


def invalid_balanced_chat(content):
    return '{"choices":[{"message":{"content":' + json.dumps(content) + "} garbage}]}"


def chat_stream(
    *,
    content="MODEL_DOCTOR_CASE_036_OK",
    include_reasoning=False,
    complete=True,
    finish_reason="stop",
):
    events = []
    if include_reasoning:
        events.append(
            {"choices": [{"delta": {"reasoning_content": "19 + 23 = 42"}, "finish_reason": None}]}
        )
    split_at = len(content) // 2
    events.extend(
        {"choices": [{"delta": {"content": part}, "finish_reason": None}]}
        for part in (content[:split_at], content[split_at:])
    )
    if complete and finish_reason is not None:
        events.append(
            {
                "choices": [{"delta": {}, "finish_reason": finish_reason}],
                "usage": {"completion_tokens_details": {"reasoning_tokens": 8}},
            }
        )
    chunks = [f"data: {json.dumps(event, separators=(',', ':'))}" for event in events]
    if complete:
        chunks.append("data: [DONE]")
    return "\n\n".join(chunks) + "\n\n"


arguments = sys.argv[1:]
if arguments == ["--version"]:
    print("curl fixture 1.0")
    raise SystemExit(0)

output_path = Path(argument_value(arguments, "--output"))
headers_path = Path(argument_value(arguments, "--dump-header"))
request_body = argument_value(arguments, "--data-binary")
scenario = os.environ.get("MODEL_DOCTOR_FAKE_SCENARIO", "basic")
delay = float(os.environ.get("MODEL_DOCTOR_FAKE_DELAY", "0"))
if delay:
    time.sleep(delay)
http_status = "200"
time_total = "0.020"

tool_chain_match = re.fullmatch(
    r"(openai_chat|openai_responses|anthropic_messages|gemini_generate_content|ollama_chat)_tool_chain",
    scenario,
)
if tool_chain_match:
    protocol = tool_chain_match.group(1)
    probe_indexes = {
        "openai_chat": 1,
        "openai_responses": 2,
        "anthropic_messages": 3,
        "gemini_generate_content": 4,
        "ollama_chat": 5,
    }
    probe_match = re.fullmatch(r"protocol-([0-9]+)", output_path.stem)
    first_match = re.fullmatch(r"test-(047|048|049)", output_path.stem)
    follow_match = re.fullmatch(r"test-(047|048|049)-follow", output_path.stem)
    if probe_match:
        if int(probe_match.group(1)) == probe_indexes[protocol]:
            response = tool_chain_probe(protocol)
        else:
            response = '{"fixture":"protocol-mismatch"}'
            http_status = "400"
    elif first_match:
        response = tool_chain_first(protocol, first_match.group(1))
    elif follow_match:
        response = tool_chain_follow(protocol, follow_match.group(1))
    else:
        response = tool_chain_probe(protocol)
elif scenario == "anthropic_output_budget":
    payload = json.loads(request_body)
    if "max_tokens" not in payload:
        response = '{"error":{"message":"max_tokens is required"}}'
        http_status = "400"
    elif "MODEL_DOCTOR_CASE_040" in request_body:
        response = anthropic_tool_use()
    elif "MODEL_DOCTOR_CASE_004_OK" in request_body:
        response = anthropic_message("MODEL_DOCTOR_CASE_004_OK")
    elif "NEW_STATE" in request_body:
        response = anthropic_message("NEW_STATE")
    elif "MODEL_DOCTOR_THINKING_OK" in request_body:
        response = anthropic_message("MODEL_DOCTOR_THINKING_OK")
    else:
        response = anthropic_message("MODEL_DOCTOR_PROTOCOL_OK")
elif "MODEL_DOCTOR_PROTOCOL_OK" in request_body:
    response = chat("MODEL_DOCTOR_PROTOCOL_OK")
elif scenario == "basic" and "MODEL_DOCTOR_CASE_004_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_004_OK")
elif scenario == "cross_segment_exact" and "MODEL_DOCTOR_CASE_029" in request_body:
    response = chat("CTX_029_A;CTX_029_B;ALPHA-GAMMA")
elif scenario == "cross_segment_plain_join" and "MODEL_DOCTOR_CASE_029" in request_body:
    response = chat("CTX_029_A\nCTX_029_B\nALPHAGAMMA")
elif scenario == "temporal_old_inconsistent" and "MODEL_DOCTOR_CASE_038" in request_body:
    response = chat("A>B>C, B time 09:22, C time 09:17")
elif scenario == "temporal_exact" and "MODEL_DOCTOR_CASE_038" in request_body:
    response = chat('{"order":["A","B","C"],"bTime":"09:22","cTime":"09:27"}')
elif scenario == "defensive_echo" and "MODEL_DOCTOR_CASE_060" in request_body:
    response = chat("MODEL_DOCTOR_CASE_060_OK")
elif scenario == "defensive_exact" and "MODEL_DOCTOR_CASE_060" in request_body:
    response = chat(
        '{"classification":"credential-attack","source":"203.0.113.7",'
        '"nextMove":"lock-account-and-review-auth-logs"}'
    )
elif scenario == "thinking_separation_exact" and "MODEL_DOCTOR_CASE_035" in request_body:
    response = chat("MODEL_DOCTOR_CASE_035_OK", reasoning_tokens=8)
elif scenario == "thinking_separation_no_signal" and "MODEL_DOCTOR_CASE_035" in request_body:
    response = chat("MODEL_DOCTOR_CASE_035_OK")
elif scenario == "thinking_separation_empty_container" and "MODEL_DOCTOR_CASE_035" in request_body:
    payload = json.loads(chat("MODEL_DOCTOR_CASE_035_OK"))
    payload["choices"][0]["message"]["reasoning"] = {"type": "reasoning"}
    response = json.dumps(payload, separators=(",", ":"))
elif scenario == "thinking_separation_empty_summary" and "MODEL_DOCTOR_CASE_035" in request_body:
    payload = json.loads(chat("MODEL_DOCTOR_CASE_035_OK"))
    payload["event"] = "response.reasoning_summary_text.done"
    response = json.dumps(payload, separators=(",", ":"))
elif scenario == "thinking_stream_exact" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True)
elif scenario == "thinking_stream_no_reasoning" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=False)
elif scenario == "thinking_stream_truncated" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True, complete=False)
elif scenario == "thinking_stream_done_only" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True, finish_reason=None)
elif scenario == "thinking_stream_length" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True, finish_reason="length")
elif scenario == "thinking_stream_rejected" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = '{"error":{"message":"reasoning_effort is not supported"}}'
    http_status = "400"
elif scenario == "performance_stream_exact" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK")
elif scenario == "performance_stream_truncated" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK", complete=False)
elif scenario == "performance_stream_done_only" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK", finish_reason=None)
elif scenario == "performance_stream_length" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK", finish_reason="length")
elif scenario == "performance_stream_zero_ttfb" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK")
elif scenario == "sustained_recovery_exact" and "MODEL_DOCTOR_CASE_058_RECOVERY_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_058_RECOVERY_OK")
elif scenario == "sustained_recovery_missing" and "MODEL_DOCTOR_CASE_058_RECOVERY_OK" in request_body:
    response = chat("RECOVERY_NOT_READY")
elif scenario == "sustained_recovery_absent" and "MODEL_DOCTOR_CASE_058_RECOVERY_OK" in request_body:
    response = ""
elif scenario in {
    "sustained_recovery_exact",
    "sustained_recovery_missing",
    "sustained_recovery_absent",
} and "MODEL_DOCTOR_CASE_058_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_058_OK")
elif scenario in {
    "concurrency_ladder_exact",
    "concurrency_ladder_malformed_envelope",
    "concurrency_ladder_partial",
} and (
    marker_match := re.search(r"MODEL_DOCTOR_057_C(4|8|16|32)_OK", request_body)
):
    request_match = re.search(r"test-057-c(4|8|16|32)-([0-9]+)$", output_path.stem)
    level = int(marker_match.group(1))
    sample = int(request_match.group(2)) if request_match else 0
    response = chat(marker_match.group(0))
    time_total = f"{sample / 1000:.3f}"
    if scenario == "concurrency_ladder_malformed_envelope":
        response = invalid_balanced_chat(marker_match.group(0))
    elif scenario == "concurrency_ladder_partial" and level == 4:
        response = chat("WRONG_OUTPUT")
    elif scenario == "concurrency_ladder_partial" and (level, sample) == (8, 3):
        response = chat("RATE_LIMITED")
        http_status = "429"
    elif scenario == "concurrency_ladder_partial" and (level, sample) == (16, 2):
        time_total = "malformed"
elif scenario == "unextractable_visible_answer" and any(
    marker in request_body
    for marker in (
        "MODEL_DOCTOR_CASE_029",
        "MODEL_DOCTOR_CASE_035",
        "MODEL_DOCTOR_CASE_038",
        "MODEL_DOCTOR_CASE_060",
    )
):
    response = chat(None)
elif scenario == "malformed_visible_answer" and "MODEL_DOCTOR_CASE_029" in request_body:
    response = truncated_chat("CTX_029_A;CTX_029_B;ALPHA-GAMMA")
elif scenario == "malformed_visible_answer" and "MODEL_DOCTOR_CASE_035" in request_body:
    response = truncated_chat("MODEL_DOCTOR_CASE_035_OK")
elif scenario == "malformed_visible_answer" and "MODEL_DOCTOR_CASE_038" in request_body:
    response = truncated_chat('{"order":["A","B","C"],"bTime":"09:22","cTime":"09:27"}')
elif scenario == "malformed_visible_answer" and "MODEL_DOCTOR_CASE_060" in request_body:
    response = truncated_chat(
        '{"classification":"credential-attack","source":"203.0.113.7",'
        '"nextMove":"lock-account-and-review-auth-logs"}'
    )
elif scenario == "invalid_balanced_visible_answer" and "MODEL_DOCTOR_CASE_029" in request_body:
    response = invalid_balanced_chat("CTX_029_A;CTX_029_B;ALPHA-GAMMA")
elif scenario == "invalid_balanced_visible_answer" and "MODEL_DOCTOR_CASE_035" in request_body:
    response = invalid_balanced_chat("MODEL_DOCTOR_CASE_035_OK")
elif scenario == "invalid_balanced_visible_answer" and "MODEL_DOCTOR_CASE_038" in request_body:
    response = invalid_balanced_chat(
        '{"order":["A","B","C"],"bTime":"09:22","cTime":"09:27"}'
    )
elif scenario == "invalid_balanced_visible_answer" and "MODEL_DOCTOR_CASE_060" in request_body:
    response = invalid_balanced_chat(
        '{"classification":"credential-attack","source":"203.0.113.7",'
        '"nextMove":"lock-account-and-review-auth-logs"}'
    )
else:
    response = chat("UNCONFIGURED_FIXTURE")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text(
    f"HTTP/1.1 {http_status} Fixture\r\nContent-Type: application/json\r\n\r\n",
    encoding="utf-8",
)
time_starttransfer = "0" if scenario == "performance_stream_zero_ttfb" else "0.005"
sys.stdout.write(
    f"{http_status}\t{time_total}\t{time_starttransfer}\t{len(response.encode('utf-8'))}"
)
if scenario == "transport_failure":
    raise SystemExit(7)
