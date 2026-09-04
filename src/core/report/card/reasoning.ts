import type { ReasoningReport } from "../../../reasoning/types";
import { esc } from "./shared";

const STATUS_LABEL: Record<string, string> = {
  passed: "通过",
  failed: "错误",
  stopped: "Token 用尽",
  error: "错误",
};

export function reasoningCard(r: ReasoningReport): string {
  const pct = r.total > 0 ? Math.round((100 * r.passed) / r.total) : 0;
  return `<article class="card neutral">
      <div class="card-kicker">专项推理</div>
      <div class="card-value">${pct}%</div>
      <div class="card-sub">
        <span>${r.passed}/${r.total} 题正确</span>
        ${r.stopped ? `<span class="badge">${r.stopped} 题 Token 用尽</span>` : ""}
      </div>
      <div class="card-note">包含 GPQA Diamond、SuperGPQA、AIME 2025、COMPSEC 子集，仅供参考，不参与主评分。</div>
    </article>`;
}

export function reasoningSection(r: ReasoningReport): string {
  const rows = r.bySource
    .map(
      (s) => `<tr>
        <td>${esc(s.source)}</td>
        <td>${s.passed}/${s.total}</td>
        <td>${s.total ? Math.round((100 * s.passed) / s.total) : 0}%</td>
        <td>${s.stopped}</td>
        <td>${s.error}</td>
      </tr>`,
    )
    .join("\n");
  const cases = r.cases
    .map(
      (
        c,
      ) => `<tr class="${c.status === "passed" ? "good" : c.status === "stopped" ? "caution" : "bad"}">
        <td>${esc(c.source)}</td>
        <td>${esc(c.domain)}</td>
        <td>${esc(c.title)}</td>
        <td>${esc(STATUS_LABEL[c.status] ?? c.status)}</td>
        <td>${esc(c.got)}</td>
        <td>${esc(c.expected)}</td>
        <td>${c.outputTokens ?? "—"}</td>
      </tr>`,
    )
    .join("\n");
  return `    <section class="section" id="reasoning">
      <div class="section-head">
        <h2>专项推理 <span class="tag model">模型</span></h2>
        <div class="score">${r.passed}/${r.total}</div>
      </div>
      <p class="lede">高难题正确率，仅供参考，不参与主评分。每题最多生成 ${r.maxTokens} 个 Token，温度为 ${r.temperature}。“Token 用尽”表示没有生成最终答案行，这是预算问题，不等于答错。</p>
      ${r.scopeNote ? `<p class="fine">⚠ ${esc(r.scopeNote)}</p>` : ""}
      <div class="conf-table-wrap">
        <table class="drill-table">
          <thead><tr><th>题目来源</th><th>正确</th><th>正确率</th><th>Token 用尽</th><th>错误</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
      <details style="margin-top:12px">
        <summary class="hint-click">查看全部题目</summary>
        <div class="conf-table-wrap">
          <table class="drill-table">
            <thead><tr><th>题目来源</th><th>领域</th><th>题目</th><th>结果</th><th>实际答案</th><th>正确答案</th><th>输出 Token</th></tr></thead>
            <tbody>${cases}</tbody>
          </table>
        </div>
      </details>
    </section>`;
}
