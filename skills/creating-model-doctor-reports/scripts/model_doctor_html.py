#!/usr/bin/env python3
"""Render a validated Model Doctor assessment as offline customer HTML.

The layout follows the approved report design: a neutral diagnostic report
with a fixed reading path 总体结果 → 需要处理的问题 → 本次检测信息 →
能力检查结果.  The assessment JSON stays the audit record; plain-language
domain and test names come from the fixed display catalog keyed by test ID.
"""

from __future__ import annotations

from html import escape
from pathlib import Path
from typing import Dict, Iterable, List

from model_doctor_assessment import validate_assessment
from model_doctor_contracts import (
    CONTRACT_VERDICT_PARTITIONS,
    V4_CONTRACT,
    contract_key,
)
from model_doctor_json import JSON_LOAD_ERRORS, strict_json_loads
from model_doctor_display import (
    TIER_LABELS,
    display_domain,
    display_name,
    domain_order,
    failure_kind_label,
    protocol_family_label,
    sufficiency_label,
)


CSP = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
REPORT_TITLE = "大模型能力诊断报告"
VERDICT_LEVELS = {"PASS", "CONDITIONAL_PASS", "FAIL", "NOT_ASSESSED"}
VERDICT_TONE = {
    "PASS": "pass",
    "CONDITIONAL_PASS": "warn",
    "FAIL": "fail",
    "NOT_ASSESSED": "neutral",
}
COMPAT_LEVELS = {"PASS", "FAIL", "NOT_ASSESSED"}
COMPAT_VALUE = {
    "PASS": "兼容",
    "FAIL": "不兼容",
    "NOT_ASSESSED": "未评定",
}
GENERAL_BOUNDARY = "这个结果只说明本次检查的情况，是否上线还需要结合项目要求判断。"
IDENTITY_BOUNDARY = (
    "发送内容和返回内容里的模型名称可能不同；这些名称不能单独证明背后实际"
    "使用的是哪个商业模型。"
)
CAPABILITY_SECTION_DESCRIPTION = (
    "分别检查接口能否正常使用、能否按格式输出、能否处理长文本、能否按要求"
    "回答、逻辑任务、工具调用、响应速度和安全业务场景。每一项都可以展开查看"
    "判定说明和发给模型、模型返回的完整原始内容。"
)
NAV_SECTIONS = (
    ("conclusion", "总体结果"),
    ("issues", "需要处理的问题"),
    ("run-info", "本次检测信息"),
    ("capabilities", "能力检查"),
)


def _e(value: object) -> str:
    return escape("" if value is None else str(value), quote=True)


def _list(items: Iterable[object], empty: str = "无") -> str:
    values = list(items or [])
    if not values:
        return f'<p class="muted">{_e(empty)}</p>'
    return "<ul>" + "".join(f"<li>{_e(item)}</li>" for item in values) + "</ul>"


def _optional_logic_item(title: str, items: Iterable[object]) -> str:
    values = list(items or [])
    if not values:
        return ""
    return (
        '<section class="logic-item">'
        f"<h5>{_e(title)}</h5>{_list(values)}"
        "</section>"
    )


def _section_head(
    kicker: str,
    title: str,
    description: str = "",
) -> str:
    description_html = (
        f'<p class="section-description">{_e(description)}</p>'
        if description
        else ""
    )
    return (
        '<div class="section-head">'
        '<div class="section-title-wrap">'
        f'<p class="section-kicker">{_e(kicker)}</p>'
        f'<h2 class="section-title">{_e(title)}</h2>'
        f"{description_html}"
        "</div></div>"
    )


def _verdict_block(
    tone: str,
    label: str,
    value: str,
    statement: object,
    boundary: object,
) -> str:
    statement_html = (
        f'<p class="verdict-statement">{_e(statement)}</p>'
        if statement
        else ""
    )
    boundary_html = (
        f'<p class="verdict-boundary">{_e(boundary)}</p>' if boundary else ""
    )
    return (
        f'<div class="verdict-block {tone}">'
        f'<p class="verdict-label">{_e(label)}</p>'
        f'<p class="verdict-value">{_e(value)}</p>'
        f"{statement_html}{boundary_html}"
        "</div>"
    )


