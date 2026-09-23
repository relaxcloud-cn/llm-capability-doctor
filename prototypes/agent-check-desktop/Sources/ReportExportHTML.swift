import Foundation

// 单文件自包含 HTML 报告：与 GUI 报告页同源同结构（大结论 → 模块 → 分组 → 明细），
// 双击可开、可直接发给别人，不依赖网络与外部资源。
// 数据全部来自 RealModuleView.exportModule（页面同一份数据），保证「所见即所导」。
//
// 设计口径：这是一份「检测报告」，读者第一眼要看到结论。
// 刊头（hero）用总结论定色，是全页唯一的视觉重心；其余部分保持安静克制，
// 状态色只表达状态（绿=通过、琥珀=受限/未通过、红=阻断、灰=未知），不作装饰。
func renderReportHTML(record: RunRecord) -> String {
  let modules = CheckModule.testModules.filter { record.modules.contains($0) }
  let sections = modules.map { module in
    RealModuleView(module: module, record: record).exportModule
  }

  var body = ""
  body += heroHTML(record: record)
  body += tocHTML(modules: sections)
  for section in sections {
    body += moduleHTML(section)
  }
  body += footerHTML(record: record)

  return """
    <!doctype html>
    <html lang="zh-CN">
    <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width,initial-scale=1">
    <title>AgentCheck 检测报告 · \(escape(record.service.model))</title>
    <style>
    \(css)
    </style>
    </head>
    <body>
    <main>
    \(body)
    </main>
    </body>
    </html>
    """
}

// MARK: - 刊头与一览

private func heroHTML(record: RunRecord) -> String {
  let klass = heroClass(record.outcome)
  let icon = symbol(for: klass)
  return """
  <header class="hero \(klass)">
    <div class="hero-head">
      <span class="hero-ico">\(icon)</span>
      <div>
        <p class="eyebrow">AgentCheck 模型检测报告</p>
        <h1>\(escape(record.admissionTitle))</h1>
        <p class="explain">\(escape(record.admissionExplanation))</p>
      </div>
    </div>
    <dl class="meta-bar">
      <div><dt>模型</dt><dd>\(escape(record.service.model))</dd></div>
      <div><dt>服务地址</dt><dd>\(escape(record.service.host))</dd></div>
      <div><dt>检测时间</dt><dd>\(escape(record.date.formatted(date: .long, time: .shortened)))</dd></div>
      <div><dt>模块完成</dt><dd>\(record.completed.count)/\(record.modules.count)</dd></div>
    </dl>
  </header>
  """
}

// 模块一览：一屏看清六个模块各自结论，点击直达。
private func tocHTML(modules: [RealModuleView.ExportModule]) -> String {
  let chips = modules.map { section in
    """
    <a class="chip \(section.bannerClass)" href="#mod-\(escape(section.title))">
      <span class="chip-ico">\(symbol(for: section.bannerClass))</span>
      <span class="chip-body">
        <span class="chip-name">\(escape(section.title))</span>
        <span class="chip-head">\(escape(section.headline))</span>
      </span>
    </a>
    """
  }.joined(separator: "\n")
  return """
  <nav class="toc">
    <h2>这次检测的结论一览</h2>
    <div class="chips">
    \(chips)
    </div>
  </nav>
  """
}

// MARK: - 模块与明细

private func moduleHTML(_ section: RealModuleView.ExportModule) -> String {
  var groups = ""
  for group in section.groups {
    let samples = group.samples.map(sampleHTML).joined(separator: "\n")
    groups += """
      <div class="group">
        <div class="group-head \(verdictClass(group.verdict))">
          <span class="dot">●</span>
          <span class="gname">\(escape(group.name))</span>
          <span class="count">\(escape(group.countText))</span>
        </div>
        <p class="gnote">\(escape(group.note))</p>
    \(samples)
      </div>
    """
  }
  if groups.isEmpty { groups = "<p class=\"gnote empty\">没有找到该模块的任务级证据。</p>" }
  let scope = section.scope.map { "<p class=\"scope\">\(escape($0))</p>" } ?? ""
  let meta = section.meta.map { "<span class=\"banner-meta\">\(escape($0))</span>" } ?? ""
  return """
  <section class="module" id="mod-\(escape(section.title))">
    <div class="banner \(section.bannerClass)">
      <div class="banner-head">
        <span class="banner-ico">\(symbol(for: section.bannerClass))</span>
        <h2>\(escape(section.title))：\(escape(section.headline))</h2>
        \(meta)
      </div>
      <p class="banner-desc">\(escape(section.detail))</p>
    </div>
    <div class="card">
  \(groups)
    </div>
  \(scope)
  </section>
  """
}

