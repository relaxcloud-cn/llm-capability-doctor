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
else:
    response = chat("UNCONFIGURED_FIXTURE")

output_path.write_text(response, encoding="utf-8")
headers_path.write_text(
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n",
    encoding="utf-8",
)
sys.stdout.write(f"200\t0.020\t0.005\t{len(response.encode('utf-8'))}")