def _compat_display_value(level: object) -> str:
    key = level if isinstance(level, str) else ""
    return COMPAT_VALUE.get(key, "未评定")


def _opencodex_compatibility(compatibility: dict) -> str:
    """Render the OpenCodex compatibility verdict block for the first screen."""

    compatibility = compatibility if isinstance(compatibility, dict) else {}
    level = compatibility.get("level")
    tone = (
        VERDICT_TONE.get(level, "neutral")
        if level in COMPAT_LEVELS
        else "neutral"
    )
    return _verdict_block(
        tone,
        "能否直接接入 OpenCodex（数据格式）",
        _compat_display_value(level),
        compatibility.get("statement"),
        compatibility.get("scopeBoundary"),
    )


def _fact_strip(capability_summary: dict, counts_summary: dict) -> str:
    verdict = capability_summary.get("generalVerdict")
    verdict = verdict if isinstance(verdict, dict) else {}
    facts = capability_summary.get("verifiedFacts")
    facts = facts if isinstance(facts, dict) else {}
    context = facts.get("contextWindow")
    context = context if isinstance(context, dict) else {}
    concurrency = facts.get("concurrency")
    concurrency = concurrency if isinstance(concurrency, dict) else {}

    level = verdict.get("level")
    if level == "NOT_ASSESSED":
        result_value = (
            f'已采集 {verdict.get("collectedTests", 0)}/'
            f'{verdict.get("totalTests", 0)}'
        )
    else:
        result_value = (
            f'{verdict.get("passedTests", 0)} 通过 · '
            f'{_failed_count(counts_summary)} 未通过'
        )
    core_value = (
        f'{verdict.get("passedCoreTests", 0)} / '
        f'{verdict.get("totalCoreTests", 0)} 通过'
    )

    context_value = "未确认"
    if context.get("evidenceState") == "VERIFIED":
        context_value = context.get("highestVerifiedTier") or "已验证"
    concurrency_value = "未确认"
    if concurrency.get("evidenceState") == "VERIFIED":
        highest = concurrency.get("highestVerifiedConcurrentRequests")
        if highest is not None:
            concurrency_value = f"{highest} 路短时并发"

    cells = (
        ("检测项", result_value),
        ("关键检查项", core_value),
        ("本次通过的最长输入", context_value),
        ("本次通过的最高并发", concurrency_value),
    )
    items = "".join(
        f"<div><dt>{_e(label)}</dt><dd>{_e(value)}</dd></div>"
        for label, value in cells
    )
    return f'<dl class="fact-strip">{items}</dl>'


def _failed_count(summary: dict) -> int:
    counts = summary.get("counts", {}) if isinstance(summary, dict) else {}
    return int(counts.get("FAIL", 0) or 0)


def _issues_headline(issues: List[dict]) -> str:
    titles = [
        str(issue.get("title"))
        for issue in issues
        if isinstance(issue, dict) and issue.get("title")
    ]
    if not titles:
        return "本轮检查没有未通过项。"
    return f"当前主要有 {len(titles)} 类问题：{'；'.join(titles)}。"


def _issue_impact_label(issue: dict, tests_by_id: Dict[str, dict]) -> str:
    compat_required = {
        "002",
        "004",
        "005",
        "006",
        "040",
        "041",
        "043",
        "047",
    }
    referenced = [
        tests_by_id.get(str(ref))
        for ref in issue.get("testRefs", [])
        if isinstance(tests_by_id.get(str(ref)), dict)
    ]
    if any(str(item.get("testId")) in compat_required for item in referenced):
        return "会影响接入"
    if referenced and all(
        item.get("failureAnalysis", {}).get("failureKind")
        in {"MEASUREMENT_UNAVAILABLE", "EVIDENCE_GAP"}
        for item in referenced
    ):
        return "本次无法测量"
    return "需要处理"


