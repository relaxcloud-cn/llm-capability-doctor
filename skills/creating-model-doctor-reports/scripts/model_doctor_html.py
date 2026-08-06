#!/usr/bin/env python3
"""Render a validated Model Doctor assessment as offline customer HTML."""

from __future__ import annotations

from html import escape
from pathlib import Path
from typing import Iterable, List

from model_doctor_assessment import validate_assessment


CSP = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
STATUS_LABELS = {
    "PASS": "通过",
    "FAIL": "未通过",
}
SUFFICIENCY_LABELS = {
    "SUFFICIENT": "证据充分",
    "LIMITED": "证据有限",
    "INSUFFICIENT": "证据不足",
}
PROTOCOL_FAMILY_LABELS = {
    "OPENAI_CHAT_COMPLETIONS": "OpenAI Chat Completions",
    "OPENAI_RESPONSES": "OpenAI Responses",
    "ANTHROPIC_MESSAGES": "Anthropic Messages",
    "GEMINI_GENERATE_CONTENT": "Gemini GenerateContent",
    "OLLAMA_CHAT": "Ollama Chat",
    "CUSTOM": "自定义格式",
    "UNKNOWN": "未确认",
}


def _e(value: object) -> str:
    return escape("" if value is None else str(value), quote=True)


def _list(items: Iterable[object], empty: str = "无") -> str:
    values = list(items or [])
    if not values:
        return f'<p class="muted">{_e(empty)}</p>'
    return "<ul>" + "".join(f"<li>{_e(item)}</li>" for item in values) + "</ul>"


def _optional_detail_list(title: str, items: Iterable[object]) -> str:
    values = list(items or [])
    if not values:
        return ""
    return (
        '<section class="logic-item">'
        f'<h4>{_e(title)}</h4>{_list(values)}'
        "</section>"
    )


def _status_text(status: str) -> str:
    label = STATUS_LABELS.get(status, status)
    return f'<span class="status status-{_e(status)}">{_e(label)}</span>'


def _category_rows(categories: List[dict]) -> str:
    rows = []
    for category in categories:
        counts = category.get("counts", {})
        passed = int(counts.get("PASS", 0))
        failed = int(counts.get("FAIL", 0))
        rows.append(
            "<tr>"
            f'<th scope="row" data-label="能力域">{_e(category.get("name"))}</th>'
            f'<td data-label="通过">{passed}</td>'
            f'<td data-label="未通过">{failed}</td>'
            f'<td data-label="总数">{passed + failed}</td>'
            "</tr>"
        )
    return "".join(rows)


def _run_metadata(run: dict, summary: dict) -> str:
    counts = summary.get("counts", {})
    passed = int(counts.get("PASS", 0))
    failed = int(counts.get("FAIL", 0))
    fields = (
        ("检测 URL", run.get("url") or "未知"),
        ("模型名称", run.get("model") or "未知"),
        ("API Key", run.get("api_key") or "未知"),
        ("检测项总数", passed + failed),
        ("通过项", passed),
        ("未通过项", failed),
    )
    rows = "".join(
        f"<div><dt>{_e(label)}</dt><dd>{_e(value)}</dd></div>"
        for label, value in fields
    )
    return (
        '<section class="run-information" aria-labelledby="run-information-heading">'
        '<h1 id="run-information-heading" class="run-information-heading">检测信息</h1>'
        f'<dl class="run-metadata">{rows}</dl>'
        "</section>"
    )


def _verified_facts(facts: dict) -> str:
    facts = facts if isinstance(facts, dict) else {}
    interface = facts.get("interfaceProtocol")
    interface = interface if isinstance(interface, dict) else {}
    context = facts.get("contextWindow")
    context = context if isinstance(context, dict) else {}
    concurrency = facts.get("concurrency")
    concurrency = concurrency if isinstance(concurrency, dict) else {}

    family = interface.get("family")
    family_label = (
        PROTOCOL_FAMILY_LABELS.get(family, family)
        if isinstance(family, str)
        else "未确认"
    )
    interface_detail = (
        f'请求：{interface.get("requestFormat") or ""}；'
        f'响应：{interface.get("responseFormat") or ""}；'
        f"分类：{family_label}。"
    )

    highest_tier = context.get("highestVerifiedTier") or "未确认"
    highest_tokens = context.get("highestVerifiedInputTokens")
    highest_tokens = highest_tokens if highest_tokens is not None else "未观察到"
    failed_tier = context.get("firstFailedTier") or "未观察到"
    failed_tokens = context.get("firstFailedInputTokens")
    failed_tokens = failed_tokens if failed_tokens is not None else "未观察到"
    context_detail = (
        f"最高通过档：{highest_tier}；原生输入 Token：{highest_tokens}；"
        f"首个失败档：{failed_tier}；失败档输入 Token：{failed_tokens}。"
    )

    highest_concurrency = concurrency.get("highestVerifiedConcurrentRequests")
    highest_concurrency = (
        highest_concurrency if highest_concurrency is not None else "未确认"
    )
    concurrency_detail = f"最高已验证并发：{highest_concurrency}。"

    rows = (
        (
            "接口协议格式",
            interface.get("statement"),
            interface_detail,
            interface.get("boundary"),
        ),
        (
            "上下文能力",
            context.get("statement"),
            context_detail,
            context.get("boundary"),
        ),
        (
            "并发能力",
            concurrency.get("statement"),
            concurrency_detail,
            concurrency.get("boundary"),
        ),
    )
    rendered_rows = "".join(
        "<div>"
        f"<dt>{_e(label)}</dt>"
        f"<dd><strong>{_e(statement)}</strong>"
        f'<span class="verified-fact-detail">{_e(detail)}</span>'
        f'<span class="verified-fact-boundary">{_e(boundary)}</span>'
        "</dd></div>"
        for label, statement, detail, boundary in rows
    )
    return f'<dl class="verified-facts">{rendered_rows}</dl>'


