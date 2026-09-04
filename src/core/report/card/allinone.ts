import type { JsonReport, ReportRunScope } from "../json";
import { normalizeJsonReport } from "../json";
import {
  AGENT_RUNTIME_CHECK_GROUPS,
  AGENT_RUNTIME_HARD_CHECK_COUNT,
  AGENT_RUNTIME_SOFT_CHECK_COUNT,
} from "./agent-runtime-checks";
import {
  classifyAllInOneItems,
  type ClassifiedReportItem,
  type SoftImpact,
} from "./allinone-classification";
import { ALL_IN_ONE_REPORT_STYLE } from "./allinone-style";
import { lineChartSvg } from "./bench";
import {
  catLabel,
  confTableRows,
  coverageStatus,
  endpointLabel,
  esc,
  fmtDuration,
  fmtTokens,
  shortModel,
  statusPill,
  tier,
} from "./shared";
import { THEME_BOOT, THEME_SCRIPT, themeSwitcherHtml } from "./theme";
import {
  translatePhase,
  translateSurface,
  translateTier,
} from "../../../i18n/zh-cn";

export interface CardHtmlOptions {
  label?: string;
  libraryHref?: string;
  baseline?: {
    label: string;
    regressions: string[];
    improvements: string[];
  };
}

const CONCLUSION_ITEM_NAMES: Record<string, string> = {
  "chat-finish-length": "输出达到上限时的结束标记",
  "chat-limits-max-tokens": "输出长度上限",
  "chat-tool-choice-none": "禁止工具调用",
  "chat-parallel-tools-off": "单工具调用限制",
};

const SURFACE_NAMES: Record<string, string> = {
  chat: "Chat Completions",
  responses: "Responses",
  messages: "Messages",
};

const REPORT_NAV_SCRIPT = `
(function () {
  var links = Array.from(document.querySelectorAll(".toc-link"));
  var sections = Array.from(document.querySelectorAll(".section[id]"));
  function setCurrent(id) {
    links.forEach(function (link) {
      var current = link.getAttribute("data-section") === id;
      link.classList.toggle("active", current);
      if (current) link.setAttribute("aria-current", "true");
      else link.removeAttribute("aria-current");
    });
  }
  function sync() {
    var line = Math.max(120, window.innerHeight * 0.2);
    var current = sections[0];
    sections.forEach(function (section) {
      if (section.getBoundingClientRect().top <= line) current = section;
    });
    if (current) setCurrent(current.id);
  }
  var frame;
  window.addEventListener("scroll", function () {
    if (frame) return;
    frame = requestAnimationFrame(function () {
      sync();
      frame = undefined;
    });
  });
  links.forEach(function (link) {
    link.addEventListener("click", function () {
      setCurrent(link.getAttribute("data-section"));
    });
  });
  sync();
})();
`;

function ranPhase(
  report: JsonReport,
  key: keyof ReportRunScope["phases"],
): boolean {
  return (report.run?.phases?.[key]?.status ?? "measured") !== "not-run";
}

function resultText(item: ClassifiedReportItem): string {
  return item.sample ? `${item.sample} 样本` : "";
}

function impactPill(impact: SoftImpact | undefined): string {
  const values: Record<SoftImpact, string> = {
    strong: "如果失败：影响明显",
    small: "如果失败：影响较小",
    record: "仅记录",
  };
  const value = impact ?? "record";
  return `<span class="impact-pill ${value}">${values[value]}</span>`;
}

function itemRows(items: ClassifiedReportItem[], soft = false): string {
  if (items.length === 0) {
    return `<tr><td colspan="${soft ? 5 : 4}" class="reason">本轮没有可展示的检测项。</td></tr>`;
  }
  return items
    .map(
      (item) => `<tr>
        <td><span class="source">${esc(item.sourceLabel)}</span></td>
        <td><span class="item-name">${esc(item.name)}</span><code class="item-id">${esc(item.id)}</code></td>
        <td>${statusPill(item.outcome)}${resultText(item) ? `<span class="detail">${esc(resultText(item))}</span>` : ""}</td>
        ${soft ? `<td>${impactPill(item.impact)}</td>` : ""}
        <td class="reason">${esc(item.reason)}${item.detail ? `<span class="detail">${esc(item.detail)}</span>` : ""}</td>
      </tr>`,
    )
    .join("");
}

