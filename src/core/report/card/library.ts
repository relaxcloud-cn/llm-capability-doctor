import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, dirname, join, relative, resolve } from "node:path";

import type { JsonReport } from "../json";
import { normalizeJsonReport } from "../json";
import { renderCardHtml } from "./allinone";
import { renderCompareWorkbenchHtml } from "./compare-workbench";
import { CARD_STYLE } from "./style.css";
import { THEME_BOOT, THEME_SCRIPT, themeSwitcherHtml } from "./theme";
import { LIBRARY_SCRIPT } from "./library-script";
import { translateHtml } from "../../../i18n/zh-cn";
import {
  endpointLabel,
  esc,
  embedJson,
  runSlug,
  shortModel,
  tier,
} from "./shared";

/** Catalog / non-report artifacts that must never enter the ranking table. */
const SKIP_JSON = new Set([
  "library.json",
  "compare-model.json",
  "view-model.json",
]);

export class LibraryEmptyError extends Error {
  constructor(dir: string, detail?: string) {
    super(
      `no llmprobe --save reports found in ${dir}${detail ? ` (${detail})` : ""}. ` +
        `Put probe JSON saves in this directory (or its parent), then re-run ` +
        `\`llmprobe --library ${dir}\`.`,
    );
    this.name = "LibraryEmptyError";
  }
}

export interface LibraryRun {
  slug: string;
  label: string;
  report: JsonReport;
  href: string;
  src: string;
  jsonName: string;
  /** When this run happened — its own `run.startedAt`, else the file's mtime. */
  recordedAt: string;
}

export function isJsonReport(obj: unknown): obj is JsonReport {
  if (!obj || typeof obj !== "object") return false;
  const o = obj as JsonReport;
  return Boolean(
    o.target &&
    (o.target.model || o.target.baseUrl) &&
    o.coverage &&
    Array.isArray(o.coverage.byTier),
  );
}

function listCandidateJsonFiles(dir: string): string[] {
  const absDir = resolve(dir);
  if (!existsSync(absDir) || !statSync(absDir).isDirectory()) return [];
  const found: string[] = [];
  for (const name of readdirSync(absDir)) {
    if (!name.endsWith(".json")) continue;
    if (SKIP_JSON.has(name)) continue;
    const abs = join(absDir, name);
    try {
      if (!statSync(abs).isFile()) continue;
    } catch {
      continue;
    }
    found.push(abs);
  }
  return found.sort();
}

function loadRunsFromFiles(files: string[], hrefDir: string): LibraryRun[] {
  const runs: LibraryRun[] = [];
  const usedSlugs = new Map<string, number>();
  const hrefRoot = resolve(hrefDir);

  for (const abs of files) {
    let report: JsonReport;
    try {
      report = JSON.parse(readFileSync(abs, "utf8")) as JsonReport;
    } catch {
      continue;
    }
    if (!isJsonReport(report)) continue;
    // Read the timestamp before normalizing: normalizeJsonReport stamps a v1
    // report with "now", which on a most-recent-first table would float every
    // legacy run to the top on every rebuild. The file's mtime is the honest
    // answer for a save that never recorded when it ran.
    const recordedAt = report.run?.startedAt ?? mtimeIso(abs);
    report = normalizeJsonReport(report);

    let s = runSlug(
      report.target?.model || basename(abs, ".json"),
      report.target?.baseUrl,
    );
    const n = (usedSlugs.get(s) ?? 0) + 1;
    usedSlugs.set(s, n);
    if (n > 1) s = `${s}-${n}`;

    // Cards always live next to the library index.
    const jsonName =
      resolve(dirname(abs)) === hrefRoot ? basename(abs) : `${s}.json`;

    runs.push({
      slug: s,
      label: runLabel(report),
      report,
      href: `${s}.html`,
      src: abs,
      jsonName,
      recordedAt,
    });
  }

  return runs;
}

/**
 * Discover valid --save reports for a library directory.
 *
 * Uses JSON files inside the library, and also adopts any *additional* probe
 * saves from the parent directory (prototype layout: saves in `runs/`, HTML in
 * `runs/report-card/`). Parent files are copied into the library when missing
 * so the next rebuild is self-contained.
 *
 * Identity for dedupe is `target.model` (then baseUrl+model).
 */