def _capability_summary(summary: dict) -> str:
    issues = summary.get("issues", [])
    issue_list = ""
    if issues:
        rendered_issues = []
        for issue in issues:
            test_refs = "、".join(str(value) for value in issue.get("testRefs", []))
            rendered_issues.append(
                "<li>"
                f'<p><strong>{_e(issue.get("title"))}：</strong>'
                f'{_e(issue.get("statement"))}</p>'
                f'<p class="final-conclusion-boundary">{_e(issue.get("boundary"))}</p>'
                f'<p class="final-conclusion-refs">关联检测项：{_e(test_refs)}</p>'
                "</li>"
            )
        issue_list = (
            '<ol class="final-conclusion-list">'
            + "".join(rendered_issues)
            + "</ol>"
        )
    return (
        '<section class="final-conclusion" aria-labelledby="final-conclusion-heading">'
        '<h2 id="final-conclusion-heading" class="final-conclusion-heading">'
        "最终结论</h2>"
        f'{_verified_facts(summary.get("verifiedFacts", {}))}'
        f'<p class="final-conclusion-lead">{_e(summary.get("headline"))}</p>'
        f"{issue_list}"
        f'<p class="final-conclusion-scope">{_e(summary.get("scopeBoundary"))}</p>'
        "</section>"
    )


def _failure_analysis(item: dict) -> str:
    if item.get("reviewedStatus") != "FAIL":
        return ""
    analysis = item.get("failureAnalysis", {})
    sufficiency = SUFFICIENCY_LABELS.get(
        analysis.get("evidenceSufficiency"),
        analysis.get("evidenceSufficiency"),
    )
    rows = [
        ("失败类型", analysis.get("failureKind")),
        ("证据充分性", sufficiency),
        ("证据支持", analysis.get("supportedClaim")),
    ]
    unsupported = analysis.get("unsupportedClaims", [])
    if unsupported:
        rows.append(("不可扩大推断", "；".join(str(value) for value in unsupported)))
    dependencies = analysis.get("dependsOnTestIds", [])
    if dependencies:
        rows.append(("依赖检测项", "、".join(str(value) for value in dependencies)))
    evidence_refs = analysis.get("evidenceRefs", [])
    if evidence_refs:
        rows.append(("证据引用", "、".join(str(value) for value in evidence_refs)))
    details = "".join(
        f"<div><dt>{_e(label)}</dt><dd>{_e(value)}</dd></div>"
        for label, value in rows
    )
    return (
        '<section class="logic-item failure-analysis">'
        "<h4>未通过项证据复核</h4>"
        f'<dl class="failure-analysis-details">{details}</dl>'
        "</section>"
    )


def _request_evidence(requests: List[dict]) -> str:
    if not requests:
        return '<p class="empty-evidence">日志未包含可关联的完整请求块。</p>'
    turns = []
    for index, request in enumerate(requests, start=1):
        metrics = request.get("metrics", {})
        meta = " · ".join(
            value
            for value in (
                str(request.get("request_id") or f"Turn {index}"),
                f"curl exit {metrics.get('curl_exit_code')}"
                if metrics.get("curl_exit_code")
                else "",
                f"HTTP {metrics.get('http_status')}" if metrics.get("http_status") else "",
                f"TTFB {metrics.get('time_starttransfer')}s"
                if metrics.get("time_starttransfer")
                else "",
                f"{metrics.get('time_total')}s" if metrics.get("time_total") else "",
                f"{metrics.get('size_download')} bytes" if metrics.get("size_download") else "",
            )
            if value
        )
        output_parts = [str(request.get("responseBody") or "")]
        if request.get("stderr"):
            output_parts.append("curl stderr:\n" + str(request["stderr"]))
        output = "\n\n".join(part for part in output_parts if part)
        turns.append(
            '<section class="turn-evidence">'
            f'<h4>Turn {index}<span>{_e(meta)}</span></h4>'
            "<h5>请求输入</h5>"
            f"<pre><code>{_e(request.get('requestBody'))}</code></pre>"
            "<h5>请求输出</h5>"
            f"<pre><code>{_e(output)}</code></pre>"
            "</section>"
        )
    return "".join(turns)