def _issues_section(capability_summary: dict, tests: List[dict]) -> str:
    issues = capability_summary.get("issues", [])
    issues = [issue for issue in issues if isinstance(issue, dict)]
    tests_by_id = {
        str(item.get("testId")): item
        for item in tests
        if isinstance(item, dict)
    }
    if issues:
        title = f"先处理这 {len(issues)} 个问题"
        description = (
            "先看最影响使用的问题；每个问题下面保留对应检测项编号，"
            "完整判定过程和原始记录在第 04 节对应检测项中。"
        )
    else:
        title = "本轮没有需要处理的问题"
        description = "本轮全部检测项通过。"
    rendered_issues = []
    for issue in issues:
        refs = "、".join(str(value) for value in issue.get("testRefs", []))
        rendered_issues.append(
            '<article class="issue">'
            '<div class="issue-top"><div>'
            f"<h3>{_e(issue.get('title'))}</h3>"
            f'<p class="issue-statement">{_e(issue.get("statement"))}</p>'
            "</div>"
            f'<span class="impact-label">{_e(_issue_impact_label(issue, tests_by_id))}</span>'
            "</div>"
            '<div class="issue-grid">'
            f'<div class="issue-field"><span>关联检测项</span><strong>{_e(refs)}</strong></div>'
            f'<div class="issue-field"><span>我们能确定什么</span><strong>{_e(issue.get("boundary"))}</strong></div>'
            "</div></article>"
        )
    body = (
        '<div class="issue-list">' + "".join(rendered_issues) + "</div>"
        if rendered_issues
        else '<p class="issue-empty">本轮没有需要处理的问题。</p>'
    )
    return (
        '<section class="report-section" id="issues" data-nav-section>'
        f"{_section_head('02 · 需要处理的问题', title, description)}"
        f"{body}"
        "</section>"
    )


def _response_model_name(tests: List[dict]) -> str:
    for item in tests:
        if not isinstance(item, dict):
            continue
        for request in item.get("requests", []):
            if not isinstance(request, dict):
                continue
            body = request.get("responseBody")
            if not isinstance(body, str) or not body.strip():
                continue
            try:
                document = strict_json_loads(body)
            except JSON_LOAD_ERRORS:
                continue
            if isinstance(document, dict) and isinstance(
                document.get("model"), str
            ) and document["model"].strip():
                return document["model"]
    return ""


def _sha256_display(value: object) -> str:
    text = value if isinstance(value, str) else ""
    if len(text) >= 24:
        return f"{text[:16]}…{text[-4:]}（用于确认记录没有被改动）"
    return text or "未记录"


def _capability_summary(assessment: dict) -> str:
    """Render section 01 本次检测结果: verdict pillars, facts, headline."""

    capability_summary = assessment.get("capabilitySummary", {})
    capability_summary = (
        capability_summary if isinstance(capability_summary, dict) else {}
    )
    verdict = capability_summary.get("generalVerdict")
    verdict = verdict if isinstance(verdict, dict) else {}
    level = verdict.get("level")
    tone = (
        VERDICT_TONE.get(level, "neutral")
        if level in VERDICT_LEVELS
        else "neutral"
    )
    issues = [
        issue
        for issue in capability_summary.get("issues", [])
        if isinstance(issue, dict)
    ]

    description = (
        f"通用能力：{verdict.get('label') or '未评定'}；"
        f"OpenCodex 数据格式：{_compat_display_value(capability_summary.get('openCodexCompatibility', {}).get('level'))}。"
    )
    return (
        '<section class="report-section" id="conclusion" data-nav-section>'
        f"{_section_head('01 · 总体结果', '本次检测结果', description)}"
        '<div class="result-columns">'
        '<section class="result-pillar" aria-labelledby="general-result-title">'
        '<h3 id="general-result-title">通用能力</h3>'
        f"{_verdict_block(tone, '总体结果', verdict.get('label') or '通用能力未评定', verdict.get('statement'), GENERAL_BOUNDARY)}"
        "</section>"
        '<section class="result-pillar" aria-labelledby="compatibility-result-title">'
        '<h3 id="compatibility-result-title">兼容情况</h3>'
        f"{_opencodex_compatibility(capability_summary.get('openCodexCompatibility', {}))}"
        "</section>"
        "</div>"
        f"{_fact_strip(assessment.get('capabilitySummary', {}), assessment.get('summary', {}))}"
        f'<p class="conclusion-headline">{_e(_issues_headline(issues))}</p>'
        f"{_verified_facts(capability_summary.get('verifiedFacts', {}))}"
        "</section>"
    )


