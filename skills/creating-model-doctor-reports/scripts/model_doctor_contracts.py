#!/usr/bin/env python3
"""Exact evidence contracts and their capability-check partitions."""

from __future__ import annotations

from collections.abc import Mapping


V1_CONTRACT = ("llm-capability-doctor.evidence.v1", "0.9.0")
V2_CONTRACT = ("llm-capability-doctor.evidence.v2", "0.10.0")
V3_CONTRACT = ("llm-capability-doctor.evidence.v3", "0.11.0")
LEGACY_TEST_IDS = frozenset({
    *(f"{value:03d}" for value in range(1, 21)),
    "022", "024", "031",
    *(f"{value:03d}" for value in range(33, 37)),
    "038",
    *(f"{value:03d}" for value in range(40, 46)),
    *(f"{value:03d}" for value in range(47, 51)),
    *(f"{value:03d}" for value in range(52, 58)),
    "059", "060",
})
V3_TEST_IDS = LEGACY_TEST_IDS | frozenset({"046"})
LEGACY_CORE_TEST_IDS = frozenset({
    "001", "002", "003", "004", "005", "006", "007",
    "009", "010", "011", "012", "013", "014", "019",
    "022", "031", "038", "040", "041", "042", "043",
    "044", "047", "048", "049", "050", "052", "053",
    "054", "055", "057",
})
V3_CORE_TEST_IDS = LEGACY_CORE_TEST_IDS | frozenset({"046"})
ENHANCED_TEST_IDS = frozenset({
    "008", "015", "016", "017", "018", "020", "024",
    "033", "034", "035", "036", "045", "056", "059",
    "060",
})
CONTRACT_TEST_IDS = {
    V1_CONTRACT: LEGACY_TEST_IDS,
    V2_CONTRACT: LEGACY_TEST_IDS,
    V3_CONTRACT: V3_TEST_IDS,
}
CONTRACT_VERDICT_PARTITIONS = {
    V1_CONTRACT: (LEGACY_CORE_TEST_IDS, ENHANCED_TEST_IDS),
    V2_CONTRACT: (LEGACY_CORE_TEST_IDS, ENHANCED_TEST_IDS),
    V3_CONTRACT: (V3_CORE_TEST_IDS, ENHANCED_TEST_IDS),
}


def contract_key(run: object) -> tuple[str, str]:
    """Return a supported exact schema/version pair from run metadata."""

    if not isinstance(run, Mapping):
        raise ValueError("Run metadata must be a mapping")

    schema = run.get("log_schema")
    version = run.get("script_version")
    if not isinstance(schema, str) or not isinstance(version, str):
        raise ValueError(
            f"Unsupported log schema/version pair: {(schema, version)!r}"
        )

    try:
        contract = (schema, version)
        hash(contract)
        supported = contract in CONTRACT_TEST_IDS
    except TypeError as error:
        raise ValueError(
            f"Unsupported log schema/version pair: {(schema, version)!r}"
        ) from error
    if not supported:
        raise ValueError(f"Unsupported log schema/version pair: {contract!r}")
    return contract