function itemTable(items: ClassifiedReportItem[], soft = false): string {
  return `<div class="table-wrap">
    <table class="report-table ${soft ? "soft-table" : ""}">
      <thead><tr>
        <th>检测来源</th><th>检测小项</th><th>本次结果</th>
        ${soft ? "<th>如果失败</th>" : ""}
        <th>${soft ? "为什么列为柔性项" : "为什么列为刚性项"}</th>
      </tr></thead>
      <tbody>${itemRows(items, soft)}</tbody>
    </table>
  </div>`;
}

function agentRuntimeGroups(): string {
  return AGENT_RUNTIME_CHECK_GROUPS.map((group) => {
    const rows = group.checks
      .map(
        (check) => `<tr>
          <td>${esc(check.id)}</td>
          <td>${esc(check.name)}</td>
          <td>${esc(check.passCriteria)}</td>
          <td><span class="status-pill not-run">待执行</span></td>
        </tr>`,
      )
      .join("");
    return `<div class="agent-group">
      <div class="agent-group-head">
        <div>
          <small>${esc(group.subtitle)} · ${group.checks.length} 项${group.kind === "hard" ? "刚性检查" : "柔性指标"}</small>
          <h3>${esc(group.title)}</h3>
        </div>
        <span class="status-pill not-run">待执行</span>
      </div>
      <div class="table-wrap">
        <table class="report-table agent-table" aria-label="${esc(group.title)}">
          <thead><tr><th>编号</th><th>${group.kind === "hard" ? "Pi Agent 检测内容" : "记录内容"}</th><th>${group.kind === "hard" ? "通过标准" : "怎么理解"}</th><th>状态</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>
    </div>`;
  }).join("");
}

function coverageDetails(report: JsonReport): string {
  const rows = (report.coverage?.entries ?? [])
    .map(
      (entry) => `<tr>
        <td>${esc(entry.label ?? entry.id)}<code class="item-id">${esc(entry.id)}</code></td>
        <td>${esc(translateTier(entry.tier))}</td>
        <td>${statusPill(coverageStatus(entry))}</td>
        <td class="reason">${esc(entry.detail ?? (entry.supported ? "已检测到" : "未支持"))}</td>
      </tr>`,
    )
    .join("");
  return `<section class="technical-section" id="coverage">
    <h3>能力覆盖汇总</h3>
    <p>只用于了解接口和功能范围，不与实际测试重复计数。</p>
    <div class="table-wrap"><table class="report-table"><thead><tr><th>接口或功能</th><th>层级</th><th>状态</th><th>说明</th></tr></thead><tbody>${rows}</tbody></table></div>
  </section>`;
}

function conformanceDetails(report: JsonReport): string {
  if (!ranPhase(report, "conformance")) return "";
  const rows = confTableRows(report)
    .map(
      (row) => `<tr>
        <td>${esc(row.test)}<code class="item-id">${esc(row.id)}</code></td>
        <td>${esc(translateSurface(row.surface))}</td>
        <td>${statusPill(row.status)}</td>
        <td class="reason">${esc(row.assertion)}${row.evidence ? `<span class="detail">${esc(row.evidence)}</span>` : ""}</td>
      </tr>`,
    )
    .join("");
  return `<section class="technical-section" id="conformance">
    <h3>协议检测明细</h3>
    <p>${report.conformance.passed}/${report.conformance.total} 条已执行规则通过。</p>
    <div class="table-wrap"><table class="report-table"><thead><tr><th>测试</th><th>接口</th><th>状态</th><th>规则与证据</th></tr></thead><tbody>${rows}</tbody></table></div>
  </section>`;
}

