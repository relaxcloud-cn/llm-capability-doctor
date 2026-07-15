#!/usr/bin/env python3
"""Render a validated Model Doctor assessment as offline customer HTML."""

from __future__ import annotations

from html import escape
from pathlib import Path
from typing import Dict, Iterable, List

from model_doctor_assessment import STATUSES, validate_assessment


CSP = "default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
STATUS_ORDER = ["PASS", "FAIL", "UNDETERMINED", "UNSUPPORTED", "SKIPPED", "ERROR"]
STATUS_LABELS = {
    "PASS": "通过",
    "FAIL": "失败",
    "UNDETERMINED": "无法判定",
    "UNSUPPORTED": "不支持",
    "SKIPPED": "跳过",
    "ERROR": "执行错误",
}
GATE_LABELS = {"critical": "硬门禁", "important": "重要能力", "observation": "观察指标"}
CONFIDENCE_LABELS = {"high": "高置信", "medium": "中置信", "low": "低置信"}


def _e(value: object) -> str:
    return escape("" if value is None else str(value), quote=True)


def _status_badge(status: str) -> str:
    return f'<span class="status-badge status-{_e(status)}">{_e(status)}</span>'


def _list(items: Iterable[object], empty: str = "无") -> str:
    values = list(items or [])
    if not values:
        return f'<p class="muted">{_e(empty)}</p>'
    return "<ul>" + "".join(f"<li>{_e(item)}</li>" for item in values) + "</ul>"


def _verdict_statement(verdict: str) -> str:
    return {
        "READY": "当前证据表明所有硬门禁与重要能力均满足要求。",
        "CONDITIONAL": "当前没有已确认的硬门禁失败，但仍有重要能力需要复测或补证。",
        "BLOCKED": "当前存在硬门禁失败或错误，不建议直接用于 ClawOps-style Agent 工作。",
    }.get(verdict, "当前证据不足以形成就绪度结论。")


def _decision_items(items: List[dict], empty: str) -> str:
    if not items:
        return f'<p class="muted">{_e(empty)}</p>'
    rows = []
    for item in items:
        rows.append(
            "<li>"
            f"<div>{_status_badge(item.get('status', 'UNDETERMINED'))}</div>"
            f"<div><strong>{_e(item.get('testId'))} {_e(item.get('name'))}</strong>"
            f"<p>{_e(item.get('conclusion'))}</p></div>"
            "</li>"
        )
    return '<ul class="decision-list">' + "".join(rows) + "</ul>"


def _counts(overall: dict) -> str:
    counts = overall.get("counts", {})
    return "".join(
        '<div class="status-count">'
        f'<strong data-status-count="{status}">{int(counts.get(status, 0))}</strong>'
        f'<span>{_e(STATUS_LABELS[status])}</span>'
        "</div>"
        for status in STATUS_ORDER
    )


def _category_summary(categories: List[dict]) -> str:
    rows = []
    for category in categories:
        counts = category.get("counts", {})
        total = sum(int(value) for value in counts.values())
        passed = int(counts.get("PASS", 0))
        impact = []
        if category.get("criticalFailures"):
            impact.append("硬门禁失败：" + ", ".join(category["criticalFailures"]))
        if category.get("unknowns"):
            impact.append("待补证：" + ", ".join(category["unknowns"]))
        rows.append(
            "<tr>"
            f"<th scope='row'>{_e(category.get('name'))}</th>"
            f"<td>{_status_badge(category.get('status', 'UNDETERMINED'))}</td>"
            f"<td>{passed}/{total}</td>"
            f"<td>{_e('；'.join(impact) if impact else '未发现阻断项')}</td>"
            "</tr>"
        )
    return "".join(rows)


def _request_timeline(requests: List[dict]) -> str:
    if not requests:
        return '<p class="muted">日志未包含可关联的完整请求块。</p>'
    blocks = []
    for index, request in enumerate(requests, start=1):
        metrics = request.get("metrics", {})
        metric_text = " · ".join(
            part
            for part in (
                f"HTTP {metrics.get('http_status')}" if metrics.get("http_status") else "",
                f"{metrics.get('time_total')}s" if metrics.get("time_total") else "",
                f"{metrics.get('size_download')} bytes" if metrics.get("size_download") else "",
            )
            if part
        )
        blocks.append(
            '<section class="request-block">'
            '<div class="request-heading">'
            f"<h3>Turn {index} · {_e(request.get('request_id'))}</h3>"
            f'<span class="muted">{_e(metric_text)}</span>'
            "</div>"
            "<h3>请求输入</h3>"
            f"<pre><code>{_e(request.get('requestBody'))}</code></pre>"
            "<h3>模型输出</h3>"
            f"<pre><code>{_e(request.get('responseBody'))}</code></pre>"
            + (
                "<h3>curl stderr</h3>"
                f"<pre><code>{_e(request.get('stderr'))}</code></pre>"
                if request.get("stderr")
                else ""
            )
            + "</section>"
        )
    return "".join(blocks)