private func sampleHTML(_ sample: RealModuleView.ExportSample) -> String {
  // 与页面同一套口径：①检测方式 ②判定方式 默认展示；
  // ③原始请求 ④原始返回（JSON）默认折叠；智能体任务显示「任务执行记录」。
  let judge = sample.judge + (sample.note.isEmpty ? "" : "\n本次结果：\(sample.note)")
  let requestTitle = sample.isTaskStyle ? "查看原始请求" : "查看原始 curl 请求"
  let responseTitle = sample.isTaskStyle ? "任务执行记录（JSON）" : "原始返回（JSON）"
  let responseValue =
    sample.rawResponse.isEmpty
    ? (sample.isTaskStyle ? "本次没有取得任务执行记录" : "本次没有取得响应体")
    : sample.rawResponse
  return """
  <div class="sample">
    <div class="sample-head">
      <span class="pill \(verdictClass(sample.verdict))">\(escape(sample.verdictText))</span>
      <span class="sname">\(escape(sample.name))</span>
    </div>
    <div class="info"><h4>检测方式</h4><p>\(escape(sample.how))</p></div>
    <div class="info"><h4>判定方式</h4><p>\(escape(judge))</p></div>
    <details><summary>\(requestTitle)</summary><pre>\(escape(sample.isTaskStyle ? sample.request : sample.curl))</pre></details>
    <details><summary>\(responseTitle)</summary><pre>\(escape(responseValue))</pre></details>
  </div>
  """
}

private func footerHTML(record: RunRecord) -> String {
  """
  <footer>
    <p>本报告由 AgentCheck 桌面端导出，与报告页同源生成；判定口径以检测程序为准。</p>
    <p class="meta">导出于 \(escape(Date().formatted(date: .long, time: .shortened))) · 模型 \(escape(record.service.model))</p>
  </footer>
  """
}

// MARK: - 状态与工具

// 状态色与页面同一套语义；刊头跟随总体结论定色。
private func heroClass(_ outcome: Outcome) -> String {
  switch outcome {
  case .usable: return "v-pass"
  case .limited: return "v-fail"
  case .blocked: return "v-block"
  case .inconclusive: return "v-unknown"
  }
}

private func verdictClass(_ verdict: RealTaskReport.Verdict) -> String {
  switch verdict {
  case .pass: return "v-pass"
  case .fail: return "v-fail"
  case .unverified: return "v-unknown"
  }
}

private func symbol(for klass: String) -> String {
  switch klass {
  case "v-pass": return "✔"
  case "v-fail": return "!"
  case "v-block": return "✕"
  default: return "?"
  }
}

private func escape(_ text: String) -> String {
  text
    .replacingOccurrences(of: "&", with: "&amp;")
    .replacingOccurrences(of: "<", with: "&lt;")
    .replacingOccurrences(of: ">", with: "&gt;")
    .replacingOccurrences(of: "\"", with: "&quot;")
}

// MARK: - 样式