export function discoverLibraryRuns(
  dir: string,
  options: { adoptFromParent?: boolean } = {},
): LibraryRun[] {
  const absDir = resolve(dir);
  const adopt = options.adoptFromParent !== false;

  mkdirSync(absDir, { recursive: true });

  const identity = (r: LibraryRun): string =>
    `${r.report.target?.model ?? ""}@@${r.report.target?.baseUrl ?? ""}`;

  const present = new Set(
    loadRunsFromFiles(listCandidateJsonFiles(absDir), absDir).map(identity),
  );

  if (adopt) {
    const parent = dirname(absDir);
    if (parent && parent !== absDir) {
      const parentRuns = loadRunsFromFiles(
        listCandidateJsonFiles(parent),
        absDir,
      );
      for (const run of parentRuns) {
        const id = identity(run);
        if (present.has(id)) continue;
        const dest = join(absDir, run.jsonName);
        if (resolve(run.src) !== resolve(dest) && !existsSync(dest)) {
          copyFileSync(run.src, dest);
        }
        present.add(id);
      }
    }
  }

  return loadRunsFromFiles(listCandidateJsonFiles(absDir), absDir);
}

function mtimeIso(path: string): string {
  try {
    return statSync(path).mtime.toISOString();
  } catch {
    return new Date(0).toISOString();
  }
}

/** Model, plus the endpoint that tells two runs of it apart. */
function runLabel(report: JsonReport): string {
  const model = shortModel(report.target?.model) || "run";
  const host = endpointLabel(report.target?.baseUrl);
  return host ? `${model} · ${host}` : model;
}

function runSummary(run: LibraryRun) {
  const r = run.report;
  const core = tier(r, "core");
  const ext = tier(r, "extended");
  const front = tier(r, "frontier");
  const confMeasured = (r.conformance?.total ?? 0) > 0;
  const capMeasured = (r.capability?.categories?.length ?? 0) > 0;
  return {
    slug: run.slug,
    href: run.href,
    recordedAt: run.recordedAt,
    model: r.target?.model ?? run.label,
    short: shortModel(r.target?.model ?? run.label),
    engine: r.target?.engine ?? null,
    baseUrl: r.target?.baseUrl ?? null,
    // What separates two rows for the same model, so it is what the row shows.
    endpoint: endpointLabel(r.target?.baseUrl),
    source: run.jsonName,
    core: core?.pct ?? null,
    extended: ext?.pct ?? null,
    frontier: front?.pct ?? null,
    conformance: confMeasured ? r.conformance.pct : null,
    capability: capMeasured ? r.capability.pct : null,
    verdict: capMeasured ? r.capability.verdict : null,
    agenticPassed: r.agentic?.passed ?? null,
    agenticTotal: r.agentic?.total ?? null,
    agenticRatio:
      r.agentic && r.agentic.total > 0
        ? r.agentic.passed / r.agentic.total
        : null,
    fidelity: r.fidelity?.pct ?? null,
    // Only present when --bench ran. Null, never zero: a run nobody
    // benchmarked must not rank as the slowest engine in the library.
    decode: r.bench?.decodeTokPerSec?.median ?? null,
    prefill: r.bench?.prefillTokPerSec?.median ?? null,
    ttft: r.bench?.ttftMs?.median ?? null,
  };
}