def _test_row_group(item: dict) -> str:
    logic = item.get("logic", {})
    test_id = str(item.get("testId", "unknown"))
    detail_id = f"test-detail-{test_id}"
    reviewed = item.get("reviewedStatus", "FAIL")
    return (
        f'<tbody class="result-group" data-status="{_e(reviewed)}">'
        f'<tr class="result-row" data-detail-id="{_e(detail_id)}">'
        f'<th scope="row" data-label="编号">{_e(test_id)}</th>'
        '<td data-label="检测项"><div class="cell-content">'
        f'<span class="category">{_e(item.get("category"))}</span>'
        f'<span>{_e(item.get("name"))}</span></div></td>'
        '<td data-label="检测结果"><div class="cell-content">'
        f'{_status_text(reviewed)}'
        f'<span class="detail">{_e(item.get("rawObservation"))}</span></div></td>'
        '<td data-label="检测结论"><div class="conclusion-layout">'
        f'<span>{_e(item.get("conclusion"))}</span>'
        f'<button type="button" class="row-toggle" aria-expanded="false" aria-controls="{_e(detail_id)}" '
        f'aria-label="展开检测项 {_e(test_id)}"><span aria-hidden="true">⌄</span></button>'
        "</div></td></tr>"
        f'<tr id="{_e(detail_id)}" class="evidence-row" hidden><td colspan="4">'
        '<div class="evidence-content">'
        '<section class="logic-item"><h4>检测目的</h4>'
        f'<p>{_e(logic.get("purpose"))}</p></section>'
        '<section class="logic-item"><h4>检测方法</h4>'
        f'<p>{_e(logic.get("method"))}</p></section>'
        '<section class="logic-item pass-criteria"><h4>通过条件</h4>'
        f'{_list(logic.get("passCriteria", []))}</section>'
        '<section class="logic-item fail-criteria"><h4>未通过条件</h4>'
        f'{_list(logic.get("failCriteria", []))}</section>'
        '<section class="logic-item"><h4>能力边界</h4>'
        f'<p>{_e(logic.get("capabilityBoundary"))}</p></section>'
        f'{_optional_detail_list("关键证据摘录", item.get("evidenceExcerpts", []))}'
        f'{_optional_detail_list("证据引用", item.get("evidenceRefs", []))}'
        f'{_optional_detail_list("限制", item.get("limitations", []))}'
        f'{_optional_detail_list("重跑建议", item.get("retestInstructions", []))}'
        f'{_failure_analysis(item)}'
        f'{_request_evidence(item.get("requests", []))}'
        "</div></td></tr></tbody>"
    )


def _test_rows(items: List[dict]) -> str:
    return "".join(_test_row_group(item) for item in items)


def _result_section(model: object, title: str, section_id: str, items: List[dict]) -> str:
    return (
        f'<section class="result-section" aria-labelledby="{_e(section_id)}">'
        f'<h2 id="{_e(section_id)}" class="results-heading">{_e(title)}</h2>'
        '<table class="results-table">'
        f'<caption>{_e(model)} {_e(title)}</caption>'
        '<colgroup><col><col><col><col></colgroup>'
        '<thead><tr><th>编号</th><th>检测项</th><th>检测结果</th><th>检测结论</th></tr></thead>'
        f'{_test_rows(items)}'
        '</table></section>'
    )


def _result_sections(model: object, categories: List[dict], tests: List[dict]) -> str:
    sections = []
    for index, category in enumerate(categories, start=1):
        name = str(category.get("name") or "未分类")
        items = [item for item in tests if item.get("category") == name]
        sections.append(
            _result_section(
                model,
                name,
                f"category-{index}-results-heading",
                items,
            )
        )
    return "".join(sections)


def _observed_protocol(assessment: dict) -> str:
    run = assessment.get("run", {})
    declared = run.get("protocol") or run.get("detected_protocol")
    if declared:
        return str(declared)

    protocols = {
        str(request.get("protocol"))
        for test in assessment.get("tests", [])
        for request in test.get("requests", [])
        if request.get("protocol")
    }
    return ", ".join(sorted(protocols)) if protocols else "unknown"


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
    protocol = _observed_protocol(assessment)
    tests = assessment.get("tests", [])
    categories = assessment.get("categories", [])

    return f"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="referrer" content="no-referrer">
  <meta http-equiv="Content-Security-Policy" content="{_e(CSP)}">
  <title>{_e(model)} · Model Doctor 模型能力检测报告</title>
  <style>{css}</style>
</head>
<body>
<main class="report-shell" aria-label="{_e(model)} 模型能力检测结论与逐项结果">
  {_run_metadata(run, assessment.get('summary', {}))}

  {_capability_summary(assessment.get('capabilitySummary', {}))}

  <table class="summary-table">
    <caption>{_e(model)} · {_e(protocol)} 能力域总结</caption>
    <colgroup><col><col><col><col></colgroup>
    <thead><tr><th>能力域</th><th>通过</th><th>未通过</th><th>总数</th></tr></thead>
    <tbody>{_category_rows(categories)}</tbody>
  </table>

  {_result_sections(model, categories, tests)}
</main>
<script>{script}</script>
</body>
</html>
"""
