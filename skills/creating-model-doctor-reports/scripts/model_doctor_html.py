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
    "FAIL": "失败",
    "UNDETERMINED": "无法判定",
    "UNSUPPORTED": "不支持",
    "SKIPPED": "跳过",
    "ERROR": "执行错误",
}
CATEGORY_STATUS_LABELS = {
    "PASS": "通过",
    "FAIL": "未通过",
    "CONDITIONAL": "需复测",
}


def _e(value: object) -> str:
    return escape("" if value is None else str(value), quote=True)


def _list(items: Iterable[object], empty: str = "无") -> str:
    values = list(items or [])
    if not values:
        return f'<p class="muted">{_e(empty)}</p>'
    return "<ul>" + "".join(f"<li>{_e(item)}</li>" for item in values) + "</ul>"


def _status_text(status: str) -> str:
    label = STATUS_LABELS.get(status, status)
    return f'<span class="status status-{_e(status)}">{_e(label)}</span>'


def _category_key_data(category: dict) -> str:
    counts = category.get("counts", {})
    total = sum(int(value) for value in counts.values())
    parts = [f"{int(counts.get('PASS', 0))}/{total} 通过"]
    if category.get("criticalFailures"):
        parts.append("硬门禁失败：" + ", ".join(category["criticalFailures"]))
    if category.get("unknowns"):
        parts.append("待补证：" + ", ".join(category["unknowns"]))
    return "；".join(parts)


def _category_conclusion(category: dict) -> str:
    status = category.get("status")
    if status == "PASS":
        return "本次证据未发现该能力域的已确认问题。"
    if category.get("criticalFailures"):
        return "存在硬门禁失败，需先处理对应检测项。"
    return "存在失败、错误或证据不足项，请查看逐项检测证据。"


def _category_rows(categories: List[dict]) -> str:
    rows = []
    for category in categories:
        status = category.get("status", "CONDITIONAL")
        rows.append(
            "<tr>"
            f'<th scope="row" data-label="能力域">{_e(category.get("name"))}</th>'
            f'<td data-label="状态"><span class="category-status category-status-{_e(status)}">'
            f'{_e(CATEGORY_STATUS_LABELS.get(status, status))}</span></td>'
            f'<td data-label="关键数据">{_e(_category_key_data(category))}</td>'
            f'<td data-label="最终结论">{_e(_category_conclusion(category))}</td>'
            "</tr>"
        )
    return "".join(rows)


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
                f"HTTP {metrics.get('http_status')}" if metrics.get("http_status") else "",
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
    reviewed = item.get("reviewedStatus", "UNDETERMINED")
    discrepancy = bool(item.get("discrepancy", False))
    discrepancy_note = (
        '<span class="discrepancy-note">判定发生变化</span>' if discrepancy else ""
    )
    return (
        f'<tbody class="result-group" data-status="{_e(reviewed)}">'
        f'<tr class="result-row" data-detail-id="{_e(detail_id)}">'
        f'<th scope="row" data-label="编号">{_e(test_id)}</th>'
        '<td data-label="检测项">'
        f'<span class="category">{_e(item.get("category"))}</span>'
        f'{_e(item.get("name"))}</td>'
        '<td data-label="检测结果">'
        f'{_status_text(reviewed)}'
        f'<span class="detail">{_e(item.get("rawObservation"))}</span></td>'
        '<td data-label="检测结论"><div class="conclusion-layout">'
        f'<span>{_e(item.get("conclusion"))}{discrepancy_note}</span>'
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
        f'{_request_evidence(item.get("requests", []))}'
        "</div></td></tr></tbody>"
    )


def _test_rows(items: List[dict]) -> str:
    return "".join(_test_row_group(item) for item in items)


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

    return f"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="referrer" content="no-referrer">
  <meta http-equiv="Content-Security-Policy" content="{_e(CSP)}">
  <title>{_e(model)} · Model Doctor 客户模型就绪度报告</title>
  <style>{css}</style>
</head>
<body>
<main class="report-shell" aria-label="{_e(model)} 模型能力检测结论与逐项结果">
  <table class="summary-table">
    <caption>{_e(model)} · {_e(protocol)} 能力域总结</caption>
    <colgroup><col><col><col><col></colgroup>
    <thead><tr><th>能力域</th><th>状态</th><th>关键数据</th><th>最终结论</th></tr></thead>
    <tbody>{_category_rows(assessment.get('categories', []))}</tbody>
  </table>

  <table class="results-table">
    <caption>{_e(model)} 逐项检测结果</caption>
    <colgroup><col><col><col><col></colgroup>
    <thead><tr><th>编号</th><th>检测项</th><th>检测结果</th><th>检测结论</th></tr></thead>
    {_test_rows(assessment.get('tests', []))}
  </table>
</main>
<script>{script}</script>
</body>
</html>
"""
