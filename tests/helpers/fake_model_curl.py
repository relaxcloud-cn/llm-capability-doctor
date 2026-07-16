#!/usr/bin/env python3
"""Minimal curl-compatible executable for Model Doctor CLI tests."""

import json
import os
import sys
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


def chat_stream(*, content="MODEL_DOCTOR_CASE_036_OK", include_reasoning=False, complete=True):
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
    if complete:
        events.append(
            {
                "choices": [{"delta": {}, "finish_reason": "stop"}],
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

if "MODEL_DOCTOR_PROTOCOL_OK" in request_body:
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
elif scenario == "thinking_stream_exact" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True)
elif scenario == "thinking_stream_no_reasoning" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=False)
elif scenario == "thinking_stream_truncated" and "MODEL_DOCTOR_CASE_036" in request_body:
    response = chat_stream(include_reasoning=True, complete=False)
elif scenario == "performance_stream_exact" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK")
elif scenario == "performance_stream_truncated" and "MODEL_DOCTOR_CASE_053" in request_body:
    response = chat_stream(content="MODEL_DOCTOR_CASE_053_OK", complete=False)
elif scenario == "sustained_recovery_exact" and "MODEL_DOCTOR_CASE_058_RECOVERY_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_058_RECOVERY_OK")
elif scenario == "sustained_recovery_missing" and "MODEL_DOCTOR_CASE_058_RECOVERY_OK" in request_body:
    response = chat("RECOVERY_NOT_READY")
elif scenario in {"sustained_recovery_exact", "sustained_recovery_missing"} and "MODEL_DOCTOR_CASE_058_OK" in request_body:
    response = chat("MODEL_DOCTOR_CASE_058_OK")
else:
    response = chat("UNCONFIGURED_FIXTURE")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text(
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n",
    encoding="utf-8",
)
sys.stdout.write(f"200\t0.020\t0.005\t{len(response.encode('utf-8'))}")
