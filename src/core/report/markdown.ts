import type { EvalCategory, RunReport } from "../outcome";
import {
  CATEGORY_LABELS_ZH,
  translateAgenticTaskName,
  translateCapabilityLabel,
  translateDetail,
  translateSurface,
  translateTier,
  translateVerdict,
} from "../../i18n/zh-cn";

const CATEGORY_LABELS: Record<EvalCategory, string> = {
  ...CATEGORY_LABELS_ZH,
};

const badge = (label: string, value: string, colour: string): string =>
  `![${label}](https://img.shields.io/badge/${encodeURIComponent(label)}-${encodeURIComponent(value)}-${colour})`;

const colourFor = (pct: number): string =>
  pct >= 90
    ? "brightgreen"
    : pct >= 70
      ? "yellow"
      : pct >= 40
        ? "orange"
        : "red";

/** README-ready output: badges plus the three tables. */
export function renderMarkdown(report: RunReport): string {
  const { coverage, conformance, capability, target } = report;

  const core = coverage.byTier.find((t) => t.tier === "core");
  const lines: string[] = [];

  lines.push(`# llmprobe 检测报告 — ${target.engine ?? target.baseUrl}`);
  lines.push("");
  lines.push(
    [
      badge("core", `${core?.pct ?? 0}%`, colourFor(core?.pct ?? 0)),
      badge("conformance", `${conformance.pct}%`, colourFor(conformance.pct)),
      badge(
        "model",
        capability.verdict === "below-floor"
          ? "未达到最低要求"
          : translateVerdict(capability.verdict),
        capability.verdict === "strong"
          ? "brightgreen"
          : capability.verdict === "capable"
            ? "green"
            : "red",
      ),
    ].join(" "),
  );
  lines.push("");
  lines.push(
    `**检测地址：** \`${target.baseUrl}\` · **模型：** \`${target.model}\``,
  );
  lines.push("");

  lines.push("## 接口与能力覆盖情况");
  lines.push("");
  lines.push("| 层级 | 已支持 | 覆盖率 | 缺失项 |");
  lines.push("| --- | --- | --- | --- |");
  for (const tier of coverage.byTier) {
    const missing = tier.missing.length
      ? tier.missing.map(translateCapabilityLabel).join(", ")
      : "—";
    lines.push(
      `| ${translateTier(tier.tier)} | ${tier.supported}/${tier.total} | ${tier.pct}% | ${missing} |`,
    );
  }
  if (coverage.credits.length) {
    lines.push("");
    for (const credit of coverage.credits) {
      lines.push(`> 已检测到但不计分：${credit.label}`);
    }
  }
  lines.push("");

  lines.push(`## 接口协议正确性 — ${conformance.pct}%`);
  lines.push("");
  lines.push("_只统计已实现接口中的 MUST（必须满足）断言。_");
  lines.push("");
  if (conformance.total === 0) {
    lines.push("没有执行任何检测：未发现可执行的接口。");
  } else {
    lines.push("| 接口 | 通过 | 得分 |");
    lines.push("| --- | --- | --- |");
    for (const surface of conformance.bySurface) {
      lines.push(
        `| ${translateSurface(surface.surface)} | ${surface.passed}/${surface.total} | ${surface.pct}% |`,
      );
    }
    if (conformance.inconclusive.length) {
      lines.push("");
      lines.push(
        `**${conformance.inconclusive.length} 项无法确定** — 未能实际执行该接口：`,
      );
      lines.push("");
      for (const result of conformance.inconclusive) {
        lines.push(
          `- \`${result.id}\` — ${translateDetail(result.reason) ?? "未知原因"}`,
        );
      }
    }
    if (conformance.warnings.length) {
      lines.push("");
      lines.push(
        `**${conformance.warnings.length} 项 SHOULD 警告**（不计分）：`,
      );
      lines.push("");
      for (const warning of conformance.warnings) {
        lines.push(
          `- ${translateDetail(warning.label) ?? warning.label}${warning.message ? ` — ${translateDetail(warning.message)}` : ""}`,
        );
      }
    }
  }
  lines.push("");

  const verdict =
    capability.verdict === "below-floor"
      ? "未达到最低要求 ❌"
      : `${translateVerdict(capability.verdict)} ✅`;
  lines.push(`## 模型能力 — ${capability.pct}%（${verdict}）`);
  lines.push("");
  if (capability.total === 0) {
    lines.push("未执行模型能力测试。");
  } else {
    lines.push("| 能力类别 | 通过 | 得分 |");
    lines.push("| --- | --- | --- |");
    for (const category of capability.categories) {
      const weak = capability.weakCategories.includes(category.category)
        ? " ⚠️"
        : "";
      lines.push(
        `| ${CATEGORY_LABELS[category.category]}${weak} | ${category.passed}/${category.total} | ${category.pct}% |`,
      );
    }
  }

  if (report.agentic) {
    const ag = report.agentic;
    lines.push("");
    lines.push(`## Agent 任务 — ${ag.passed}/${ag.total} 项`);
    lines.push("");
    lines.push(
      "_在模拟工作区中执行多步工具调用。这是比基础能力更高的要求，结果不会与模型能力分数合并。_",
    );
    lines.push("");
    lines.push("| 任务 | 结果 | 步骤数 |");
    lines.push("| --- | --- | --- |");
    for (const task of ag.tasks) {
      const result = task.passed
        ? "✅"
        : `❌ ${translateDetail(task.detail) ?? translateDetail(task.failure) ?? "失败"}`;
      lines.push(
        `| ${translateAgenticTaskName(task.id, task.name)} | ${result} | ${task.steps} |`,
      );
    }
  }

  if (report.fidelity) {
    const fid = report.fidelity;
    lines.push("");
    lines.push(`## 引擎保真度 — ${fid.pct}%`);
    lines.push("");
    lines.push(
      "_仅适用于同一模型的比较：模型保持不变，因此该数值主要反映引擎差异。_",
    );
    lines.push("");
    lines.push("| 项目 | 得分 | 说明 |");
    lines.push("| --- | --- | --- |");
    for (const slice of fid.slices) {
      const score = slice.measured
        ? `${Math.round(slice.score * 10000) / 100}%`
        : "未检测";
      lines.push(
        `| ${slice.label} | ${score} | ${translateDetail(slice.detail) ?? ""} |`,
      );
    }
    if (fid.firstDivergence) {
      const d = fid.firstDivergence;
      lines.push("");
      lines.push(
        `> ⚠️ 温度为 0 的重复运行在第 ${d.charIndex} 个字符处出现差异（\`${d.itemId}\`，第 ${d.run}/${d.runs} 次）— 引擎存在非确定性。`,
      );
    }
    if (fid.reasoningCaveat) {
      lines.push("");
      lines.push(
        "> 推理模型：置信度取自思考后的概率分布，因此该分数只应作为最低参考。",
      );
    }
  }

  lines.push("");
  lines.push("---");
  lines.push("");
  lines.push(
    `_由 [llmprobe](https://github.com/ddalcu/llmprobe) 生成 · ${report.usage ? `${(report.usage.inputTokens + report.usage.outputTokens).toLocaleString()} Tokens · ` : ""}${Math.round(report.durationMs / 1000)} 秒_`,
  );

  return lines.join("\n");
}
