import type { JsonReport, ReportRunScope } from "../json";
import { normalizeJsonReport } from "../json";
import { benchSection } from "./bench";
import { reasoningCard, reasoningSection } from "./reasoning";
import { CARD_STYLE } from "./style.css";
import { THEME_BOOT, THEME_SCRIPT, themeSwitcherHtml } from "./theme";
import { REPORT_SCRIPT } from "./report-script";
import {
  AGENTIC_FAILURE_GLOSS,
  CATEGORY_FLOOR_PCT,
  barFill,
  catLabel,
  confTableRows,
  coverageStatus,
  esc,
  embedJson,
  fmtDuration,
  fmtTokens,
  miniTiers,
  mustFailures,
  outcomeCounts,
  shortModel,
  statusPill,
  tier,
  toneForPct,
  verdictTone,
} from "./shared";
import {
  translateAgenticTaskName,
  translateDetail,
  translateFidelityLabel,
  translatePhase,
  translateSurface,
  translateTier,
  translateVerdict,
} from "../../../i18n/zh-cn";

const translatePhaseName = (phase: string): string => translatePhase(phase);

export interface CardHtmlOptions {
  /** Optional path/label for the source save file. */
  label?: string;
  /** When set, show ← Library linking here (e.g. "index.html"). */
  libraryHref?: string;
  baseline?: {
    label: string;
    regressions: string[];
    improvements: string[];
  };
}

function tierBlocks(report: JsonReport): string {
  const entries = report.coverage?.entries ?? [];
  return (report.coverage?.byTier ?? [])
    .map((t) => {
      const tone = toneForPct(t.pct);
      const tierEntries = entries.filter((e) => e.tier === t.tier);
      let rows = "";
      if (tierEntries.length > 0) {
        rows = tierEntries
          .slice()
          .sort((a, b) => {
            const rank = (e: (typeof tierEntries)[number]) =>
              coverageStatus(e) === "missing"
                ? 0
                : coverageStatus(e) === "not-probed"
                  ? 1
                  : 2;
            return (
              rank(a) - rank(b) ||
              (a.label || a.id).localeCompare(b.label || b.id)
            );
          })
          .map((e) => {
            const st = coverageStatus(e);
            return `<tr>
                  <td>${esc(e.label || e.id)}</td>
                  <td>${esc(e.kind === "surface" ? "接口" : e.kind === "feature" ? "功能" : e.kind || "—")}</td>
                  <td>${statusPill(st)}</td>
                  <td>${esc(e.detail || (st === "supported" ? "已提供" : st === "not-probed" ? "本轮未检测" : "不支持"))}</td>
                </tr>`;
          })
          .join("");
      } else {
        rows = [
          ...(t.missing || []).map(
            (m) =>
              `<tr><td>${esc(m)}</td><td>—</td><td>${statusPill("missing")}</td><td>listed as missing on tier summary</td></tr>`,
          ),
          ...(t.unprobed || []).map(
            (m) =>
              `<tr><td>${esc(m)}</td><td>—</td><td>${statusPill("not-probed")}</td><td>not probed at this depth</td></tr>`,
          ),
        ].join("");
        if (!rows) {
          rows = `<tr><td colspan="4" class="fine">当前报告没有更详细的能力数据。</td></tr>`;
        }
      }

      const missing =
        t.missing?.length > 0
          ? `<div class="missing">${t.missing.map((m) => `<span>✗ ${esc(m)}</span>`).join("")}</div>`
          : "";
      const unprobed =
        (t.unprobed?.length ?? 0) > 0
          ? `<div class="fine">本轮未检测：${t.unprobed!.map(esc).join(", ")}</div>`
          : "";

      return `<div class="tier-block">
        <button type="button" class="tier-toggle" data-tier="${esc(t.tier)}" aria-expanded="false" aria-controls="tier-panel-${esc(t.tier)}">
          <div class="row">
            <span class="row-label"><span class="chev">▸</span>${esc(translateTier(t.tier))}</span>
            <span class="row-ratio">${t.supported}/${t.total}</span>
            <span class="row-pct ${tone}">${t.pct}%</span>
            ${barFill(t.pct)}
          </div>
        </button>
        ${missing}${unprobed}
        <div class="expand-panel" id="tier-panel-${esc(t.tier)}" hidden>
          <table class="drill-table">
            <thead><tr><th>能力</th><th>类型</th><th>状态</th><th>说明</th></tr></thead>
            <tbody>${rows}</tbody>
          </table>
        </div>
      </div>`;
    })
    .join("");
}