def _test_detail(item: dict) -> str:
    logic = item.get("logic", {})
    discrepancy = item.get("discrepancy", False)
    original = item.get("originalStatus", "UNDETERMINED")
    reviewed = item.get("reviewedStatus", "UNDETERMINED")
    return (
        f'<details class="test-item" data-status="{_e(reviewed)}" '
        f'data-gate="{_e(item.get("gateLevel"))}" data-discrepancy="{str(bool(discrepancy)).lower()}">'
        '<summary class="test-summary">'
        f'<span class="test-id">{_e(item.get("testId"))}</span>'
        f'<span class="test-name">{_e(item.get("name"))}</span>'
        f'<span class="test-observation">{_e(item.get("rawObservation"))}</span>'
        '<span class="test-result">'
        f'{_status_badge(reviewed)}'
        f'<span class="gate-badge">{_e(GATE_LABELS.get(item.get("gateLevel"), item.get("gateLevel")))}</span>'
        "</span>"
        '<span class="test-conclusion">'
        f'{_e(item.get("conclusion"))}'
        + ('<span class="discrepancy-note">判定发生变化</span>' if discrepancy else "")
        + "</span>"
        "</summary>"
        '<div class="test-detail"><div class="detail-grid">'
        '<section class="detail-block"><h3>检测目的</h3>'
        f'<p>{_e(logic.get("purpose"))}</p></section>'
        '<section class="detail-block"><h3>测试方法</h3>'
        f'<p>{_e(logic.get("method"))}</p></section>'
        '<section class="detail-block"><h3>通过条件</h3>'
        f'{_list(logic.get("passCriteria", []))}</section>'
        '<section class="detail-block"><h3>失败条件</h3>'
        f'{_list(logic.get("failCriteria", []))}</section>'
        '<section class="detail-block full"><h3>能力边界</h3>'
        f'<p>{_e(logic.get("capabilityBoundary"))}</p></section>'
        '<section class="detail-block full"><h3>复核结论</h3>'
        '<div class="judgment-grid">'
        '<div class="judgment-box"><strong>原始判断</strong>'
        f'<div>{_status_badge(original)}</div></div>'
        '<div class="judgment-box"><strong>Skill 复核</strong>'
        f'<div>{_status_badge(reviewed)} <span class="confidence-badge">{_e(CONFIDENCE_LABELS.get(item.get("confidence"), item.get("confidence")))}</span></div></div>'
        "</div>"
        f'<p>{_e(item.get("conclusion"))}</p></section>'
        '<section class="detail-block"><h3>判定证据</h3>'
        f'{_list(item.get("evidenceExcerpts", []))}</section>'
        '<section class="detail-block"><h3>证据引用</h3>'
        f'{_list(item.get("evidenceRefs", []))}</section>'
        '<section class="detail-block"><h3>限制</h3>'
        f'{_list(item.get("limitations", []))}</section>'
        '<section class="detail-block"><h3>复测建议</h3>'
        f'{_list(item.get("retestInstructions", []))}</section>'
        '<section class="detail-block full"><h3>完整输入输出</h3>'
        f'{_request_timeline(item.get("requests", []))}</section>'
        "</div></div></details>"
    )


def _test_groups(items: List[dict]) -> str:
    categories: Dict[str, List[dict]] = {}
    for item in items:
        categories.setdefault(item.get("category", "Unclassified"), []).append(item)
    sections = []
    for name, group in categories.items():
        pass_count = sum(1 for item in group if item.get("reviewedStatus") == "PASS")
        sections.append(
            f'<section class="category-section" data-category="{_e(name)}">'
            '<div class="category-heading">'
            f'<h2>{_e(name)}</h2><span class="category-meta">{pass_count}/{len(group)} 通过</span>'
            "</div>"
            + "".join(_test_detail(item) for item in group)
            + "</section>"
        )
    return "".join(sections)