function capabilityDetails(report: JsonReport): string {
  if (!ranPhase(report, "capability")) return "";
  const rows = (report.capability?.evals ?? [])
    .map(
      (evaluation) => `<tr>
        <td>${esc(evaluation.name ?? evaluation.id)}<code class="item-id">${esc(evaluation.id)}</code></td>
        <td>${esc(evaluation.category)}</td>
        <td>${statusPill(evaluation.passed >= evaluation.total ? "pass" : "fail")}</td>
        <td class="reason">${evaluation.passed}/${evaluation.total} 样本通过${evaluation.failures?.length ? `<span class="detail">${esc(evaluation.failures.join("；"))}</span>` : ""}</td>
      </tr>`,
    )
    .join("");
  const unmeasured = report.capability.unmeasured?.length
    ? `<p>未检测：${report.capability.unmeasured.map((category) => esc(catLabel(category))).join("、")}。</p>`
    : "";
  return `<section class="technical-section" id="capability">
    <h3>模型行为任务</h3>
    <p>保留 llmprobe 的原始行为测试结果。</p>
    ${unmeasured}
    <div class="table-wrap"><table class="report-table"><thead><tr><th>任务</th><th>类别</th><th>状态</th><th>样本</th></tr></thead><tbody>${rows}</tbody></table></div>
  </section>`;
}

function fidelityDetails(report: JsonReport): string {
  if (!ranPhase(report, "fidelity") || !report.fidelity) return "";
  const rows = report.fidelity.slices
    .map(
      (slice) =>
        `<tr><td>${esc(slice.label)}</td><td>${slice.measured ? statusPill("pass") : statusPill("not-run")}</td><td>${slice.measured ? `${Math.round(slice.score * 10000) / 100}%` : "—"}</td><td class="reason">${esc(slice.detail ?? slice.unmeasuredReason ?? "")}</td></tr>`,
    )
    .join("");
  return `<section class="technical-section"><h3>引擎保真度</h3><div class="table-wrap"><table class="report-table"><thead><tr><th>项目</th><th>状态</th><th>得分</th><th>说明</th></tr></thead><tbody>${rows}</tbody></table></div></section>`;
}

function performanceDetails(report: JsonReport): string {
  if (!report.bench) return "";
  const bench = report.bench;
  const summaryRows: Array<[string, string, string]> = [
    [
      "生成速度",
      bench.decodeTokPerSec
        ? `${Math.round(bench.decodeTokPerSec.median * 10) / 10} Token/秒`
        : "—",
      "同一环境下采样的中位数",
    ],
    [
      "首 Token 延迟",
      bench.ttftMs ? `${Math.round(bench.ttftMs.median)} 毫秒` : "—",
      "只适合相同硬件和网络条件下比较",
    ],
    [
      "输入处理速度",
      bench.prefillTokPerSec
        ? `${Math.round(bench.prefillTokPerSec.median)} Token/秒`
        : "—",
      "提示词预处理速度",
    ],
    [
      "测试机器",
      `${bench.machine.cpu} · ${bench.machine.memGB} GB · ${bench.machine.platform} ${bench.machine.arch}`,
      "性能结果与机器和网络有关",
    ],
    [
      "前缀缓存",
      bench.prefixCache
        ? `${bench.prefixCache.verdict} · ${bench.prefixCache.speedup}x`
        : "—",
      "重复提示词的缓存效果",
    ],
    [
      "并发",
      bench.batching
        ? `${bench.batching.streams} 路 · ${bench.batching.aggregateTokPerSec ?? "—"} Token/秒`
        : "—",
      "多请求同时执行的表现",
    ],
    [
      "持续负载",
      bench.loadDrift
        ? `${bench.loadDrift.verdict} · ${bench.loadDrift.driftPct ?? "—"}%`
        : "—",
      "长时间运行时的速度变化",
    ],
  ];
  const contextRows: Array<[string, string, string]> = (
    bench.contextScaling ?? []
  ).map((point) => [
    `上下文 ${point.targetTokens}`,
    point.note
      ? point.note
      : `${point.inputTokens ?? point.targetTokens} Token · ${point.decodeTokPerSec ?? "—"} Token/秒`,
    point.note ? "该档位未完成" : "该档位已完成",
  ]);
  const rows = [...summaryRows, ...contextRows]
    .map(
      ([name, value, note]) =>
        `<tr><td>${esc(name!)}</td><td>${esc(value!)}</td><td class="reason">${esc(note!)}</td></tr>`,
    )
    .join("");
  const measuredContext = (bench.contextScaling ?? []).filter(
    (point) => !point.note && point.decodeTokPerSec != null,
  );
  const chart =
    measuredContext.length >= 2
      ? `<div class="ctx-charts">${lineChartSvg(
          "生成速度与上下文",
          "Token/秒",
          [
            {
              label: "生成速度",
              color: "var(--blue)",
              points: measuredContext.map((point) => ({
                x: point.inputTokens ?? point.targetTokens,
                y: point.decodeTokPerSec!,
              })),
            },
          ],
        )}</div>`
      : "";
  return `<section class="technical-section" id="performance"><h3>性能数据</h3><p>只作为柔性指标记录。</p>${chart}<div class="table-wrap"><table class="report-table"><thead><tr><th>项目</th><th>结果</th><th>说明</th></tr></thead><tbody>${rows}</tbody></table></div></section>`;
}