/** Ranking table + search + multi-select compare dock. */
export function renderLibraryHtml(
  runs: LibraryRun[],
  options: { dirLabel?: string } = {},
): string {
  const catalog = runs.map(runSummary);
  const dirLabel = options.dirLabel ?? "this directory";

  return translateHtml(`<!DOCTYPE html>
<html lang="zh-CN" data-theme="light">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>llmprobe · 模型报告库</title>
<script>${THEME_BOOT}</script>
<style>${CARD_STYLE}</style>
</head>
<body>
<div class="wrap">
  <header class="top">
    <div>
      <div class="brand">llmprobe</div>
      <h1>模型报告库</h1>
      <div class="meta">
        <span id="library-count">${catalog.length} model${catalog.length === 1 ? "" : "s"}</span>
        <span>自动同步 · ${esc(dirLabel)}</span>
        <span>各项结果保持独立</span>
      </div>
    </div>
    <nav class="nav-links">
      <a class="btn" href="index.html">报告库</a>
      ${
        catalog.length >= 1
          ? `<a class="btn primary" href="compare.html">快速比较</a>`
          : ""
      }
      ${themeSwitcherHtml()}
    </nav>
  </header>

  <div class="narrative">
    <h2>如何使用报告库</h2>
    <p class="lead">可以按任意列对模型排序，打开完整报告，或选择模型进行比较。</p>
    <ul>
      <li><strong>接口与能力覆盖</strong>展示核心、扩展、前沿三层能力（绿色 ≥90%，黄色 ≥70%，低于此值为红色）。</li>
      <li><strong>接口协议正确性</strong>和<strong>模型能力</strong>保持独立，不合并成一个总分。</li>
      <li><strong>生成速度</strong>、<strong>输入处理速度</strong>和<strong>首 Token 延迟</strong>只来自 <code>--bench</code> 检测，并且与硬件有关，只应在同一台机器上比较。</li>
      <li>在两行中选择<strong>比较</strong>，然后点击底部的<strong>比较模型</strong>，也可以直接打开<strong>快速比较</strong>。</li>
      <li>使用 <code>--library</code> 检测，或将报告保存到此目录后，列表会自动刷新。</li>
    </ul>
  </div>

  <div class="library-toolbar">
    <div class="library-search" role="search">
      <label class="visually-hidden" for="model-search">搜索模型</label>
      <input
        id="model-search"
        type="search"
        placeholder="搜索模型…"
        autocomplete="off"
        spellcheck="false"
      />
      <button type="button" class="search-clear" id="search-clear" aria-label="清除搜索">×</button>
    </div>
    <div class="sort-ctrl">
      <label for="sort-key">排序方式</label>
      <select id="sort-key">
        <option value="date">最近检测</option>
        <option value="decode">生成速度 Token/秒</option>
        <option value="prefill">输入处理速度 Token/秒</option>
        <option value="ttft">首 Token 延迟</option>
        <option value="capability">Model capability</option>
        <option value="conformance">Engine conformance</option>
        <option value="coverage">覆盖情况（核心→扩展→前沿）</option>
        <option value="core">核心能力覆盖</option>
        <option value="extended">扩展能力覆盖</option>
        <option value="frontier">前沿能力覆盖</option>
        <option value="agentic">Agentic</option>
        <option value="fidelity">引擎保真度</option>
        <option value="model">模型名称</option>
      </select>
      <select id="sort-dir">
        <option value="desc">High → low</option>
        <option value="asc">Low → high</option>
      </select>
    </div>
    <div class="library-count" id="filter-meta">最新在前 · 点击列标题排序 · 最多选择 2 个模型进行比较</div>
  </div>

  <div class="rank-wrap">
    <table class="rank-table" aria-label="Model rankings">
      <thead>
        <tr>
          <th scope="col">#</th>
          <th scope="col" data-sort="model">模型 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="coverage" title="核心 | 扩展 | 前沿">接口与能力覆盖 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="conformance">接口协议正确性 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="capability">模型能力 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="agentic">Agentic <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="decode" title="持续生成速度，Token/秒；与硬件有关">生成速度 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="prefill" title="输入内容处理速度，Token/秒；与硬件有关">输入处理速度 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="ttft" title="首 Token 延迟，越低越好">首 Token 延迟 <span class="sort-ind">↕</span></th>
          <th scope="col" data-sort="date" class="active">Last run <span class="sort-ind">▼</span></th>
          <th scope="col">操作</th>
        </tr>
      </thead>
      <tbody id="rank-body"></tbody>
    </table>
  </div>

  <p class="fine" style="margin-top:4px">
    重建报告库：<code>llmprobe --library ${esc(dirLabel)}</code>
    · 将检测结果写入此报告库：
    <code>llmprobe &lt;url&gt; --library ${esc(dirLabel)}</code>
    · 当 <code>--save</code>/<code>--html</code> 写入已有 <code>library.json</code> 的目录时，也会自动同步。
  </p>
</div>

<div class="compare-dock" id="compare-dock" role="dialog" aria-label="比较选择">
  <h3>比较选择</h3>
  <div class="picks" id="compare-picks"></div>
  <div class="dock-actions">
    <button type="button" class="btn" id="compare-clear">清除</button>
    <button type="button" class="btn primary" id="compare-go" disabled>比较模型</button>
  </div>
</div>

<script>window.__LIBRARY__=${embedJson(catalog)};</script>
<script>${LIBRARY_SCRIPT}</script>
<script>${THEME_SCRIPT}</script>
</body>
</html>`);
}

