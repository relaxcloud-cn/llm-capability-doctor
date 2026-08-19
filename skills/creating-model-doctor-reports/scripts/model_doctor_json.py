"""Strict JSON decoding shared by Model Doctor evidence validators."""

from __future__ import annotations

import json
import math


JSON_LOAD_ERRORS = (TypeError, ValueError, RecursionError)


def _reject_constant(value: str) -> object:
    raise ValueError(f"non-finite JSON constant: {value}")


def _finite_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise ValueError(f"non-finite JSON number: {value}")
    return parsed


def strict_json_loads(value: object) -> object:
    return json.loads(
        value,
        parse_constant=_reject_constant,
        parse_float=_finite_float,
    )