private let css = """
:root{
  --ink:#1b2733;--muted:#5d6b78;--faint:#8d99a5;--line:#e3e9ef;--canvas:#f5f7f9;
  --accent:#1769aa;
  --pass:#1c7f47;--pass-bg:#e9f6ef;
  --warn:#9c5d08;--warn-bg:#fdf3e3;
  --block:#b3352c;--block-bg:#fdefee;
  --unknown:#6f7b86;--unknown-bg:#eef1f4;
}
*{box-sizing:border-box}
body{margin:0;background:var(--canvas);color:var(--ink);font:14px/1.75 -apple-system,BlinkMacSystemFont,"PingFang SC","Segoe UI",sans-serif;-webkit-font-smoothing:antialiased}
main{max-width:920px;margin:0 auto;padding:36px 22px 56px}

/* ---- 刊头：结论是这份报告的身份 ---- */
.hero{background:#fff;border:1px solid var(--line);border-radius:14px;overflow:hidden;margin-bottom:16px}
.hero-head{display:flex;gap:18px;align-items:flex-start;padding:28px 30px 22px}
.hero-ico{flex:none;width:46px;height:46px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:21px;font-weight:700;margin-top:4px}
.hero h1{margin:2px 0 6px;font-size:25px;line-height:1.4;letter-spacing:.2px}
.eyebrow{margin:0;font-size:11px;font-weight:600;letter-spacing:.22em;color:var(--faint)}
.explain{margin:0;color:var(--muted);font-size:14px;max-width:640px}
.meta-bar{display:grid;grid-template-columns:repeat(4,1fr);gap:0;margin:0;padding:14px 30px;border-top:1px solid var(--line);background:#fafcfd}
.meta-bar div{padding:0 18px;border-left:1px solid var(--line)}
.meta-bar div:first-child{border-left:none;padding-left:0}
.meta-bar dt{font-size:11px;color:var(--faint);letter-spacing:.08em;margin:0 0 2px}
.meta-bar dd{margin:0;font-size:12.5px;font-weight:600;color:var(--ink);word-break:break-all}

/* ---- 模块一览：一屏结论 ---- */
.toc{margin-bottom:30px}
.toc h2{margin:0 0 10px;font-size:13px;color:var(--muted);letter-spacing:.06em}
.chips{display:grid;grid-template-columns:repeat(auto-fill,minmax(262px,1fr));gap:10px}
.chip{display:flex;gap:11px;align-items:center;padding:13px 15px;background:#fff;border:1px solid var(--line);border-radius:11px;text-decoration:none;color:var(--ink);transition:border-color .15s}
.chip:hover{border-color:var(--accent)}
.chip-ico{flex:none;width:26px;height:26px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:12.5px;font-weight:700}
.chip-body{min-width:0}
.chip-name{display:block;font-size:13px;font-weight:600;line-height:1.4}
.chip-head{display:block;font-size:11.5px;color:var(--muted);line-height:1.5}

/* ---- 模块横幅：左侧状态轨 ---- */
.module{margin-bottom:34px}
.banner{border-radius:12px;padding:16px 20px 15px;border:1px solid transparent;border-left-width:4px}
.banner-head{display:flex;align-items:center;gap:10px;flex-wrap:wrap}
.banner-ico{flex:none;width:24px;height:24px;border-radius:50%;display:flex;align-items:center;justify-content:center;font-size:12px;font-weight:700}
.banner h2{margin:0;font-size:16.5px;font-weight:650;letter-spacing:.2px}
.banner-meta{margin-left:auto;font-size:11.5px;padding:3px 10px;border-radius:99px;font-variant-numeric:tabular-nums}
.banner-desc{margin:7px 0 2px 34px;font-size:13px}

/* ---- 分组与明细 ---- */
.card{background:#fff;border:1px solid var(--line);border-radius:12px;padding:8px 20px 4px}
.group{padding:14px 0 10px;border-bottom:1px solid var(--line)}
.group:last-child{border-bottom:none}
.group-head{display:flex;align-items:center;gap:9px;padding:8px 12px;border-radius:8px;background:#f7fafc}
.dot{font-size:9px;line-height:1}
.gname{font-weight:600;font-size:13.5px}
.count{margin-left:auto;font-size:12px;color:var(--muted);font-variant-numeric:tabular-nums}
.gnote{margin:8px 2px 4px;color:var(--muted);font-size:12px}
.gnote.empty{padding:16px 2px}
.sample{margin:4px 0 12px 5px;padding:2px 0 0 15px;border-left:2px solid #e8edf2}
.sample-head{display:flex;align-items:center;gap:9px;margin:10px 0 9px}
.sname{font-size:13px;font-weight:500}
.pill{flex:none;font-size:11px;font-weight:600;padding:2px 10px;border-radius:99px}
.info{margin:0 0 9px}
.info h4{margin:0 0 2px;font-size:11px;font-weight:600;color:var(--faint);letter-spacing:.1em}
.info p{margin:0;font-size:12.5px;white-space:pre-wrap}
details{margin:7px 0}
summary{list-style:none;cursor:pointer;user-select:none;display:inline-flex;align-items:center;gap:5px;font-size:12px;font-weight:500;color:var(--accent)}
summary::-webkit-details-marker{display:none}
summary::before{content:"▸";font-size:10px;transition:transform .15s}
details[open] summary::before{transform:rotate(90deg)}
pre{margin:7px 0 10px;padding:11px 13px;background:#f8fafb;border:1px solid #e8edf2;border-radius:8px;font:11px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;overflow-x:auto;white-space:pre-wrap;word-break:break-all}
.scope{margin:9px 2px 0;color:var(--faint);font-size:11.5px}

/* ---- 页脚 ---- */
footer{margin-top:6px;padding-top:14px;border-top:1px solid var(--line);color:var(--faint);font-size:11.5px}
footer p{margin:0 0 3px}

/* ---- 状态着色（图标圆底 / 横幅底色 / 文字色）---- */
.v-pass .hero-ico,.v-pass .chip-ico,.v-pass .banner-ico{color:var(--pass);background:var(--pass-bg)}
.v-pass .dot{color:var(--pass)}
.v-pass .pill{color:var(--pass);background:var(--pass-bg)}
.banner.v-pass{background:#f4fbf7;border-color:#d5eadf}
.banner.v-pass .banner-meta{color:var(--pass);background:var(--pass-bg)}
.hero.v-pass{background:linear-gradient(180deg,#f2faf5 0%,#fff 100%)}
.hero.v-pass .hero-ico{background:var(--pass);color:#fff}

.v-fail .hero-ico,.v-fail .chip-ico,.v-fail .banner-ico{color:var(--warn);background:var(--warn-bg)}
.v-fail .dot{color:var(--warn)}
.v-fail .pill{color:var(--warn);background:var(--warn-bg)}
.banner.v-fail{background:#fdf9f1;border-color:#f2e3c4}
.banner.v-fail .banner-meta{color:var(--warn);background:var(--warn-bg)}
.hero.v-fail{background:linear-gradient(180deg,#fdf8ef 0%,#fff 100%)}
.hero.v-fail .hero-ico{background:var(--warn);color:#fff}

.v-block .hero-ico,.v-block .chip-ico,.v-block .banner-ico{color:var(--block);background:var(--block-bg)}
.v-block .pill{color:var(--block);background:var(--block-bg)}
.banner.v-block{background:#fdf4f3;border-color:#f1d2cf}
.banner.v-block .banner-meta{color:var(--block);background:var(--block-bg)}
.banner.v-block{border-left-color:var(--block)}
.hero.v-block{background:linear-gradient(180deg,#fdf3f2 0%,#fff 100%)}
.hero.v-block .hero-ico{background:var(--block);color:#fff}

.v-unknown .hero-ico,.v-unknown .chip-ico,.v-unknown .banner-ico{color:var(--unknown);background:var(--unknown-bg)}
.v-unknown .dot{color:var(--unknown)}
.v-unknown .pill{color:var(--unknown);background:var(--unknown-bg)}
.banner.v-unknown{background:#f8fafb;border-color:#e5eaef}
.banner.v-unknown .banner-meta{color:var(--unknown);background:var(--unknown-bg)}
.hero.v-unknown{background:linear-gradient(180deg,#f6f8fa 0%,#fff 100%)}
.hero.v-unknown .hero-ico{background:var(--unknown);color:#fff}

.banner.v-pass{border-left-color:var(--pass)}
.banner.v-fail{border-left-color:var(--warn)}
.banner.v-unknown{border-left-color:#b9c4cd}

@media (max-width:640px){
  .meta-bar{grid-template-columns:1fr 1fr}
  .meta-bar div{padding:6px 14px}
  .meta-bar div:nth-child(odd){border-left:none;padding-left:0}
  .banner-desc{margin-left:0}
}
@media print{
  body{background:#fff}
  main{padding:12px 0}
}
"""
