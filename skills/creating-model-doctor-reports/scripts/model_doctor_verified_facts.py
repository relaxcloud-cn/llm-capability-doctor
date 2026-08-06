"""Validate evidence-bounded capability facts for Model Doctor reports."""

from __future__ import annotations

import re
from typing import Dict, List, Set, Tuple


EVIDENCE_STATES = {"VERIFIED", "INCONCLUSIVE", "NOT_COLLECTED"}
PROTOCOL_FAMILIES = {
    "OPENAI_CHAT_COMPLETIONS",
    "OPENAI_RESPONSES",
    "ANTHROPIC_MESSAGES",
    "GEMINI_GENERATE_CONTENT",
    "OLLAMA_CHAT",
    "CUSTOM",
    "UNKNOWN",
}
VERIFIED_FACTS_FIELDS = {"interfaceProtocol", "contextWindow", "concurrency"}
INTERFACE_FIELDS = {
    "evidenceState",
    "family",
    "requestFormat",
    "responseFormat",
    "statement",
    "evidenceRefs",
    "boundary",
}
CONTEXT_FIELDS = {
    "evidenceState",
    "highestVerifiedTier",
    "highestVerifiedInputTokens",
    "firstFailedTier",
    "firstFailedInputTokens",
    "statement",
    "evidenceRefs",
    "boundary",
}
CONCURRENCY_FIELDS = {
    "evidenceState",
    "highestVerifiedConcurrentRequests",
    "statement",
    "evidenceRefs",
    "boundary",
}
_UNBOUNDED_CLAIMS = re.compile(
    r"真实最大|硬上限|最大上下文|最大并发|一定支持更高"
)