export interface SyncLibraryResult {
  dir: string;
  runs: number;
  models: string[];
  indexPath: string;
  comparePath: string;
  cardPaths: string[];
  catalogPath: string;
}

/**
 * Rebuild library index, compare workbench, and per-model cards from JSON saves in `dir`.
 * Refuses to write an empty catalog (avoids wiping a good index).
 */
export function syncLibrary(dir: string): SyncLibraryResult {
  const absDir = resolve(dir);
  mkdirSync(absDir, { recursive: true });

  const runs = discoverLibraryRuns(absDir);
  if (runs.length === 0) {
    throw new LibraryEmptyError(
      absDir,
      "no valid probe JSON — put --save files here or in the parent folder; " +
        "catalogs like library.json / view-model.json are ignored",
    );
  }

  const cardPaths: string[] = [];

  for (const run of runs) {
    const cardPath = join(absDir, run.href);
    writeFileSync(
      cardPath,
      renderCardHtml(run.report, {
        label: run.jsonName,
        libraryHref: "index.html",
      }),
    );
    cardPaths.push(cardPath);
  }

  const indexPath = join(absDir, "index.html");
  writeFileSync(
    indexPath,
    renderLibraryHtml(runs, { dirLabel: basename(absDir) }),
  );

  const comparePath = join(absDir, "compare.html");
  writeFileSync(
    comparePath,
    renderCompareWorkbenchHtml(
      runs.map((r) => ({
        label: r.label,
        report: r.report,
        href: r.href,
        file: r.src,
        slug: r.slug,
        recordedAt: r.recordedAt,
      })),
      { libraryHref: "index.html" },
    ),
  );

  const catalogPath = join(absDir, "library.json");
  writeFileSync(
    catalogPath,
    `${JSON.stringify(
      {
        generatedAt: new Date().toISOString(),
        runs: runs.map((r) => ({
          ...runSummary(r),
          sourcePath: r.src,
        })),
      },
      null,
      2,
    )}\n`,
  );

  return {
    dir: absDir,
    runs: runs.length,
    models: runs.map((r) => r.label),
    indexPath,
    comparePath,
    cardPaths,
    catalogPath,
  };
}

/**
 * Write (or overwrite) a report JSON into the library under a stable slug name,
 * then rebuild the library. Returns the slug used.
 */
export function ingestReportIntoLibrary(
  dir: string,
  report: JsonReport,
  options: { preferredFileName?: string } = {},
): { slug: string; jsonPath: string; sync: SyncLibraryResult } {
  const absDir = resolve(dir);
  mkdirSync(absDir, { recursive: true });
  const normalized = normalizeJsonReport(report);
  const s = runSlug(
    normalized.target?.model || "run",
    normalized.target?.baseUrl,
  );
  const jsonName = options.preferredFileName?.endsWith(".json")
    ? basename(options.preferredFileName)
    : `${s}.json`;
  const jsonPath = join(absDir, jsonName);
  writeFileSync(jsonPath, `${JSON.stringify(normalized, null, 2)}\n`);
  const sync = syncLibrary(absDir);
  return { slug: s, jsonPath, sync };
}

/** True when dir looks like an llmprobe library (has library.json). */
export function isLibraryDir(dir: string): boolean {
  try {
    return existsSync(join(resolve(dir), "library.json"));
  } catch {
    return false;
  }
}

/**
 * Where every run is recorded unless --library points elsewhere: one library
 * per machine, so probes accumulate across projects instead of scattering a
 * report-card/ beside whatever directory you happened to be in.
 */
export const HOME_LIBRARY_DIR = join(homedir(), ".llmprobe");