function runScopeNote(report: JsonReport): string {
  const phases = report.run?.phases;
  if (!phases) return "";
  const modeNote =
    report.run?.mode === "bench-only"
      ? "本轮只执行性能测试。"
      : report.run?.mode === "eval-only"
        ? "本轮只执行模型行为测试。"
        : "";
  const notRun = (
    Object.entries(phases) as Array<
      [keyof ReportRunScope["phases"], { status: string; reason?: string }]
    >
  ).filter(([, phase]) => phase.status === "not-run");
  return modeNote || notRun.length
    ? `<p class="scope-note">${modeNote}${notRun.length ? `本轮未执行：${notRun.map(([phase]) => esc(translatePhase(phase))).join("、")}。` : ""}</p>`
    : "";
}

export function renderCardHtml(
  reportInput: JsonReport,
  options: CardHtmlOptions = {},
): string {
  const report = normalizeJsonReport(reportInput);
  const classification = classifyAllInOneItems(report);
  const hardIssues = classification.hardIssues;
  const hardIncomplete = !classification.available || !classification.complete;
  const hardComplete = !hardIncomplete && hardIssues.length === 0;
  const primarySurface = classification.primarySurface
    ? (SURFACE_NAMES[classification.primarySurface] ??
      classification.primarySurface)
    : "未确定";
  const model = report.target?.model ?? "unknown";
  const core = tier(report, "core");
  const extended = tier(report, "extended");
  const issueNames = hardIssues
    .map((item) => CONCLUSION_ITEM_NAMES[item.id] ?? item.name)
    .join("、");

  const headline = hardIncomplete
    ? "本轮基础检测不完整"
    : hardComplete
      ? "可以进入 Agent Runtime 实测"
      : "暂不建议直接接入 Agent Runtime";
  const conclusion = hardIncomplete
    ? classification.available
      ? `还有 ${classification.missingHardCount} 项刚性要求没有检测结果，不能判断基础要求是否满足。`
      : "缺少协议检测结果，需要重新执行完整检测。"
    : hardComplete
      ? "刚性要求全部达到；最终结论仍需 Agent Runtime 真实任务验证。"
      : `当前有 ${hardIssues.length} 项刚性要求未达到：${issueNames}。应先确认模型配置或适配办法，再执行 Agent Runtime 真实任务。`;
  const overallStatus = hardIncomplete
    ? "需要重新检测"
    : hardComplete
      ? "等待 Agent 实测"
      : "暂不建议直接接入";
  const hardSummary = classification.available
    ? `${classification.hardPassed}/${classification.hard.length} 通过`
    : "未检测";
  const softAttention = classification.soft.filter(
    (item) => item.impact !== "record" && item.outcome !== "pass",
  ).length;
  const softSummary = `${classification.soft.length} 项，${softAttention} 项需关注`;
  const nav = options.libraryHref
    ? `<a class="btn" href="${esc(options.libraryHref)}">报告库</a>`
    : "";
  const footer = [
    report.usage
      ? `${fmtTokens(report.usage.inputTokens + report.usage.outputTokens)} Token`
      : null,
    fmtDuration(report.durationMs),
    options.label ?? null,
  ].filter(Boolean);

  return `<!doctype html>
<html lang="zh-CN" data-theme="light">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${esc(shortModel(model))} · All-in-One 模型能力诊断报告</title>
  <script>${THEME_BOOT}</script>
  <style>${ALL_IN_ONE_REPORT_STYLE}</style>
</head>
<body>
<main class="report-page">
  <header class="top">
    <div>
      <div class="brand">LLMPROBE · ALL-IN-ONE 模型能力诊断报告</div>
      <h1>${esc(model)}</h1>
      <div class="meta">
        ${report.target?.engine ? `<span>${esc(report.target.engine)}</span>` : ""}
        ${report.run?.startedAt ? `<span>${esc(report.run.startedAt.slice(0, 10))}</span>` : ""}
        ${report.target?.baseUrl ? `<span>${esc(endpointLabel(report.target.baseUrl))}</span>` : ""}
      </div>
      ${runScopeNote(report)}
    </div>
    <nav class="nav-links" aria-label="报告操作">${nav}${themeSwitcherHtml()}</nav>
  </header>

  <div class="report-layout">
    <aside class="toc">
      <div class="toc-title">报告目录</div>
      <div class="toc-note">点击问题，跳到对应章节</div>
      <nav aria-label="报告目录">
        <a class="toc-link active" href="#conclusion" data-section="conclusion" aria-current="true"><span class="toc-number">结论</span><span class="toc-question">总体结论</span><span class="toc-status">${esc(overallStatus)}</span></a>
        <a class="toc-link" href="#hard" data-section="hard"><span class="toc-number">01</span><span class="toc-question">刚性项</span><span class="toc-status">${esc(hardSummary)}</span></a>
        <a class="toc-link" href="#soft" data-section="soft"><span class="toc-number">02</span><span class="toc-question">柔性项</span><span class="toc-status">${esc(softSummary)}</span></a>
        <a class="toc-link" href="#agent-runtime" data-section="agent-runtime"><span class="toc-number">03</span><span class="toc-question">Agent Runtime 实测</span><span class="toc-status">2 类任务 · 27 项待执行</span></a>
      </nav>
    </aside>

    <div class="report-content">
      <section class="section" id="conclusion">
        <div class="section-head"><div class="qno">结论</div><div><h2>总体结论</h2><p class="lede">先说明现在能确认什么，再说明最终判断还缺什么。</p></div><div class="answer"><strong>${esc(hardIncomplete ? "基础检测不完整" : hardComplete ? "刚性要求全部达到" : "刚性要求未全部达到")}</strong><span>${hardIncomplete ? (classification.available ? `${classification.missingHardCount} 项没有结果` : "缺少必要结果") : `${hardIssues.length} 项需要处理`}</span></div></div>
        <div class="verdict-overview">
          <div class="verdict-copy"><div class="verdict-kicker">当前可确认</div><strong class="${hardComplete ? "good" : "bad"}">${esc(headline)}</strong><p>${esc(conclusion)}</p></div>
          <div class="verdict-status"><small>ALL-IN-ONE 最终结论</small><strong>${esc(overallStatus)}</strong><span>Agent Runtime 真实任务尚未执行，不能用 llmprobe 基础检测代替。</span></div>
        </div>
        <div class="decision-path">
          <article class="path-item"><div class="path-index">01</div><div class="path-copy"><strong>刚性项</strong><div class="path-result ${hardComplete ? "good-text" : "bad-text"}">${esc(hardSummary)}</div><span>${hardIncomplete ? (classification.available ? `${classification.missingHardCount} 项没有检测结果。` : "本轮没有协议检测结果。") : hardIssues.length ? `${hardIssues.length} 项未达到，需要先处理。` : "没有发现基础阻断项。"}</span></div></article>
          <article class="path-item"><div class="path-index">02</div><div class="path-copy"><strong>柔性项</strong><div class="path-result warn-text">${esc(softSummary)}</div><span>不会抵消刚性项失败。</span></div></article>
          <article class="path-item"><div class="path-index">03</div><div class="path-copy"><strong>Agent Runtime 实测</strong><div class="path-result pending-text">尚未执行</div><span>21 项刚性检查、6 项柔性指标。</span></div></article>
        </div>
        <p class="scope-note">刚性项按 All-in-One 必要任务判断；协议名称本身不是刚性项，只要存在一条可用路径即可。</p>
      </section>

      <section class="section" id="hard">
        <div class="section-head"><div class="qno">01</div><div><h2>刚性项：失败后任务无法正确、安全完成</h2><p class="lede">当前主要接入方式：${esc(primarySurface)}。</p></div><div class="answer"><strong>${esc(hardSummary)}</strong><span>${hardIncomplete ? `${classification.missingHardCount} 项没有结果` : `${hardIssues.length} 项未达到要求`}</span></div></div>
        <p class="inventory-note">${classification.hard.length} 个实际测试逐项展示；能力覆盖汇总不重复计数。</p>
        ${itemTable(classification.hard)}
      </section>

      <section class="section" id="soft">
        <div class="section-head"><div class="qno">02</div><div><h2>柔性项：影响效果、成本或非必要能力</h2><p class="lede">柔性项失败不能推翻刚性项结果，也不能补偿刚性项失败。</p></div><div class="answer"><strong>${esc(softSummary)}</strong><span>逐项保留影响说明</span></div></div>
        ${itemTable(classification.soft, true)}
        <div class="metric-grid">
          <article class="metric"><div class="score-label">柔性项 · 仅记录</div><strong>${report.conformance?.pct ?? 0}%</strong><p>协议规则通过率，不能单独判断能否接入。</p></article>
          <article class="metric"><div class="score-label">柔性项 · 仅记录</div><strong>${extended ? `${extended.pct}%` : "—"}</strong><p>扩展能力覆盖。</p></article>
          <article class="metric"><div class="score-label">柔性项 · 仅记录</div><strong>${report.bench?.decodeTokPerSec ? `${Math.round(report.bench.decodeTokPerSec.median * 10) / 10} Token/秒` : "—"}</strong><p>生成速度。</p></article>
          <article class="metric"><div class="score-label">柔性项 · 仅记录</div><strong>${report.bench?.ttftMs ? `${Math.round(report.bench.ttftMs.median)} 毫秒` : "—"}</strong><p>首 Token 延迟。</p></article>
          <article class="metric"><div class="score-label">能力覆盖汇总</div><strong>${core ? `${core.pct}%` : "—"}</strong><p>核心接口和功能覆盖，不重复计入检测项。</p></article>
        </div>
      </section>

      <section class="section" id="agent-runtime">
        <div class="section-head"><div class="qno">03</div><div><h2>进入 Agent 后能完成真实任务吗？</h2><p class="lede">在 Agent Runtime 下执行完整任务，看模型在智能体下的表现。</p></div><div class="answer"><strong>27 项待执行</strong><span>21 项刚性 · 6 项柔性</span></div></div>
        <div class="agent-overview"><div class="agent-status"><div class="score-label">执行载体：Pi Agent</div><strong>等待执行</strong><p>当前报告没有 Agent Runtime 运行轨迹，因此不对真实任务能力作出通过或失败判断。</p></div><div class="agent-counts"><div class="agent-count"><small>真实任务</small><strong>2 类</strong><span>初次研判、升级研判</span></div><div class="agent-count"><small>刚性检查</small><strong>${AGENT_RUNTIME_HARD_CHECK_COUNT} 项</strong><span>失败不能被性能指标抵消</span></div><div class="agent-count"><small>柔性指标</small><strong>${AGENT_RUNTIME_SOFT_CHECK_COUNT} 项</strong><span>记录效率和稳定性</span></div></div></div>
        ${agentRuntimeGroups()}
        <div class="agent-final-rule"><strong>任务通过规则</strong><p>必须读取决定性证据、得出与证据相符的结论、引用真实位置、遵守工具和权限边界、正确处理不确定性、提交合法报告，并在完成后停止。任何刚性项失败，任务都不能算通过。</p></div>
      </section>

      <details><summary>查看全部 llmprobe 技术明细</summary><div class="technical-body">${coverageDetails(report)}${conformanceDetails(report)}${capabilityDetails(report)}${fidelityDetails(report)}${performanceDetails(report)}</div></details>
      <details><summary>查看 Agent Runtime 运行明细</summary><div class="technical-body"><p class="scope-note">实测完成后，这里展示任务输入、工具调用、工具结果、证据引用、最终报告、耗时和 Token 使用情况。</p></div></details>

      <footer class="page-footer">${footer.map((value) => `<span>${esc(String(value))}</span>`).join("")}<span>未包含 llmprobe 模拟 Agent 功能</span></footer>
    </div>
  </div>
</main>
<script>${REPORT_NAV_SCRIPT}</script>
<script>${THEME_SCRIPT}</script>
</body>
</html>`;
}
