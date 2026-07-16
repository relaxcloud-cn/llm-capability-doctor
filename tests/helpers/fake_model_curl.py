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
else:
    response = chat("UNCONFIGURED_FIXTURE")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text(
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n",
    encoding="utf-8",
)
sys.stdout.write(f"200\t0.020\t0.005\t{len(response.encode('utf-8'))}")
