export const ALL_IN_ONE_REPORT_STYLE = `
:root, [data-theme="light"] {
  color-scheme: light;
  --page: #f3f2ed;
  --surface: #fffdf8;
  --surface-2: #f7f6f1;
  --ink: #171916;
  --ink-2: #434842;
  --muted: #747a73;
  --line: #dedfd9;
  --line-strong: #c9ccc5;
  --green: #237656;
  --green-bg: #e4f1e9;
  --amber: #9a660e;
  --amber-bg: #fff0d5;
  --red: #b33c35;
  --red-bg: #fbe7e4;
  --blue: #2f6fa0;
  --blue-bg: #e8f0f7;
}
[data-theme="dark"] {
  color-scheme: dark;
  --page: #111310;
  --surface: #1a1d19;
  --surface-2: #21241f;
  --ink: #f3f4ef;
  --ink-2: #ccd1ca;
  --muted: #999f98;
  --line: #30342e;
  --line-strong: #454a42;
  --green: #61c497;
  --green-bg: #193c2d;
  --amber: #efbb5b;
  --amber-bg: #473615;
  --red: #f08378;
  --red-bg: #492523;
  --blue: #83b8e1;
  --blue-bg: #20384a;
}
[data-theme="cyber"] {
  color-scheme: dark;
  --page: #071018;
  --surface: #0d1822;
  --surface-2: #12202b;
  --ink: #ebfaff;
  --ink-2: #b3d4df;
  --muted: #7899a5;
  --line: #1a3946;
  --line-strong: #2a5967;
  --green: #68f0aa;
  --green-bg: #123b2d;
  --amber: #ffe083;
  --amber-bg: #423817;
  --red: #ff7185;
  --red-bg: #452431;
  --blue: #46d7ff;
  --blue-bg: #123b4a;
}
* { box-sizing: border-box; }
html { scroll-behavior: smooth; }
body {
  margin: 0;
  background: var(--page);
  color: var(--ink);
  font: 14px/1.55 Inter, "PingFang SC", "Microsoft YaHei", system-ui, sans-serif;
}
button, select { font: inherit; }
a { color: inherit; }
.report-page {
  width: min(1460px, calc(100% - 36px));
  margin: 0 auto;
  padding: 28px 0 64px;
}
.top {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 24px;
  padding-bottom: 20px;
}
.brand { color: var(--muted); font-size: 12px; font-weight: 800; }
h1 { margin: 5px 0 4px; font-size: 27px; line-height: 1.2; letter-spacing: 0; }
.meta { color: var(--muted); font-size: 13px; }
.meta span + span::before { content: "·"; margin: 0 7px; }
.nav-links { display: flex; align-items: center; gap: 10px; }
.nav-links .btn {
  border: 1px solid var(--line-strong);
  border-radius: 7px;
  background: var(--surface);
  padding: 7px 10px;
  text-decoration: none;
}
.theme-switch { display: flex; align-items: center; gap: 8px; color: var(--muted); font-size: 12px; }
.theme-switch select {
  border: 1px solid var(--line-strong);
  border-radius: 7px;
  background: var(--surface);
  color: var(--ink);
  padding: 7px 10px;
}
.report-layout {
  display: grid;
  grid-template-columns: 220px minmax(0, 1fr);
  gap: 28px;
  align-items: start;
  border-top: 1px solid var(--line);
}
.toc { position: sticky; top: 18px; padding: 25px 0; }
.toc-title { font-size: 15px; font-weight: 800; }
.toc-note { margin: 3px 0 16px; color: var(--muted); font-size: 11px; }
.toc nav { border-left: 1px solid var(--line-strong); }
.toc-link {
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr);
  gap: 2px 8px;
  min-width: 0;
  margin-left: -2px;
  padding: 10px 12px;
  border-left: 3px solid transparent;
  color: var(--ink-2);
  text-decoration: none;
}
.toc-link:hover, .toc-link:focus-visible, .toc-link.active {
  border-left-color: var(--blue);
  background: var(--blue-bg);
  color: var(--ink);
  outline: none;
}
.toc-number { grid-row: 1 / span 2; color: var(--muted); font-size: 11px; font-weight: 800; }
.toc-question { overflow-wrap: anywhere; font-size: 13px; font-weight: 800; }
.toc-status { color: var(--muted); font-size: 11px; }
.toc-link.active .toc-number, .toc-link.active .toc-status { color: var(--blue); }
.report-content { min-width: 0; }
.section { padding: 26px 0; border-bottom: 1px solid var(--line); scroll-margin-top: 18px; }
.section-head {
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr) auto;
  gap: 14px;
  align-items: start;
  margin-bottom: 14px;
}
.qno { color: var(--blue); font-size: 26px; font-weight: 800; line-height: 1; }
h2 { margin: 0; font-size: 19px; line-height: 1.25; letter-spacing: 0; }
h3 { letter-spacing: 0; }
.lede { margin: 5px 0 0; color: var(--muted); }
.answer { text-align: right; }
.answer strong { display: block; font-size: 17px; }
.answer span { color: var(--muted); font-size: 12px; }
.verdict-overview {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 230px;
  background: var(--surface);
  border: 1px solid var(--line);
  border-left: 4px solid var(--blue);
  border-radius: 8px;
  overflow: hidden;
}
.verdict-copy { padding: 20px 22px; }
.verdict-kicker, .score-label { color: var(--muted); font-size: 11px; font-weight: 800; }
.verdict-copy > strong { display: block; margin-top: 4px; font-size: 23px; line-height: 1.3; }
.verdict-copy > strong.good { color: var(--green); }
.verdict-copy > strong.bad { color: var(--red); }
.verdict-copy p { margin: 8px 0 0; color: var(--ink-2); }
.verdict-status { padding: 20px; background: var(--blue-bg); border-left: 1px solid var(--line); }
.verdict-status small, .verdict-status strong, .verdict-status span { display: block; }
.verdict-status small { color: var(--muted); font-size: 11px; font-weight: 800; }
.verdict-status strong { margin: 5px 0; color: var(--blue); font-size: 19px; }
.verdict-status span { color: var(--ink-2); font-size: 12px; }
.decision-path {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  margin-top: 12px;
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: 8px;
  overflow: hidden;
}
.path-item { display: grid; grid-template-columns: 26px minmax(0, 1fr); gap: 8px; padding: 14px 16px; border-right: 1px solid var(--line); }
.path-item:last-child { border-right: 0; }
.path-index { color: var(--muted); font-size: 11px; font-weight: 800; }
.path-copy strong, .path-copy span { display: block; }
.path-copy strong { font-size: 13px; }
.path-copy span { margin-top: 3px; color: var(--muted); font-size: 11px; }
.path-result { margin-top: 7px; font-size: 15px; font-weight: 800; }
.good-text { color: var(--green); }
.bad-text { color: var(--red); }
.warn-text { color: var(--amber); }
.pending-text { color: var(--blue); }
.scope-note { margin: 10px 0 0; padding-left: 12px; border-left: 3px solid var(--line-strong); color: var(--muted); font-size: 12px; }
.inventory-note { margin: 0 0 10px; color: var(--muted); font-size: 12px; }
.table-wrap { max-height: 620px; overflow: auto; background: var(--surface); border: 1px solid var(--line); border-radius: 8px; }
.report-table { width: 100%; border-collapse: collapse; }
.report-table th {
  position: sticky;
  top: 0;
  z-index: 1;
  padding: 9px 13px;
  text-align: left;
  color: var(--muted);
  background: var(--surface-2);
  font-size: 11px;
  font-weight: 800;
}
.report-table td { padding: 11px 13px; border-top: 1px solid var(--line); vertical-align: top; }
.report-table td:first-child { width: 120px; font-weight: 700; }
.report-table td:nth-child(2) { width: 250px; }
.report-table td:nth-child(3) { width: 108px; }
.soft-table td:nth-child(4) { width: 104px; }
.source { color: var(--ink-2); font-size: 11px; font-weight: 800; }
.item-name { display: block; font-weight: 700; }
.item-id { display: block; margin-top: 2px; color: var(--muted); font: 10px ui-monospace, SFMono-Regular, Menlo, monospace; overflow-wrap: anywhere; }
.reason { color: var(--ink-2); font-size: 12px; }
.detail { display: block; margin-top: 4px; color: var(--muted); font-size: 11px; }
.status-pill, .impact-pill {
  display: inline-flex;
  border-radius: 999px;
  padding: 3px 8px;
  font-size: 11px;
  font-weight: 800;
  white-space: nowrap;
}
.status-pill.pass, .status-pill.supported { color: var(--green); background: var(--green-bg); }
.status-pill.fail, .status-pill.unsupported { color: var(--red); background: var(--red-bg); }
.status-pill.inconclusive, .status-pill.skipped, .status-pill.not-run, .status-pill.not-probed { color: var(--blue); background: var(--blue-bg); }
.impact-pill.strong { color: var(--red); background: var(--red-bg); }
.impact-pill.small { color: var(--amber); background: var(--amber-bg); }
.impact-pill.record { color: var(--blue); background: var(--blue-bg); }
.metric-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 12px; margin-top: 12px; }
.metric { padding: 15px 16px; background: var(--surface); border: 1px solid var(--line); border-radius: 8px; }
.metric strong { display: block; margin-top: 5px; font-size: 21px; }
.metric p { margin: 6px 0 0; color: var(--muted); font-size: 11px; }
.agent-overview { display: grid; grid-template-columns: 250px minmax(0, 1fr); background: var(--surface); border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
.agent-status { padding: 20px; background: var(--blue-bg); border-right: 1px solid var(--line); }
.agent-status strong { display: block; margin-top: 7px; color: var(--blue); font-size: 22px; }
.agent-status p { margin: 7px 0 0; color: var(--ink-2); font-size: 12px; }
.agent-counts { display: grid; grid-template-columns: repeat(3, 1fr); }
.agent-count { padding: 20px 18px; border-right: 1px solid var(--line); }
.agent-count:last-child { border-right: 0; }
.agent-count small, .agent-count strong, .agent-count span { display: block; }
.agent-count small { color: var(--muted); font-size: 11px; font-weight: 800; }
.agent-count strong { margin-top: 5px; font-size: 21px; }
.agent-count span { margin-top: 4px; color: var(--muted); font-size: 11px; }
.agent-group { margin-top: 18px; }
.agent-group-head { display: flex; justify-content: space-between; align-items: end; gap: 16px; margin-bottom: 8px; }
.agent-group-head small { color: var(--muted); font-size: 11px; font-weight: 800; }
.agent-group-head h3 { margin: 2px 0 0; font-size: 16px; }
.agent-table td:first-child { width: 72px; color: var(--muted); font: 11px ui-monospace, SFMono-Regular, Menlo, monospace; }
.agent-table td:nth-child(2) { width: 250px; font-weight: 700; }
.agent-table td:nth-child(3) { width: auto; color: var(--ink-2); }
.agent-table td:last-child { width: 92px; }
.agent-final-rule { margin-top: 18px; padding: 14px 16px; background: var(--surface-2); border-left: 4px solid var(--red); }
.agent-final-rule strong { display: block; font-size: 13px; }
.agent-final-rule p { margin: 4px 0 0; color: var(--ink-2); font-size: 12px; }
details { margin-top: 12px; background: var(--surface); border: 1px solid var(--line); border-radius: 8px; }
summary { cursor: pointer; padding: 13px 15px; font-weight: 800; }
.technical-body { padding: 0 15px 15px; }
.technical-section { padding: 18px 0; border-top: 1px solid var(--line); }
.technical-section h3 { margin: 0 0 8px; font-size: 15px; }
.technical-section p { margin: 0 0 8px; color: var(--muted); font-size: 12px; }
.ctx-charts { display: grid; grid-template-columns: minmax(0, 520px); gap: 12px; margin: 12px 0; }
.ctx-chart { width: 100%; height: auto; display: block; background: var(--surface-2); border: 1px solid var(--line); border-radius: 8px; }
.page-footer { padding-top: 18px; color: var(--muted); font-size: 12px; }
.page-footer span + span::before { content: "·"; margin: 0 7px; }
@media (max-width: 980px) {
  .report-layout { grid-template-columns: 184px minmax(0, 1fr); gap: 18px; }
  .toc-link { grid-template-columns: 23px minmax(0, 1fr); padding: 9px 8px; }
  .verdict-overview, .decision-path, .agent-overview { grid-template-columns: 1fr; }
  .verdict-status { border-top: 1px solid var(--line); border-left: 0; }
  .path-item { border-right: 0; border-bottom: 1px solid var(--line); }
  .path-item:last-child { border-bottom: 0; }
  .agent-status { border-right: 0; border-bottom: 1px solid var(--line); }
}
@media (max-width: 720px) {
  .report-page { width: min(100% - 24px, 680px); }
  .top { gap: 12px; }
  .report-layout { grid-template-columns: 146px minmax(0, 1fr); gap: 12px; }
  .toc { top: 8px; }
  .toc-link { grid-template-columns: 1fr; }
  .toc-number { grid-row: auto; }
  .section-head { grid-template-columns: 1fr; }
  .qno, .answer { text-align: left; }
  .report-table, .agent-table { min-width: 760px; }
}
`;