def _run_metadata(
    run: dict,
    facts: dict,
    source: dict,
    tests: List[dict],
) -> str:
    facts = facts if isinstance(facts, dict) else {}
    interface = facts.get("interfaceProtocol")
    interface = interface if isinstance(interface, dict) else {}
    run = run if isinstance(run, dict) else {}
    source = source if isinstance(source, dict) else {}

    family = interface.get("family")
    protocol_value = protocol_family_label(family)
    if family in {
        "OPENAI_CHAT_COMPLETIONS",
        "OPENAI_RESPONSES",
        "ANTHROPIC_MESSAGES",
        "GEMINI_GENERATE_CONTENT",
        "OLLAMA_CHAT",
    }:
        protocol_value = f"{protocol_value}（接口消息格式）"

    response_model = _response_model_name(tests) or "未观察到"
    script_version = run.get("script_version") or "未知"
    log_schema = run.get("log_schema") or "未记录"
    profile = run.get("compatibilityProfile") or "本次未使用 OpenCodex 检查配置"

    rows = (
        ("检测地址（URL）", run.get("url") or "未知"),
        ("发送时使用的模型名称", run.get("model") or "未知"),
        ("接口返回的模型名称", response_model),
        ("接口格式", protocol_value),
        ("访问密钥（API Key）", run.get("api_key") or "未知"),
        ("检测工具版本", f"v{script_version} · {log_schema}"),
        ("报告数据版本", "llm-capability-doctor.assessment.v9"),
        ("OpenCodex 检查标准", profile),
        ("原始记录校验值", _sha256_display(source.get("sha256"))),
    )
    rendered = "".join(
        f"<div><dt>{_e(label)}</dt><dd>{_e(value)}</dd></div>"
        for label, value in rows
    )
    return (
        '<section class="report-section" id="run-info" data-nav-section>'
        f"{_section_head('03 · 本次检测', '本次检测信息', '用于确认检测的是哪个接口、哪个模型，以及使用了哪个版本的检测工具。')}"
        f'<dl class="metadata">{rendered}</dl>'
        f'<p class="identity-boundary">{IDENTITY_BOUNDARY}</p>'
        "</section>"
    )


def _verified_facts(facts: dict) -> str:
    """Render the three capability facts with plain-language detail lines."""

    facts = facts if isinstance(facts, dict) else {}
    interface = facts.get("interfaceProtocol")
    interface = interface if isinstance(interface, dict) else {}
    context = facts.get("contextWindow")
    context = context if isinstance(context, dict) else {}
    concurrency = facts.get("concurrency")
    concurrency = concurrency if isinstance(concurrency, dict) else {}

    family_label = protocol_family_label(interface.get("family"))
    interface_detail = (
        f'发给模型的内容：{interface.get("requestFormat") or "未记录"}；'
        f'模型返回的内容：{interface.get("responseFormat") or "未记录"}；'
        f"归类：{family_label}。"
    )

    def _token_text(value: object) -> str:
        return str(value) if value is not None else "未观察到"

    context_detail = (
        f"本次通过的最高档：{context.get('highestVerifiedTier') or '未观察到'}；"
        f"该次请求的输入 Token：{_token_text(context.get('highestVerifiedInputTokens'))}；"
        f"首个未通过档：{context.get('firstFailedTier') or '未观察到'}；"
        f"该次请求的输入 Token：{_token_text(context.get('firstFailedInputTokens'))}。"
    )
    highest_concurrency = concurrency.get("highestVerifiedConcurrentRequests")
    concurrency_detail = (
        f"本次全部成功的最高同时请求数："
        f"{highest_concurrency if highest_concurrency is not None else '未观察到'}。"
    )

    rows = (
        ("接口格式（依据检测 002）", interface.get("statement"), interface_detail, interface.get("boundary")),
        ("长文本处理（依据检测 014-018）", context.get("statement"), context_detail, context.get("boundary")),
        ("同时请求（依据检测 057）", concurrency.get("statement"), concurrency_detail, concurrency.get("boundary")),
    )
    rendered = "".join(
        "<div>"
        f"<dt>{_e(label)}</dt>"
        f"<dd><strong>{_e(statement)}</strong>"
        f'<span class="verified-fact-detail">{_e(detail)}</span>'
        f'<span class="verified-fact-boundary">{_e(boundary)}</span>'
        "</dd></div>"
        for label, statement, detail, boundary in rows
    )
    return f'<dl class="verified-facts">{rendered}</dl>'