def _non_empty(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _refs(value: object) -> bool:
    return (
        isinstance(value, list)
        and all(_non_empty(item) for item in value)
        and len(value) == len(set(value))
    )


def _unbounded_claim(value: object) -> bool:
    if not isinstance(value, str):
        return False
    safe = (
        "不是硬上限",
        "不代表硬上限",
        "真实上限未测试",
        "上限无法确认",
        "不能确定上限",
    )
    reduced = value
    for phrase in safe:
        reduced = reduced.replace(phrase, "")
    return bool(_UNBOUNDED_CLAIMS.search(reduced))


def _shape_errors(
    value: object,
    fields: Set[str],
    prefix: str,
) -> Tuple[List[str], bool]:
    if not isinstance(value, dict):
        return [f"{prefix} must be an object"], False

    errors = [
        f"{prefix} field {field} is not allowed"
        for field in sorted(set(value) - fields)
    ]
    errors.extend(
        f"{prefix} {field} is required"
        for field in sorted(fields - set(value))
    )
    return errors, not errors


def _shared_errors(
    value: dict,
    prefix: str,
    allowed_refs: Set[str],
) -> List[str]:
    errors: List[str] = []
    evidence_state = value.get("evidenceState")
    if not isinstance(evidence_state, str) or evidence_state not in EVIDENCE_STATES:
        errors.append(f"{prefix} evidenceState is invalid")
    for field in ("statement", "boundary"):
        if not _non_empty(value.get(field)):
            errors.append(f"{prefix} {field} is required")
    if _unbounded_claim(value.get("statement")):
        errors.append(f"{prefix} statement contains an unbounded maximum claim")

    evidence_refs = value.get("evidenceRefs")
    if not _refs(evidence_refs):
        errors.append(f"{prefix} evidenceRefs must be a unique string array")
    else:
        for reference in sorted(set(evidence_refs) - allowed_refs):
            errors.append(
                f"{prefix} evidence reference is outside its allowed domain: "
                f"{reference}"
            )
    return errors


def _has_refs(value: object) -> bool:
    return _refs(value) and bool(value)


def _validate_interface(value: object, allowed_refs: Set[str]) -> List[str]:
    prefix = "interfaceProtocol"
    errors, valid_shape = _shape_errors(value, INTERFACE_FIELDS, prefix)
    if not valid_shape:
        return errors
    assert isinstance(value, dict)

    errors.extend(_shared_errors(value, prefix, allowed_refs))
    family = value.get("family")
    if not isinstance(family, str) or family not in PROTOCOL_FAMILIES:
        errors.append(f"{prefix} family is invalid")
    for field in ("requestFormat", "responseFormat"):
        if not _non_empty(value.get(field)):
            errors.append(f"{prefix} {field} is required")

    state = value.get("evidenceState")
    refs = value.get("evidenceRefs")
    if state == "VERIFIED":
        if not _has_refs(refs):
            errors.append(f"{prefix} VERIFIED requires evidenceRefs")
        if family == "UNKNOWN":
            errors.append(f"{prefix} VERIFIED requires a known family")
    elif state == "INCONCLUSIVE" and not _has_refs(refs):
        errors.append(f"{prefix} INCONCLUSIVE requires evidenceRefs")
    elif state == "NOT_COLLECTED":
        if family != "UNKNOWN":
            errors.append(f"{prefix} NOT_COLLECTED requires family UNKNOWN")
        if refs != []:
            errors.append(f"{prefix} NOT_COLLECTED requires empty evidenceRefs")
    return errors


def _tier(value: object) -> bool:
    return value is None or _non_empty(value)


def _nonnegative_integer(value: object) -> bool:
    return value is None or (
        isinstance(value, int) and not isinstance(value, bool) and value >= 0
    )


def _validate_context(value: object, allowed_refs: Set[str]) -> List[str]:
    prefix = "contextWindow"
    errors, valid_shape = _shape_errors(value, CONTEXT_FIELDS, prefix)
    if not valid_shape:
        return errors
    assert isinstance(value, dict)

    errors.extend(_shared_errors(value, prefix, allowed_refs))
    tier_fields = ("highestVerifiedTier", "firstFailedTier")
    token_fields = ("highestVerifiedInputTokens", "firstFailedInputTokens")
    for field in tier_fields:
        if not _tier(value.get(field)):
            errors.append(f"{prefix} {field} must be a non-empty string or null")
    for field in token_fields:
        if not _nonnegative_integer(value.get(field)):
            errors.append(f"{prefix} {field} must be a non-negative integer or null")
    if (
        value.get("firstFailedTier") is None
        and value.get("firstFailedInputTokens") is not None
    ):
        errors.append(
            f"{prefix} firstFailedInputTokens requires firstFailedTier"
        )

    state = value.get("evidenceState")
    refs = value.get("evidenceRefs")
    if state == "VERIFIED":
        if not _non_empty(value.get("highestVerifiedTier")):
            errors.append(f"{prefix} VERIFIED requires highestVerifiedTier")
        if not _has_refs(refs):
            errors.append(f"{prefix} VERIFIED requires evidenceRefs")
    elif state == "INCONCLUSIVE" and not _has_refs(refs):
        errors.append(f"{prefix} INCONCLUSIVE requires evidenceRefs")
    elif state == "NOT_COLLECTED":
        if any(value.get(field) is not None for field in tier_fields + token_fields):
            errors.append(
                f"{prefix} NOT_COLLECTED requires all tier and token values to be null"
            )
        if refs != []:
            errors.append(f"{prefix} NOT_COLLECTED requires empty evidenceRefs")
    return errors


def _positive_integer(value: object) -> bool:
    return value is None or (
        isinstance(value, int) and not isinstance(value, bool) and value > 0
    )


def _validate_concurrency(value: object, allowed_refs: Set[str]) -> List[str]:
    prefix = "concurrency"
    errors, valid_shape = _shape_errors(value, CONCURRENCY_FIELDS, prefix)
    if not valid_shape:
        return errors
    assert isinstance(value, dict)

    errors.extend(_shared_errors(value, prefix, allowed_refs))
    concurrent_requests = value.get("highestVerifiedConcurrentRequests")
    if not _positive_integer(concurrent_requests):
        errors.append(
            f"{prefix} highestVerifiedConcurrentRequests must be a positive "
            "integer or null"
        )

    state = value.get("evidenceState")
    refs = value.get("evidenceRefs")
    if state == "VERIFIED":
        if not (
            isinstance(concurrent_requests, int)
            and not isinstance(concurrent_requests, bool)
            and concurrent_requests > 0
        ):
            errors.append(
                f"{prefix} VERIFIED requires highestVerifiedConcurrentRequests"
            )
        if not _has_refs(refs):
            errors.append(f"{prefix} VERIFIED requires evidenceRefs")
    elif state == "INCONCLUSIVE" and not _has_refs(refs):
        errors.append(f"{prefix} INCONCLUSIVE requires evidenceRefs")
    elif state == "NOT_COLLECTED":
        if concurrent_requests is not None:
            errors.append(
                f"{prefix} NOT_COLLECTED requires "
                "highestVerifiedConcurrentRequests to be null"
            )
        if refs != []:
            errors.append(f"{prefix} NOT_COLLECTED requires empty evidenceRefs")
    return errors


def validate_verified_facts(
    facts: object,
    evidence_domains: Dict[str, Set[str]],
) -> List[str]:
    """Validate required fact shapes, state/value combinations, and evidence domains."""

    errors: List[str] = []
    if not isinstance(facts, dict):
        return ["capabilitySummary verifiedFacts must be an object"]
    for field in sorted(set(facts) - VERIFIED_FACTS_FIELDS):
        errors.append(f"verifiedFacts field {field} is not allowed")
    for field in sorted(VERIFIED_FACTS_FIELDS - set(facts)):
        errors.append(f"verifiedFacts {field} is required")
    if errors:
        return errors

    domains = evidence_domains if isinstance(evidence_domains, dict) else {}
    errors.extend(
        _validate_interface(
            facts["interfaceProtocol"],
            set(domains.get("interfaceProtocol", set())),
        )
    )
    errors.extend(
        _validate_context(
            facts["contextWindow"],
            set(domains.get("contextWindow", set())),
        )
    )
    errors.extend(
        _validate_concurrency(
            facts["concurrency"],
            set(domains.get("concurrency", set())),
        )
    )
    return errors