/**
 * Self-contained intent-based report card HTML (themes, drill-downs, filters).
 * Replaces the older perspective-based product HTML.
 */
export function renderCardHtml(
  reportIn: JsonReport,
  options: CardHtmlOptions = {},
): string {
  const report = normalizeJsonReport(reportIn);
  const model = report.target?.model ?? "unknown";
  const engine = report.target?.engine ?? null;
  const baseUrl = report.target?.baseUrl ?? "";
  const core = tier(report, "core");
  const conf = report.conformance;
  const confMeasured = (conf?.total ?? 0) > 0;
  const cap = report.capability;
  const capMeasured = (cap?.categories?.length ?? 0) > 0;
  const agentic = report.agentic;
  const fidelity = report.fidelity;
  const must = mustFailures(report);
  const outcomes = outcomeCounts(report);
  const bench = report.bench;

  // What this run actually measured. A --bench-only or --quick run never
  // touched the scored phases, and rendering their empty cards reads as an
  // engine that failed everything rather than one nobody asked about.
  const phases = report.run?.phases;
  const ran = (key: keyof ReportRunScope["phases"]): boolean =>
    (phases?.[key]?.status ?? "measured") !== "not-run";
  const notRun = (
    ["conformance", "capability", "agentic", "fidelity"] as Array<
      keyof ReportRunScope["phases"]
    >
  ).filter((key) => !ran(key));
  const notRunReasons = [
    ...new Set(notRun.map((key) => phases?.[key]?.reason).filter(Boolean)),
  ] as string[];
  const scopeNote =
    notRun.length > 0
      ? `<p class="fine scope-note">本轮未执行：${notRun.map(translatePhaseName).join("、")}${
          notRunReasons.length > 0 ? ` — ${esc(notRunReasons.join("；"))}` : ""
        }。以下只展示本轮实际检测的内容。</p>`
      : "";

  const covTone = toneForPct(core?.pct);
  const confTone = confMeasured
    ? toneForPct(conf.pct, { perfect: true })
    : "neutral";
  const confToneStrict = confMeasured && conf.pct < 100 ? "critical" : confTone;
  const capTone = capMeasured ? verdictTone(cap.verdict) : "neutral";

  const coreHeadline = core ? `${core.pct}%` : "—";
  const confHeadline = confMeasured ? `${conf.pct}%` : "—";
  const capHeadline = capMeasured ? `${cap.pct}%` : "—";

  const nav = options.libraryHref
    ? `<a class="btn" href="${esc(options.libraryHref)}">← Library</a>`
    : "";

  const credits = (report.coverage?.credits ?? [])
    .map((c) => `<div class="fine">○ ${esc(c.label)} — 已检测到但不计分</div>`)
    .join("");

  const surfaces = (conf?.bySurface ?? [])
    .map((s) => {
      const t = toneForPct(s.pct);
      const color =
        t === "good"
          ? "var(--good)"
          : t === "critical"
            ? "var(--critical)"
            : t === "caution"
              ? "var(--caution)"
              : "var(--ink)";
      return `<button type="button" class="surface" data-surface-filter="${esc(s.surface)}" title="只查看 ${esc(translateSurface(s.surface))} 的检测">
        <div class="n" style="color:${color}">${s.pct}%</div>
        <div class="l">${esc(translateSurface(s.surface))}</div>
        <div class="r">${s.passed}/${s.total} 必须项</div>
      </button>`;
    })
    .join("");

  const evals = cap?.evals ?? [];
  const cats = (cap?.categories ?? [])
    .map((c) => {
      const weak = (cap.weakCategories ?? []).includes(c.category);
      const tone = weak ? "critical" : toneForPct(c.pct);
      const panelId = `cap-${c.category}`;
      const catEvals = evals
        .filter((e) => e.category === c.category)
        .slice()
        .sort((a, b) => {
          const aFail = a.passed < a.total ? 0 : 1;
          const bFail = b.passed < b.total ? 0 : 1;
          return (
            aFail - bFail || (a.name || a.id).localeCompare(b.name || b.id)
          );
        });
      const evalRows =
        catEvals.length > 0
          ? catEvals
              .map((e) => {
                const ok = e.passed >= e.total;
                const fails = (e.failures ?? [])
                  .map((f) => esc(typeof f === "string" ? f : String(f)))
                  .join("; ");
                return `<tr class="${ok ? "" : "fail-row"}">
                  <td>${esc(e.name || e.id)}</td>
                  <td>${e.passed}/${e.total}</td>
                  <td>${ok ? statusPill("pass") : statusPill("fail")}</td>
                  <td>${fails || (ok ? "—" : "sample failure")}</td>
                </tr>`;
              })
              .join("")
          : `<tr><td colspan="4" class="fine">当前报告没有 ${esc(catLabel(c.category))} 的详细数据。</td></tr>`;

      return `<div class="cat-block">
        <button type="button" class="cat-toggle" data-expand="${esc(panelId)}" aria-expanded="false" aria-controls="${esc(panelId)}">
          <div class="cat-row">
            <span class="row-label"><span class="chev">▸</span>${esc(catLabel(c.category))}</span>
            <span class="row-ratio">${c.passed}/${c.total}</span>
            <span class="row-pct ${tone}">${c.pct}%</span>
            <span class="floor-mark">${barFill(c.pct, "model")}</span>
          </div>
        </button>
        <div class="expand-panel" id="${esc(panelId)}" hidden>
          <table class="drill-table">
            <thead><tr><th>测试项</th><th>样本数</th><th>状态</th><th>失败说明</th></tr></thead>
            <tbody>${evalRows}</tbody>
          </table>
        </div>
      </div>`;
    })
    .join("");

  const weakNote =
    (cap?.weakCategories?.length ?? 0) > 0
      ? `<div class="missing">低于最低要求：${cap.weakCategories.map((c) => esc(catLabel(c))).join(", ")}</div>`
      : "";
  const unmeasNote =
    (cap?.unmeasured?.length ?? 0) > 0
      ? `<div class="fine">⚠ 未检测：${cap.unmeasured!.map((c) => esc(catLabel(c))).join(", ")} — 不视为通过</div>`
      : "";

  const tasks = agentic
    ? agentic.tasks
        .map((t) => {
          const icon = t.passed
            ? `<span class="icon ok">✓</span>`
            : `<span class="icon bad">✗</span>`;
          const chip =
            !t.passed && t.failure
              ? `<span class="chip">${esc(t.failure)}</span>`
              : "";
          const gloss =
            !t.passed && t.failure && AGENTIC_FAILURE_GLOSS[t.failure]
              ? AGENTIC_FAILURE_GLOSS[t.failure]
              : null;
          const detail = !t.passed
            ? `<div class="detail">→ ${esc([gloss, translateDetail(t.detail)].filter(Boolean).join(" — ") || "失败")}</div>`
            : "";
          return `<div class="task">
            ${icon}
            <div>
              <div class="name">${esc(translateAgenticTaskName(t.id, t.name))}${chip}</div>
            </div>
            <div class="steps">${t.steps} 步</div>
            ${detail}
          </div>`;
        })
        .join("")
    : `<p class="fine">本轮未执行 Agent 任务。</p>`;

  const fidSlices = fidelity
    ? fidelity.slices
        .map((s) => {
          const panelId = `fid-${s.id}`;
          const sp = s.measured ? Math.round(s.score * 10000) / 100 : null;
          const header = s.measured
            ? `<div class="row" style="grid-template-columns:minmax(140px,200px) 52px 1fr">
                <span class="row-label"><span class="chev">▸</span>${esc(translateFidelityLabel(s.label))}</span>
                <span class="row-pct ${toneForPct(sp)}">${sp}%</span>
                ${barFill(sp)}
              </div>`
            : `<div class="row" style="grid-template-columns:minmax(140px,200px) 1fr">
                <span class="row-label"><span class="chev">▸</span>${esc(translateFidelityLabel(s.label))}</span>
                <span class="fine" style="margin:0"><span class="status-pill not-probed">未检测</span> — ${esc(s.detail || s.unmeasuredReason || "")}</span>
              </div>`;

          const weightPct = Math.round((s.weight ?? 0) * 100);
          const checks: string[] = [];
          checks.push(
            `<tr><td>是否检测</td><td>${s.measured ? statusPill("pass") : statusPill("unsupported")}</td><td>${esc(s.measured ? "计入保真度结果" : "不计入分母，不按 0 分处理")}</td></tr>`,
          );
          checks.push(
            `<tr><td>得分</td><td>${s.measured ? `${sp}%` : "—"}</td><td>${esc(s.detail || "")}</td></tr>`,
          );
          checks.push(
            `<tr><td>权重</td><td>${weightPct}%</td><td>在已检测项目中的合并权重</td></tr>`,
          );
          if (!s.measured) {
            checks.push(
              `<tr><td>未检测原因</td><td colspan="2">${esc(s.unmeasuredReason || s.detail || "引擎无法完成该项目的检测")}</td></tr>`,
            );
          }
          if (s.id === "correctness" && fidelity.items != null) {
            checks.push(
              `<tr><td>测试题数</td><td>${esc(String(fidelity.items))}</td><td>${esc(s.detail || "按正确性评分")}</td></tr>`,
            );
          }
          if (s.id === "determinism" && fidelity.firstDivergence) {
            const d = fidelity.firstDivergence;
            checks.push(
              `<tr><td>首次差异</td><td class="fail-row">${esc(d.itemId)} @ 第 ${d.charIndex} 个字符</td><td>${d.runs} 次温度为 0 的运行结果不一致</td></tr>`,
            );
          } else if (s.id === "determinism" && s.measured) {
            checks.push(
              `<tr><td>首次差异</td><td>${statusPill("pass")}</td><td>未发现温度为 0 时的结果差异</td></tr>`,
            );
          }
          if (s.id === "confidence" || s.id === "consistency") {
            checks.push(
              `<tr><td>依赖</td><td colspan="2">引擎提供 logprobs；未提供时不检测该项目，不按 0 分处理</td></tr>`,
            );
          }

          return `<div class="fid-block">
            <button type="button" class="fid-toggle" data-expand="${esc(panelId)}" aria-expanded="false" aria-controls="${esc(panelId)}">
              ${header}
            </button>
            <div class="expand-panel" id="${esc(panelId)}" hidden>
              <table class="drill-table">
                <thead><tr><th>检查内容</th><th>结果</th><th>说明</th></tr></thead>
                <tbody>${checks.join("")}</tbody>
              </table>
            </div>
          </div>`;
        })
        .join("")
    : `<p class="fine">本轮未检测引擎保真度。</p>`;

  const confRows = confTableRows(report);
  const boot = { confRows };

  const footerBits = [
    report.usage
      ? `${fmtTokens(report.usage.inputTokens + report.usage.outputTokens)} tokens (${fmtTokens(report.usage.inputTokens)} in · ${fmtTokens(report.usage.outputTokens)} out)`
      : null,
    fmtDuration(report.durationMs),
    options.label ? `file: ${options.label}` : null,
  ].filter(Boolean);

  const coverageCard = `<article class="card engine">
      <div class="card-kicker">接口与能力覆盖</div>
      <div class="card-value ${covTone}">${esc(coreHeadline)}</div>
      <div class="card-sub">
        <span>核心能力 ${core ? `${core.supported}/${core.total}` : "—"}</span>
        ${core?.missing?.length ? `<span class="badge critical">${core.missing.length} 项缺失</span>` : `<span class="badge good">核心能力完整</span>`}
      </div>
      ${miniTiers(report)}
      <div class="card-note">展示标准接口和功能的覆盖情况，缺失项会明确列出。</div>
    </article>`;

  const conformanceCard = `<article class="card engine">
      <div class="card-kicker">接口协议正确性</div>
      <div class="card-value ${confToneStrict}">${esc(confHeadline)}</div>
      <div class="card-sub">
        ${confMeasured ? `<span>${conf.passed}/${conf.total} 必须项</span>` : `<span>未检测</span>`}
        ${must.length ? `<span class="badge critical">${must.length} 项不符合要求</span>` : confMeasured ? `<span class="badge good">没有必须项失败</span>` : ""}
      </div>
      <div class="card-note">检查已实现接口是否正确遵循协议。接口不支持不等于协议错误。</div>
    </article>`;

  const capabilityCard = `<article class="card model">
      <div class="card-kicker">模型能力</div>
      <div class="card-value ${capTone}">${esc(capHeadline)}</div>
      <div class="card-sub">
        ${capMeasured ? `<span class="badge ${capTone}">${esc(translateVerdict(cap.verdict))}</span>` : `<span>未检测</span>`}
        ${capMeasured ? `<span>${cap.categories.length} 个能力类别</span>` : ""}
      </div>
      <div class="card-note">检查工具、JSON、指令遵循等实际能力，并判断是否达到最低要求。</div>
    </article>`;

  // Only when the benchmark ran: a headline rate belongs beside the scores it
  // is not, rather than buried under the section that explains it.
  const performanceCard = bench
    ? `<article class="card neutral">
      <div class="card-kicker">性能</div>
      <div class="card-value">${bench.decodeTokPerSec ? `${Math.round(bench.decodeTokPerSec.median * 10) / 10}` : "—"}</div>
      <div class="card-sub">
        <span>Token/秒生成</span>
        ${bench.ttftMs ? `<span class="badge">首 Token ${Math.round(bench.ttftMs.median)} 毫秒</span>` : ""}
      </div>
      <div class="card-note">仅供参考，与硬件有关，不参与评分；只适合在同一台机器上比较。</div>
    </article>`
    : "";

  const heroCards = [
    coverageCard,
    ran("conformance") ? conformanceCard : "",
    ran("capability") ? capabilityCard : "",
    performanceCard,
    report.reasoning ? reasoningCard(report.reasoning) : "",
  ].filter(Boolean);

  const outcomeHonesty = `<div class="outcome-lines">
    <div class="outcome-line good"><span class="ol-label">通过</span><span class="ol-n">${outcomes.pass}</span></div>
    <div class="outcome-line critical"><span class="ol-label">失败</span><span class="ol-n">${outcomes.fail}</span></div>
    <div class="outcome-line critical"><span class="ol-label">不支持</span><span class="ol-n">${outcomes.unsupported}</span></div>
    <div class="outcome-line caution"><span class="ol-label">无法确定</span><span class="ol-n">${outcomes.inconclusive}</span></div>
    <div class="outcome-line muted"><span class="ol-label">未执行</span><span class="ol-n">${outcomes.skipped}</span></div>
  </div>
  <div class="card-note">不支持和无法确定都不计作 0 分，也不计作失败。</div>`;

  const secondaryCards = [
    ran("agentic")
      ? `<div class="sec-card">
      <div class="card-kicker">Agent 任务</div>
      <div class="card-value ${agentic ? (agentic.passed === agentic.total ? "good" : agentic.passed === 0 ? "critical" : "caution") : ""}">${agentic ? `${agentic.passed}/${agentic.total}` : "—"}</div>
      <div class="card-note">多步工具任务，要求高于基础能力测试，不与模型能力合并。</div>
    </div>`
      : "",
    ran("fidelity")
      ? `<div class="sec-card">
      <div class="card-kicker">引擎保真度</div>
      <div class="card-value ${fidelity ? toneForPct(fidelity.pct) : ""}">${fidelity ? `${fidelity.pct}%` : "—"}</div>
      <div class="card-note">仅比较同一模型；模型不变，因此数值主要反映引擎差异。</div>
    </div>`
      : "",
    ran("conformance")
      ? `<div class="sec-card">
      <div class="card-kicker">结果状态说明</div>
      ${outcomeHonesty}
    </div>`
      : "",
  ].filter(Boolean);

  const secondaryRow =
    secondaryCards.length > 0
      ? `<div class="secondary" aria-label="Secondary signals">${secondaryCards.join("\n")}</div>`
      : "";

  const baselineSection = options.baseline
    ? `<section class="section" id="baseline">
      <div class="section-head">
        <h2>基线变化 <span class="tag engine">对比</span></h2>
        <div class="score">${esc(options.baseline.label)}</div>
      </div>
      ${
        options.baseline.regressions.length
          ? `<div class="findings">${options.baseline.regressions
              .map(
                (item) =>
                  `<div class="finding critical"><div class="finding-label">✗ 出现退化</div><div class="finding-detail">${esc(item)}</div></div>`,
              )
              .join("")}</div>`
          : `<div class="fine">没有记录到退化。</div>`
      }
      ${
        options.baseline.improvements.length
          ? `<div class="findings" style="margin-top:8px">${options.baseline.improvements
              .map(
                (item) =>
                  `<div class="finding"><div class="finding-label">· 有所改进</div><div class="finding-detail">${esc(item)}</div></div>`,
              )
              .join("")}</div>`
          : ""
      }
    </section>`
    : "";

  const conformanceSection = ran("conformance")
    ? `    <section class="section" id="conformance">
      <div class="section-head">
        <h2>接口协议正确性 <span class="tag engine">接口</span></h2>
        <div class="score ${confToneStrict}">${esc(confHeadline)}</div>
      </div>
      <p class="lede">只统计已实现接口中的必须项。点击接口卡片可以筛选下方检测明细，默认只显示失败项。</p>
      <div class="surface-grid">${surfaces || `<p class="fine">没有接口明细。</p>`}</div>
      <p class="fine" style="margin-top:10px">不支持和无法确定不属于失败，不计入协议正确性分母。可使用下方筛选查看所有检测。</p>
      <div class="filter-bar" role="toolbar" aria-label="筛选接口协议检测">
        <span class="label">显示</span>
        <button type="button" class="filter-chip active" data-outcome-filter="fail">失败</button>
        <button type="button" class="filter-chip" data-outcome-filter="all">全部检测</button>
        <button type="button" class="filter-chip" data-outcome-filter="pass">通过</button>
        <button type="button" class="filter-chip" data-outcome-filter="unsupported">不支持</button>
        <button type="button" class="filter-chip" data-outcome-filter="inconclusive">无法确定</button>
        <button type="button" class="filter-chip" data-outcome-filter="skipped">未执行</button>
        <button type="button" class="filter-chip" id="clear-surface-filter">清除接口筛选</button>
        <span class="filter-meta" id="conf-filter-count"></span>
      </div>
      <div class="conf-table-wrap" id="conf-table">
        <table class="drill-table">
          <thead>
            <tr>
              <th>检测项</th>
              <th>接口</th>
              <th>断言</th>
              <th>证据</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody id="conf-tbody"></tbody>
        </table>
      </div>
    </section>`
    : "";

  const capabilitySection = ran("capability")
    ? `    <section class="section" id="capability">
      <div class="section-head">
        <h2>模型能力 <span class="tag model">模型</span></h2>
        <div class="score ${capTone}">${capMeasured ? `${esc(capHeadline)} · ${esc(translateVerdict(cap.verdict))}` : "—"}</div>
      </div>
      <p class="lede">这是最低能力检查，不是智力排名。能力类别最低要求为 ${CATEGORY_FLOOR_PCT}%。点击类别可以展开详细测试，失败项优先显示。</p>
      <p class="hint-click">点击能力类别查看详细测试</p>
      ${capMeasured ? cats : `<p class="fine">本轮未检测模型能力。</p>`}
      ${weakNote}${unmeasNote}
    </section>`
    : "";

  const agenticSection = ran("agentic")
    ? `    <section class="section" id="agentic">
      <div class="section-head">
        <h2>Agent 任务 <span class="tag model">模型</span></h2>
        <div class="score ${agentic ? (agentic.passed === agentic.total ? "good" : "caution") : ""}">${agentic ? `${agentic.passed}/${agentic.total} 项` : "—"}</div>
      </div>
      <p class="lede">在模拟工作区中执行多步工具调用，要求高于基础能力测试，结果不会与模型能力合并。</p>
      ${tasks}
    </section>`
    : "";

  const fidelitySection = ran("fidelity")
    ? `    <section class="section" id="fidelity">
      <div class="section-head">
        <h2>引擎保真度 <span class="tag engine">引擎</span></h2>
        <div class="score ${fidelity ? toneForPct(fidelity.pct) : ""}">${fidelity ? `${fidelity.pct}%` : "—"}</div>
      </div>
      <p class="lede">仅适用于同一模型的比较。点击项目可以查看检测详情。未检测项目会明确标出，不按 0 分处理。</p>
      ${fidSlices}
      ${
        fidelity?.unmeasured?.length
          ? `<div class="fine">· ${fidelity.unmeasured.map(esc).join(", ")} 未检测</div>`
          : ""
      }
    </section>`
    : "";

  const performanceSection = bench ? benchSection(bench) : "";
  const reasoningSectionHtml = report.reasoning
    ? reasoningSection(report.reasoning)
    : "";

  return `<!DOCTYPE html>
<html lang="zh-CN" data-theme="light">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>llmprobe · ${esc(shortModel(model))}</title>
<script>${THEME_BOOT}</script>
<style>${CARD_STYLE}</style>
</head>
<body>
<div class="wrap">
  <header class="top">
    <div>
      <div class="brand">llmprobe 检测报告</div>
      <h1>${esc(model)}</h1>
      <div class="meta">
        ${engine ? `<span>${esc(engine)}</span>` : ""}
        ${baseUrl ? `<span>${esc(baseUrl)}</span>` : ""}
        ${report.run?.mode === "bench-only" ? `<span class="badge">benchmark only</span>` : ""}
        ${report.run?.mode === "eval-only" ? `<span class="badge">eval only</span>` : ""}
        ${report.run?.depth && report.run.depth !== "default" ? `<span class="badge">--${esc(report.run.depth)}</span>` : ""}
      </div>
      ${scopeNote}
    </div>
    <nav class="nav-links" aria-label="报告导航">${nav}${themeSwitcherHtml()}</nav>
  </header>

  <div class="overview-label">
    <h2>概览</h2>
    <p>${
      ran("conformance") && ran("capability")
        ? "三个独立结果，不合并平均"
        : "只展示本轮实际检测的内容，各项结果保持独立"
    }</p>
  </div>
  <div class="hero" aria-label="主要结果">
    ${heroCards.join("\n")}
  </div>

  ${secondaryRow}

  <div class="story">
    ${baselineSection}
    <section class="section" id="coverage">
      <div class="section-head">
        <h2>接口与能力覆盖 <span class="tag engine">引擎</span></h2>
        <div class="score ${covTone}">核心能力 ${esc(coreHeadline)}</div>
      </div>
      <p class="lede">按能力层级分别展示，不合并平均。点击能力层级可以展开该层级下的全部接口和功能。</p>
      <p class="hint-click">点击能力层级展开详情，缺失项优先显示</p>
      ${tierBlocks(report)}
      ${credits}
    </section>

    ${conformanceSection}
    ${capabilitySection}
    ${agenticSection}
    ${fidelitySection}
    ${performanceSection}
    ${reasoningSectionHtml}
  </div>

  <footer class="page">
    ${footerBits.map((b) => `<span>${esc(String(b))}</span>`).join('<span class="sep">·</span>')}
    <span>各项结果保持独立，不合并平均</span>
  </footer>
</div>
<script>window.__LLMPROBE__=${embedJson(boot)};</script>
<script>${REPORT_SCRIPT}</script>
<script>${THEME_SCRIPT}</script>
</body>
</html>`;
}