def _tier_for_test(test_id: str, core_ids: frozenset) -> str:
    return "core" if test_id in core_ids else "enhanced"


def _test_row(
    item: dict,
    tier: str,
) -> str:
    test_id = str(item.get("testId", "unknown"))
    detail_id = f"test-detail-{test_id}"
    reviewed = item.get("reviewedStatus", "FAIL")
    name = display_name(item)
    manifest_name = str(item.get("name") or "")
    logic = item.get("logic", {})
    failure = item.get("failureAnalysis", {})
    search_text = f"{test_id} {name} {manifest_name}".lower()

    status_text = "通过" if reviewed == "PASS" else "未通过"
    tier_label = TIER_LABELS.get(tier, TIER_LABELS["enhanced"])

    failure_html = _failure_audit(item)
    extras = "".join(
        (
            _optional_logic_item("什么情况算未通过", logic.get("failCriteria", [])),
            _optional_logic_item("关键证据摘录", item.get("evidenceExcerpts", [])),
            _optional_logic_item("这项检测的已知限制", item.get("limitations", [])),
            _optional_logic_item("需要时的重跑方法", item.get("retestInstructions", [])),
        )
    )
    logic_grid = (
        '<div class="logic-grid">'
        '<section class="logic-item"><h5>要检查什么</h5>'
        f'<p>{_e(logic.get("purpose"))}</p></section>'
        '<section class="logic-item"><h5>怎么检查</h5>'
        f'<p>{_e(logic.get("method"))}</p></section>'
        '<section class="logic-item"><h5>什么情况算通过</h5>'
        f'{_list(logic.get("passCriteria", []))}</section>'
        '<section class="logic-item"><h5>这个结果不能说明什么</h5>'
        f'<p>{_e(logic.get("capabilityBoundary"))}</p></section>'
        f"{extras}"
        "</div>"
    )
    return (
        f'<details class="test-row" data-status="{_e(reviewed)}" '
        f'data-tier="{_e(tier)}" data-search="{_e(search_text)}">'
        "<summary>"
        '<span class="test-summary">'
        f'<span class="test-id">{_e(test_id)}</span>'
        f'<span class="test-name">{_e(name)}</span>'
        f'<span class="test-tier">{_e(tier_label)}</span>'
        f'<span class="test-status {status_text == "通过" and "pass" or "fail"}">{status_text}</span>'
        "</span>"
        "</summary>"
        f'<div class="test-detail" id="{_e(detail_id)}">'
        f'<p class="test-conclusion">{_e(item.get("conclusion"))}</p>'
        f"{logic_grid}"
        f"{failure_html}"
        f"{_request_evidence(item.get('requests', []))}"
        "</div></details>"
    )


def _failure_audit(item: dict) -> str:
    if item.get("reviewedStatus") != "FAIL":
        return ""
    analysis = item.get("failureAnalysis", {})
    rows = [
        ("为什么判为未通过", failure_kind_label(analysis.get("failureKind"))),
        ("现有记录能支持哪些判断", sufficiency_label(analysis.get("evidenceSufficiency"))),
        ("本次可以确认", analysis.get("supportedClaim")),
    ]
    unsupported = analysis.get("unsupportedClaims", [])
    if unsupported:
        rows.append(("不能因此断定", "；".join(str(value) for value in unsupported)))
    dependencies = analysis.get("dependsOnTestIds", [])
    if dependencies:
        rows.append(("关联的其他未通过项", "、".join(str(value) for value in dependencies)))
    evidence_refs = analysis.get("evidenceRefs", [])
    if evidence_refs:
        rows.append(("对应的原始记录编号", "、".join(str(value) for value in evidence_refs)))
    details = "".join(
        f"<div><dt>{_e(label)}</dt><dd>{_e(value)}</dd></div>"
        for label, value in rows
    )
    return (
        '<section class="failure-audit">'
        "<h5>为什么没有通过</h5>"
        f'<dl class="failure-audit-details">{details}</dl>'
        "</section>"
    )