def _filters() -> str:
    status_buttons = ['<button class="filter-button" data-filter="status" data-value="all" aria-pressed="true">全部</button>']
    status_buttons.extend(
        f'<button class="filter-button" data-filter="status" data-value="{status}" aria-pressed="false">{_e(STATUS_LABELS[status])}</button>'
        for status in STATUS_ORDER
    )
    return (
        '<div class="filter-bar" aria-label="检测项筛选">'
        '<div class="filter-group"><span class="filter-label">状态</span>'
        + "".join(status_buttons)
        + "</div>"
        '<div class="filter-group"><span class="filter-label">门禁</span>'
        '<button class="filter-button" data-filter="gate" data-value="all" aria-pressed="true">全部</button>'
        '<button class="filter-button" data-filter="gate" data-value="critical" aria-pressed="false">硬门禁</button>'
        '<button class="filter-button" data-filter="gate" data-value="important" aria-pressed="false">重要</button>'
        '<button class="filter-button" data-filter="gate" data-value="observation" aria-pressed="false">观察</button>'
        "</div>"
        '<div class="filter-group"><span class="filter-label">复核</span>'
        '<button class="filter-button" data-filter="discrepancy" data-value="all" aria-pressed="true">全部</button>'
        '<button class="filter-button" data-filter="discrepancy" data-value="true" aria-pressed="false">发生改判</button>'
        "</div>"
        '<button class="expand-button" data-action="expand-all">展开全部证据</button>'
        "</div>"
    )


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
    source = assessment.get("source", {})
    overall = assessment.get("overall", {})
    verdict = overall.get("verdict", "CONDITIONAL")
    warnings = assessment.get("warnings", [])

    return f"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="referrer" content="no-referrer">
  <meta http-equiv="Content-Security-Policy" content="{_e(CSP)}">
  <title>{_e(run.get('model', 'Model'))} · Model Doctor 客户模型就绪度报告</title>
  <style>{css}</style>
</head>
<body>
<main class="report-shell">
  <header class="report-header">
    <div>
      <h1>Model Doctor 客户模型就绪度报告</h1>
      <p class="subtitle">基于完整审计日志的黑盒能力复核</p>
    </div>
    <dl class="run-meta">
      <dt>模型</dt><dd>{_e(run.get('model', '未知'))}</dd>
      <dt>协议</dt><dd>{_e(_observed_protocol(assessment))}</dd>
      <dt>运行 ID</dt><dd>{_e(run.get('run_id', '未知'))}</dd>
      <dt>脚本版本</dt><dd>{_e(run.get('script_version', '未知'))}</dd>
    </dl>
  </header>

  <section class="report-section verdict-{_e(verdict)}" id="overall">
    <div class="verdict-layout">
      <div>
        <h2>总体结论</h2>
        <div class="verdict-mark"><span class="verdict-dot" aria-hidden="true"></span><span class="verdict-value">{_e(verdict)}</span></div>
        <p>{_e(_verdict_statement(verdict))}</p>
        <div class="status-counts">{_counts(overall)}</div>
      </div>
      <div>
        <h3>阻断项</h3>
        {_decision_items(overall.get('blockers', []), '未发现已确认的硬门禁失败。')}
        <h3>条件与复测项</h3>
        {_decision_items(overall.get('conditions', []), '当前没有必须补证的关键项目。')}
      </div>
    </div>
  </section>

  <section class="report-section" id="categories">
    <h2>能力域总结</h2>
    <table class="summary-table">
      <thead><tr><th>能力域</th><th>状态</th><th>通过数</th><th>就绪度影响</th></tr></thead>
      <tbody>{_category_summary(assessment.get('categories', []))}</tbody>
    </table>
  </section>

  <section class="report-section" id="methodology">
    <h2>检测方法</h2>
    <div class="method-grid">
      <section><h3>证据来源</h3><p>脚本使用客户提供的模型连接信息发送真实请求，并完整记录请求、响应、协议和运行指标。Skill 不重新调用被测模型。</p></section>
      <section><h3>复核方法</h3><p>Skill 对每个检测项分别核对输入设计、协议事实、模型输出、前后轮关联和通过条件。HTTP 成功不自动等于能力通过。</p></section>
      <section><h3>能力边界</h3><p>本报告证明的是本次端点黑盒证据，不证明上游商业模型身份，也不替代 Codex app-server、MCP、插件和审批链的产品内验收。</p></section>
    </div>
    {_list(warnings, '日志结构完整，未发现解析警告。')}
  </section>

  <section class="report-section" id="tests">
    <h2>逐项检测结果</h2>
    {_filters()}
    {_test_groups(assessment.get('tests', []))}
  </section>

  <section class="report-section" id="integrity">
    <h2>运行信息与完整性</h2>
    <dl class="integrity-grid">
      <div class="integrity-item"><dt>源日志</dt><dd>{_e(source.get('fileName'))}</dd></div>
      <div class="integrity-item"><dt>文件大小</dt><dd>{_e(source.get('size'))} bytes</dd></div>
      <div class="integrity-item"><dt>SHA-256</dt><dd>{_e(source.get('sha256'))}</dd></div>
      <div class="integrity-item"><dt>生成时间</dt><dd>{_e(assessment.get('generatedAt'))}</dd></div>
    </dl>
    <p class="distribution-warning">本报告包含测试提示词和模型原始输出。虽然已执行二次脱敏，向客户或第三方分发前仍需复核其中的业务数据。</p>
  </section>
</main>
<script>{script}</script>
</body>
</html>
"""
