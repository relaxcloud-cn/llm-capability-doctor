#!/usr/bin/env python3
"""Derive OpenCodex data-format compatibility from reviewed evidence."""

from __future__ import annotations

from typing import Mapping


COMPATIBILITY_PROFILE = "opencodex-2.7.42-data-format"
EVIDENCE_CONTRACT = ("llm-capability-doctor.evidence.v4", "0.12.0")
REQUIRED_TEST_IDS = (
    "002",
    "004",
    "005",
    "006",
    "040",
    "041",
    "043",
    "047",
)
SUPPORTED_PROTOCOL_FAMILIES = frozenset(
    {
        "OPENAI_CHAT_COMPLETIONS",
        "OPENAI_RESPONSES",
        "ANTHROPIC_MESSAGES",
        "GEMINI_GENERATE_CONTENT",
    }
)
LABELS = {
    "PASS": "OpenCodex 数据格式兼容",
    "FAIL": "OpenCodex 数据格式不兼容",
    "NOT_ASSESSED": "OpenCodex 数据格式未评定",
}
SCOPE_BOUNDARY = (
    "仅判断本轮模型端数据格式，不覆盖鉴权、网络、部署或 ClawOps 运行环境。"
)


def derive_opencodex_compatibility(
    run: Mapping[str, object],
    statuses: Mapping[str, str],
    protocol_family: str,
) -> dict:
    """Return the deterministic OpenCodex compatibility object."""

    contract = (run.get("log_schema"), run.get("script_version"))
    collected_for_profile = (
        contract == EVIDENCE_CONTRACT
        and run.get("compatibilityProfile") == COMPATIBILITY_PROFILE
    )

    if not collected_for_profile:
        level = "NOT_ASSESSED"
        failed_test_ids = []
        statement = (
            "本轮日志未按 OpenCodex 2.7.42 数据格式配置采集，"
            "因此本轮兼容性未评定。"
        )
    else:
        failed_test_ids = [
            test_id
            for test_id in REQUIRED_TEST_IDS
            if statuses.get(test_id) != "PASS"
        ]
        protocol_supported = protocol_family in SUPPORTED_PROTOCOL_FAMILIES
        if protocol_supported and not failed_test_ids:
            level = "PASS"
            statement = (
                "本轮八项必需数据格式检查全部通过，因此判定符合 "
                "OpenCodex 2.7.42 数据格式合同。"
            )
        elif protocol_supported:
            level = "FAIL"
            failed_ids = "、".join(failed_test_ids)
            statement = (
                f"本轮八项必需数据格式检查有 {len(failed_test_ids)} 项未通过"
                f"（{failed_ids}），因此判定不符合 OpenCodex 2.7.42 "
                "数据格式合同。"
            )
        elif failed_test_ids:
            level = "FAIL"
            failed_ids = "、".join(failed_test_ids)
            statement = (
                f"本轮识别协议 {protocol_family} 不属于 OpenCodex 2.7.42 "
                "数据格式配置支持的四类协议，且八项必需数据格式检查有 "
                f"{len(failed_test_ids)} 项未通过（{failed_ids}），"
                "因此判定不符合该数据格式合同。"
            )
        else:
            level = "FAIL"
            statement = (
                f"本轮识别协议 {protocol_family} 不属于 OpenCodex 2.7.42 "
                "数据格式配置支持的四类协议，因此判定不符合该数据格式合同。"
            )

    return {
        "profile": COMPATIBILITY_PROFILE,
        "level": level,
        "label": LABELS[level],
        "protocolFamily": protocol_family,
        "requiredTestIds": list(REQUIRED_TEST_IDS),
        "failedTestIds": failed_test_ids,
        "statement": statement,
        "scopeBoundary": SCOPE_BOUNDARY,
    }