def _request_evidence(requests: List[dict]) -> str:
    if not requests:
        return '<p class="empty-evidence">日志未包含可关联的完整请求块。</p>'
    turns = []
    for index, request in enumerate(requests, start=1):
        metrics = request.get("metrics", {})
        v4_metadata = tuple(
            f"{field}={request[field]}"
            for field in (
                "transport_outcome",
                "stream_termination",
                "stream_end_signal",
                "tool_contract_status",
                "tool_loop_turn",
                "tool_loop_outcome",
            )
            if field in request
        )
        meta = " · ".join(
            value
            for value in (
                str(request.get("request_id") or f"Turn {index}"),
                f"curl exit {metrics.get('curl_exit_code')}"
                if metrics.get("curl_exit_code")
                else "",
                f"HTTP {metrics.get('http_status')}" if metrics.get("http_status") else "",
                f"首次返回 {metrics.get('time_starttransfer')} 秒"
                if metrics.get("time_starttransfer")
                else "",
                f"{metrics.get('time_total')} 秒完成" if metrics.get("time_total") else "",
                f"{metrics.get('size_download')} bytes" if metrics.get("size_download") else "",
                *v4_metadata,
            )
            if value
        )
        output_parts = [str(request.get("responseBody") or "")]
        if request.get("stderr"):
            output_parts.append("curl stderr:\n" + str(request["stderr"]))
        output = "\n\n".join(part for part in output_parts if part)
        tool_contract_errors = request.get("tool_contract_errors_json")
        rendered_tool_contract_errors = ""
        if tool_contract_errors not in (None, "", "[]"):
            rendered_tool_contract_errors = (
                '<div class="tool-contract-errors">'
                "<h6>工具合同检查记录（tool_contract_errors_json）</h6>"
                f"<pre><code>{_e(tool_contract_errors)}</code></pre>"
                "</div>"
            )
        turns.append(
            '<section class="turn-evidence">'
            f'<div class="turn-head"><h5>第 {index} 次请求</h5><span>{_e(meta)}</span></div>'
            '<div class="evidence-columns">'
            '<div class="evidence-panel">'
            "<h6>发给模型的完整内容（原始 JSON）</h6>"
            f"<pre><code>{_e(request.get('requestBody'))}</code></pre></div>"
            '<div class="evidence-panel">'
            "<h6>模型返回的完整内容（原始 JSON）</h6>"
            f"<pre><code>{_e(output)}</code></pre></div>"
            "</div>"
            f"{rendered_tool_contract_errors}"
            "</section>"
        )
    return "".join(turns)


def _capabilities_section(
    tests: List[dict],
    core_ids: frozenset,
) -> str:
    items = [item for item in tests if isinstance(item, dict)]
    domains_with_tests = {display_domain(item) for item in items}
    ordered_domains = domain_order(list(domains_with_tests))

    total = len(items)
    failed = sum(1 for item in items if item.get("reviewedStatus") == "FAIL")
    core_total = sum(1 for item in items if _tier_for_test(str(item.get("testId")), core_ids) == "core")
    enhanced_total = total - core_total

    domain_details = []
    for domain in ordered_domains:
        domain_items = sorted(
            (item for item in items if display_domain(item) == domain),
            key=lambda item: str(item.get("testId")),
        )
        domain_failed = sum(
            1 for item in domain_items if item.get("reviewedStatus") == "FAIL"
        )
        status_html = (
            f'<span class="domain-status fail">{domain_failed} 项未通过</span>'
            if domain_failed
            else '<span class="domain-status">全部通过</span>'
        )
        rows = "".join(
            _test_row(
                item,
                _tier_for_test(str(item.get("testId")), core_ids),
            )
            for item in domain_items
        )
        domain_details.append(
            '<details class="domain">'
            "<summary>"
            '<span class="domain-summary">'
            f'<span class="domain-name">{_e(domain)}</span>'
            f'<span class="domain-count">{len(domain_items)} 项</span>'
            f"{status_html}"
            "</span>"
            "</summary>"
            f'<div class="domain-tests">{rows}</div>'
            "</details>"
        )

    return (
        '<section class="report-section" id="capabilities" data-nav-section>'
        f"{_section_head(f'04 · {total} 项能力检查', '能力检查结果', CAPABILITY_SECTION_DESCRIPTION)}"
        '<div class="capability-toolbar">'
        '<input class="search-input" id="test-search" type="search" '
        'placeholder="搜索检测编号或名称" aria-label="搜索检测编号或名称">'
        '<div class="segmented" role="group" aria-label="检测项筛选">'
        f'<button class="filter-button is-active" type="button" data-filter="all">全部 {total}</button>'
        f'<button class="filter-button" type="button" data-filter="fail">未通过 {failed}</button>'
        f'<button class="filter-button" type="button" data-filter="core">关键检查 {core_total}</button>'
        f'<button class="filter-button" type="button" data-filter="enhanced">扩展检查 {enhanced_total}</button>'
        "</div></div>"
        f'<p class="visible-count" id="visible-count">当前显示 {total} 个检测项</p>'
        f'<div id="domain-list">{"".join(domain_details)}</div>'
        '<p class="empty-state" id="empty-state">没有符合当前条件的检测项。</p>'
        "</section>"
    )


def _sidebar(model: object) -> str:
    links = []
    options = []
    for index, (section_id, label) in enumerate(NAV_SECTIONS, start=1):
        number = f"{index:02d}"
        active = ' class="is-active"' if index == 1 else ""
        links.append(
            f'<a href="#{section_id}"{active}><span class="nav-number">{number}</span><span>{_e(label)}</span></a>'
        )
        options.append(f'<option value="{section_id}">{number} {_e(label)}</option>')
    return (
        '<div class="mobile-nav">'
        '<select id="mobile-section-nav" aria-label="跳转到报告章节">'
        f'{"".join(options)}'
        "</select></div>"
        '<div class="workspace">'
        '<aside class="sidebar" aria-label="报告目录">'
        '<div class="sidebar-head">'
        "<strong>报告目录</strong>"
        f"<span>{_e(model)}</span>"
        "</div>"
        f'<nav class="sidebar-nav">{"".join(links)}</nav>'
        "</aside>"
    )


def render_report(assessment: dict, asset_dir: Path) -> str:
    """Return one self-contained report without mutating the assessment."""

    errors = validate_assessment(assessment)
    if errors:
        raise ValueError("Invalid assessment:\n" + "\n".join(errors))
    asset_dir = Path(asset_dir)
    css = (asset_dir / "report.css").read_text(encoding="utf-8")
    script = (asset_dir / "report.js").read_text(encoding="utf-8")
    run = assessment.get("run", {})
    model = run.get("model", "未知模型")
    tests = assessment.get("tests", [])
    capability_summary = assessment.get("capabilitySummary", {})

    try:
        contract = contract_key(run)
    except ValueError:
        contract = V4_CONTRACT
    core_ids, _enhanced_ids = CONTRACT_VERDICT_PARTITIONS[contract]

    started_at = run.get("started_at") or "未记录"

    return f"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="referrer" content="no-referrer">
  <meta http-equiv="Content-Security-Policy" content="{_e(CSP)}">
  <title>{_e(model)} · {REPORT_TITLE}</title>
  <style>{css}</style>
</head>
<body>
{_sidebar(model)}
<main class="report" aria-label="{REPORT_TITLE}">
  <header class="report-header">
    <p class="eyebrow">Model Capability Assessment</p>
    <h1 class="report-title">{REPORT_TITLE}</h1>
    <p class="report-model">{_e(model)}</p>
    <p class="report-time">检测时间：{_e(started_at)}</p>
  </header>

  {_capability_summary(assessment)}

  {_issues_section(capability_summary, tests)}

  {_run_metadata(run, capability_summary.get('verifiedFacts', {}), assessment.get('source', {}), tests)}

  {_capabilities_section(tests, core_ids)}
</main>
</div>
<script>{script}</script>
</body>
</html>
"""
