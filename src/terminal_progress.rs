//! 终端进度渲染：把 ProgressEvent 流渲染成与 GUI 一致的进度界面。
//!
//! 交互终端（TTY）下维护一个原地重绘的区块：每个检测模块一行，
//! 正在执行的模块下方显示当前小项，底部是总进度条；后台线程按
//! 120ms 刷新转动动画与耗时。
//! 非 TTY（管道、CI、重定向）退化为每个事件一行纯文本。
//! 全部输出走调用方给的 writer（main 里接 stderr），不污染 stdout。

use crate::cli::{
    ProgressEvent, ProgressPhase, ProgressPlanItem, ProgressSink, ProgressItemStats,
    module_display_name, module_state_label,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BAR_WIDTH: usize = 24;
const MAX_LINE_WIDTH: usize = 100;
const TICK: Duration = Duration::from_millis(120);
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModuleStatus {
    Waiting,
    Running,
    Done {
        state: String,
        reason: String,
        /// 三段判定计数（通过/未通过/需人工确认），事件未携带统计时为 None。
        stats: Option<ProgressItemStats>,
    },
}

#[derive(Debug, Clone)]
struct ModuleView {
    id: String,
    status: ModuleStatus,
    items: Vec<ProgressPlanItem>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    /// 模块内样本进度（ProgressEvent.detail_*）。
    detail_index: usize,
    detail_total: usize,
}

impl ModuleView {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            status: ModuleStatus::Waiting,
            items: Vec::new(),
            started: None,
            elapsed: None,
            detail_index: 0,
            detail_total: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveDetail {
    module_pos: usize,
    detail_id: String,
    label: String,
    index: usize,
    total: usize,
}

/// 渲染状态，可在线程间共享（后台线程负责动画帧）。
struct Inner {
    interactive: bool,
    color: bool,
    model: String,
    endpoint: String,
    line_width: usize,
    run_started: Option<Instant>,
    modules: Vec<ModuleView>,
    positions: BTreeMap<String, usize>,
    total: usize,
    completed: usize,
    active: Option<ActiveDetail>,
    drawn: usize,
    spin_frame: usize,
    last_plain_detail: Option<String>,
}

/// 把 ProgressEvent 渲染到终端的进度显示端。
pub struct TerminalProgressSink<W: Write> {
    inner: Arc<Mutex<Inner>>,
    writer: W,
    ticker_stop: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

impl<W: Write> TerminalProgressSink<W> {
    pub fn new(
        writer: W,
        interactive: bool,
        color: bool,
        model: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                interactive,
                color,
                model: model.into(),
                endpoint: endpoint.into(),
                line_width: term_width().saturating_sub(1).clamp(20, MAX_LINE_WIDTH),
                run_started: None,
                modules: Vec::new(),
                positions: BTreeMap::new(),
                total: 0,
                completed: 0,
                active: None,
                drawn: 0,
                spin_frame: 0,
                last_plain_detail: None,
            })),
            writer,
            ticker_stop: Arc::new(AtomicBool::new(false)),
            ticker: None,
        }
    }

    fn start_ticker(&mut self) {
        if self.ticker.is_some() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        let stop = Arc::clone(&self.ticker_stop);
        self.ticker = Some(std::thread::spawn(move || {
            let mut stderr = std::io::stderr();
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                let mut guard = inner.lock().unwrap();
                guard.spin_frame = guard.spin_frame.wrapping_add(1);
                guard.redraw(&mut stderr);
            }
        }));
    }

    fn stop_ticker(&mut self) {
        self.ticker_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.ticker.take() {
            let _ = handle.join();
        }
    }
}

impl<W: Write> Drop for TerminalProgressSink<W> {
    fn drop(&mut self) {
        self.stop_ticker();
    }
}

impl<W: Write> ProgressSink for TerminalProgressSink<W> {
    fn emit(&mut self, event: ProgressEvent) {
        let finished = matches!(
            event.phase,
            ProgressPhase::RunCompleted | ProgressPhase::RunStopped
        );
        let mut spawn_ticker = false;
        {
            let mut inner = self.inner.lock().unwrap();
            let first = inner.run_started.is_none();
            inner.handle(&event);
            if !inner.interactive {
                inner.emit_plain(&mut self.writer, &event);
            } else if first && matches!(event.phase, ProgressPhase::RunStarted) {
                inner.print_header(&mut self.writer);
                inner.redraw(&mut self.writer);
                spawn_ticker = true;
            } else {
                inner.redraw(&mut self.writer);
            }
        }
        if spawn_ticker {
            self.start_ticker();
        }
        if finished {
            self.stop_ticker();
        }
    }
}

impl Inner {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.into()
        }
    }

    fn elapsed_text(&self) -> String {
        let elapsed = self
            .run_started
            .map(|started| started.elapsed())
            .unwrap_or_default();
        format_duration(elapsed)
    }

    fn module_mut(&mut self, module_id: &str) -> usize {
        if let Some(pos) = self.positions.get(module_id) {
            return *pos;
        }
        // 并发预检先于一切模块执行，永远排在列表最前，不随事件到达顺序漂移。
        if module_id == "preflight" && !self.modules.is_empty() {
            self.modules.insert(0, ModuleView::new(module_id));
            for (index, module) in self.modules.iter().enumerate() {
                self.positions.insert(module.id.clone(), index);
            }
            return 0;
        }
        let pos = self.modules.len();
        self.modules.push(ModuleView::new(module_id));
        self.positions.insert(module_id.into(), pos);
        pos
    }

    fn seed_modules(&mut self, event: &ProgressEvent) {
        let Some(ids) = &event.modules else { return };
        for id in ids {
            self.module_mut(id);
        }
    }

    fn detail_label(&self, event: &ProgressEvent, pos: usize) -> String {
        let detail_id = event.detail_id.as_deref().unwrap_or("");
        let items = &self.modules[pos].items;
        let index = event.detail_index.unwrap_or(0);
        let item = items
            .iter()
            .position(|item| item.id == detail_id)
            .or_else(|| {
                items
                    .iter()
                    .position(|item| detail_id.starts_with(&item.id))
            });
        if let Some(item_pos) = item {
            let offset: usize = items[..item_pos].iter().map(|item| item.total).sum();
            let item = &items[item_pos];
            let done = index.saturating_sub(offset).min(item.total);
            return format!("{} {} {}/{}", item.id, item.name, done, item.total);
        }
        match (event.detail_index, event.detail_total) {
            (Some(index), Some(total)) if !detail_id.is_empty() => {
                format!("{detail_id} {index}/{total}")
            }
            _ => event.message.clone(),
        }
    }

    fn handle(&mut self, event: &ProgressEvent) {
        match event.phase {
            ProgressPhase::RunStarted => {
                self.run_started = Some(Instant::now());
                self.total = event.total;
                self.seed_modules(event);
            }
            ProgressPhase::ModuleStarted => {
                let Some(id) = event.module_id.as_deref() else {
                    return;
                };
                // 没经过 ModuleStarted 就进入 Running 的行（如并发预检只发 Progress），
                // 在真正的模块开始后回到等待态，避免残留一行假"进行中"。
                for module in &mut self.modules {
                    if module.status == ModuleStatus::Running && module.started.is_none() {
                        module.status = ModuleStatus::Waiting;
                    }
                }
                let pos = self.module_mut(id);
                let module = &mut self.modules[pos];
                module.status = ModuleStatus::Running;
                module.started = Some(Instant::now());
                module.detail_index = 0;
                module.detail_total = 0;
                if let Some(items) = &event.items {
                    module.items = items.clone();
                }
                self.active = Some(ActiveDetail {
                    module_pos: pos,
                    detail_id: String::new(),
                    label: "准备中".into(),
                    index: 0,
                    total: 0,
                });
            }
            ProgressPhase::ModuleProgress => {
                let Some(id) = event.module_id.as_deref() else {
                    return;
                };
                let pos = self.module_mut(id);
                let label = self.detail_label(event, pos);
                let module = &mut self.modules[pos];
                // 并发预检等只发 Progress 不发 Started 的前置步骤：探测期间显示进行中。
                if module.status == ModuleStatus::Waiting {
                    module.status = ModuleStatus::Running;
                }
                module.detail_index = event.detail_index.unwrap_or(0);
                module.detail_total = event.detail_total.unwrap_or(0);
                self.active = Some(ActiveDetail {
                    module_pos: pos,
                    detail_id: event.detail_id.clone().unwrap_or_default(),
                    label,
                    index: module.detail_index,
                    total: module.detail_total,
                });
            }
            ProgressPhase::ModuleCompleted => {
                let Some(id) = event.module_id.as_deref() else {
                    return;
                };
                let pos = self.module_mut(id);
                let elapsed = self.modules[pos].started.map(|started| started.elapsed());
                self.modules[pos].status = ModuleStatus::Done {
                    state: event.state.clone().unwrap_or_else(|| "unknown".into()),
                    reason: event.message.clone(),
                    stats: event.item_stats.clone(),
                };
                self.modules[pos].elapsed = elapsed;
                self.completed = event.index;
                self.active = None;
            }
            ProgressPhase::RunCompleted | ProgressPhase::RunStopped => {
                self.active = None;
            }
        }
        // 每个事件顺手推进一帧动画，后台线程停顿时也有动感。
        self.spin_frame = self.spin_frame.wrapping_add(1);
    }

    fn module_line(&self, pos: usize) -> String {
        let module = &self.modules[pos];
        let name = module_display_name(&module.id);
        match &module.status {
            ModuleStatus::Waiting => {
                let marker = self.paint("2", "○");
                let state = self.paint("2", "等待中");
                format!("{marker} {name}  {state}")
            }
            ModuleStatus::Running => {
                let spin = SPINNER[self.spin_frame % SPINNER.len()];
                let marker = self.paint("33", spin);
                let counts = if module.detail_total > 0 {
                    format!(" {}/{}", module.detail_index, module.detail_total)
                } else {
                    String::new()
                };
                let state = self.paint("33", &format!("进行中{counts}"));
                format!("{marker} {name}  {state}")
            }
            ModuleStatus::Done { state, stats, .. } => {
                // 统一口径：完成后显示"X通过 Y未通过 Z需人工确认"三段计数，
                // 分段配色：通过绿、未通过红、需人工确认黄。
                let (marker_color, marker, text) = match stats {
                    Some(stats) => {
                        let text = format!(
                            "{} {} {}",
                            self.paint("32", &format!("{}通过", stats.passed)),
                            self.paint("31", &format!("{}未通过", stats.failed)),
                            self.paint("33", &format!("{}需人工确认", stats.needs_manual)),
                        );
                        if stats.failed > 0 {
                            ("31", "✗", text)
                        } else if stats.needs_manual > 0 {
                            ("33", "●", text)
                        } else {
                            ("32", "✓", text)
                        }
                    }
                    None => match state.as_str() {
                        "pass" => ("32", "✓", "通过".to_string()),
                        "fail" => ("31", "✗", "未通过".to_string()),
                        _ => ("33", "●", module_state_label(state).to_string()),
                    },
                };
                let marker = self.paint(marker_color, marker);
                let styled = text;
                let elapsed = module
                    .elapsed
                    .map(|elapsed| format!(" {}", self.paint("2", &format_duration(elapsed))))
                    .unwrap_or_default();
                format!("{marker} {name}  {styled}{elapsed}")
            }
        }
    }

    /// 运行中模块的小项明细：小项总数与样本总数一致时按偏移量算出每项
    /// 的 x/y 完成度；对不上的模块（性能按维度、Agent 按调用计数）按
    /// detail_id 或序号定位当前项，只画状态标记。
    fn item_lines(&self, pos: usize) -> Vec<String> {
        let module = &self.modules[pos];
        let Some(active) = self.active.as_ref() else {
            return Vec::new();
        };
        if active.module_pos != pos || module.items.is_empty() {
            return Vec::new();
        }
        let spin = SPINNER[self.spin_frame % SPINNER.len()];
        let sum: usize = module.items.iter().map(|item| item.total).sum();
        let use_counts = sum == active.total && active.total > 0;
        let active_pos = module
            .items
            .iter()
            .position(|item| item.id == active.detail_id)
            .or_else(|| {
                module.items.iter().position(|item| {
                    !active.detail_id.is_empty() && active.detail_id.starts_with(&item.id)
                })
            })
            .unwrap_or_else(|| active.index.min(module.items.len().saturating_sub(1)));
        let mut offset = 0usize;
        let mut lines = Vec::with_capacity(module.items.len());
        for (item_pos, item) in module.items.iter().enumerate() {
            let (marker, tail) = if use_counts {
                let done = active.index.saturating_sub(offset).min(item.total);
                let finished = done >= item.total && active.index >= offset;
                let running =
                    !finished && active.index >= offset && active.index < offset + item.total;
                offset += item.total;
                if finished {
                    (
                        self.paint("32", "✓"),
                        self.paint("2", &format!("{}/{}", item.total, item.total)),
                    )
                } else if running {
                    (self.paint("36", spin), format!("{done}/{}", item.total))
                } else {
                    (self.paint("2", "○"), String::new())
                }
            } else if item_pos < active_pos
                || (item_pos == active_pos && active.detail_id.is_empty())
            {
                (self.paint("2", "○"), String::new())
            } else if item_pos == active_pos {
                (self.paint("36", spin), String::new())
            } else {
                (self.paint("2", "○"), String::new())
            };
            let tail = if tail.is_empty() {
                tail
            } else {
                format!(" {tail}")
            };
            lines.push(format!("   {marker} {} {}{tail}", item.id, item.name));
        }
        lines
    }

    fn block(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for pos in 0..self.modules.len() {
            lines.push(self.module_line(pos));
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.module_pos == pos)
            {
                let items = self.item_lines(pos);
                if items.is_empty() {
                    let label = self
                        .active
                        .as_ref()
                        .map(|active| self.paint("36", &active.label))
                        .unwrap_or_default();
                    lines.push(format!("   └ {label}"));
                } else {
                    lines.extend(items);
                }
            }
        }
        lines.push(String::new());
        lines.push(self.bar_line());
        lines
    }

    fn bar_line(&self) -> String {
        let detail_fraction = self
            .active
            .as_ref()
            .map(|active| {
                if active.total == 0 {
                    0.0
                } else {
                    (active.index as f64 / active.total as f64).min(1.0)
                }
            })
            .unwrap_or(0.0);
        let fraction = if self.total == 0 {
            0.0
        } else {
            ((self.completed as f64 + detail_fraction) / self.total as f64).min(1.0)
        };
        let filled = (fraction * BAR_WIDTH as f64) as usize;
        let bar = format!(
            "{}{}",
            self.paint("32", &"█".repeat(filled)),
            self.paint("2", &"░".repeat(BAR_WIDTH - filled))
        );
        format!(
            "{bar} {}  {}",
            self.paint("36", &format!("{:.0}%", fraction * 100.0)),
            self.paint("2", &format!("已用 {}", self.elapsed_text()))
        )
    }

    fn redraw(&mut self, writer: &mut dyn Write) {
        let lines = self.block();
        let mut output = String::new();
        if self.drawn > 0 {
            output.push_str(&format!("\x1b[{}A", self.drawn));
        }
        for line in &lines {
            output.push_str("\r\x1b[2K");
            output.push_str(&truncate(line, self.line_width));
            output.push('\n');
        }
        // 清掉上一帧在区块下方可能残留的行（换行溢出、模块行变少）。
        output.push_str("\x1b[J");
        self.drawn = lines.len();
        let _ = writer.write_all(output.as_bytes());
        let _ = writer.flush();
    }

    fn print_line(&self, writer: &mut dyn Write, line: &str) {
        let _ = writeln!(writer, "{line}");
        let _ = writer.flush();
    }

    fn print_header(&self, writer: &mut dyn Write) {
        self.print_line(
            writer,
            &format!(
                "{}  {}",
                self.paint("34;1", "AgentCheck"),
                self.paint("2", "模型服务能力检测")
            ),
        );
        self.print_line(
            writer,
            &format!(
                "{}  {}   {}  {}",
                self.paint("2", "模型"),
                self.model,
                self.paint("2", "地址"),
                truncate(&self.endpoint, 60)
            ),
        );
        self.print_line(writer, "");
    }

    fn emit_plain(&mut self, writer: &mut dyn Write, event: &ProgressEvent) {
        match event.phase {
            ProgressPhase::RunStarted => {
                self.print_line(
                    writer,
                    &format!("[开始] 模型 {} · 共 {} 个检测项目", self.model, event.total),
                );
            }
            ProgressPhase::ModuleStarted => {
                let name = event
                    .module_id
                    .as_deref()
                    .map(module_display_name)
                    .unwrap_or("未知项目");
                self.print_line(
                    writer,
                    &format!("[{}/{}] {name} 开始检测", event.index + 1, event.total),
                );
            }
            ProgressPhase::ModuleProgress => {
                if let Some(active) = &self.active {
                    let label = active.label.clone();
                    // 同一样本前后各发一次事件，跳过与上一条相同的行。
                    if self.last_plain_detail.as_deref() == Some(label.as_str()) {
                        return;
                    }
                    self.last_plain_detail = Some(label.clone());
                    self.print_line(
                        writer,
                        &format!("[{}/{}]   {label}", event.index + 1, event.total),
                    );
                }
            }
            ProgressPhase::ModuleCompleted => {
                let name = event
                    .module_id
                    .as_deref()
                    .map(module_display_name)
                    .unwrap_or("未知项目");
                let state_label = event
                    .state
                    .as_deref()
                    .map(module_state_label)
                    .unwrap_or("未知");
                let state = match &event.item_stats {
                    Some(stats) => format!(
                        "{}通过 {}未通过 {}需人工确认",
                        stats.passed, stats.failed, stats.needs_manual
                    ),
                    None => state_label.to_string(),
                };
                let reason = if event.message.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", event.message)
                };
                self.print_line(
                    writer,
                    &format!("[{}/{}] {name} {state}{reason}", event.index, event.total),
                );
            }
            ProgressPhase::RunCompleted => {
                self.print_line(
                    writer,
                    &format!("[完成] 全部检测结束 · 用时 {}", self.elapsed_text()),
                );
            }
            ProgressPhase::RunStopped => {
                self.print_line(
                    writer,
                    &format!("[停止] {} · 用时 {}", event.message, self.elapsed_text()),
                );
            }
        }
    }
}

#[cfg(unix)]
fn stty_width() -> Option<usize> {
    for flag in ["-f", "-F"] {
        if let Ok(output) = std::process::Command::new("stty")
            .args([flag, "/dev/tty", "size"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                if let Some(cols) = text
                    .split_whitespace()
                    .nth(1)
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|cols| *cols > 0)
                {
                    return Some(cols);
                }
            }
        }
    }
    None
}

#[cfg(not(unix))]
fn stty_width() -> Option<usize> {
    None
}

fn term_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|width: &usize| *width > 0)
        .or_else(stty_width)
        .unwrap_or(80)
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3600 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{}m", seconds / 3600, (seconds % 3600) / 60)
    }
}

fn truncate(text: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut cut = text.len();
    let mut in_escape = false;
    for (pos, ch) in text.char_indices() {
        if ch == '\x1b' {
            in_escape = true;
        }
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        width += if (ch as u32) >= 0x2e80 { 2 } else { 1 };
        if width > max_width {
            cut = pos;
            break;
        }
    }
    if cut < text.len() {
        let mut trimmed = text[..cut].to_string();
        if text.contains('\x1b') {
            trimmed.push_str("\x1b[0m");
        }
        trimmed.push('…');
        trimmed
    } else {
        text.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{ProgressItemStats, ProgressPhase};

    fn event(phase: ProgressPhase, module: Option<&str>) -> ProgressEvent {
        ProgressEvent {
            phase,
            module_id: module.map(str::to_string),
            index: 0,
            total: 2,
            state: None,
            message: String::new(),
            detail_index: None,
            detail_total: None,
            detail_id: None,
            items: None,
            modules: None,
            item_stats: None,
        }
    }

    #[test]
    fn plain_mode_prints_one_line_per_event() {
        let mut buffer = Vec::new();
        {
            let mut sink =
                TerminalProgressSink::new(&mut buffer, false, false, "model-a", "http://x");
            sink.emit(event(ProgressPhase::RunStarted, None));
            let mut started = event(ProgressPhase::ModuleStarted, Some("capability"));
            started.items = crate::cli::module_plan_items("capability");
            sink.emit(started);
            let mut progress = event(ProgressPhase::ModuleProgress, Some("capability"));
            progress.detail_id = Some("C03".into());
            progress.detail_index = Some(76);
            progress.detail_total = Some(144);
            sink.emit(progress);
            let mut done = event(ProgressPhase::ModuleCompleted, Some("capability"));
            done.index = 1;
            done.state = Some("fail".into());
            done.message = "144 个样本未通过".into();
            sink.emit(done);
            sink.emit(event(ProgressPhase::RunCompleted, None));
        }
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("模型 model-a · 共 2 个检测项目"));
        assert!(output.contains("[1/2] 模型能力跑分 开始检测"));
        assert!(output.contains("C03 工具选择与参数填写 12/20"));
        assert!(output.contains("[1/2] 模型能力跑分 未通过（144 个样本未通过）"));
        assert!(output.contains("[完成] 全部检测结束"));
    }

    #[test]
    fn detail_label_uses_item_prefix_for_sub_turns() {
        let sink = TerminalProgressSink::new(Vec::new(), false, false, "m", "e");
        let mut started = event(ProgressPhase::ModuleStarted, Some("agent"));
        started.items = crate::cli::module_plan_items("agent");
        sink.inner.lock().unwrap().handle(&started);
        let mut progress = event(ProgressPhase::ModuleProgress, Some("agent"));
        progress.detail_id = Some("T3-A-call-2".into());
        progress.detail_index = Some(5);
        progress.detail_total = Some(10);
        let label = sink.inner.lock().unwrap().detail_label(&progress, 0);
        assert!(label.starts_with("T3-A 使用工具返回驱动下一步"));
    }

    #[test]
    fn interactive_mode_writes_ansi_redraw() {
        let mut buffer = Vec::new();
        {
            let mut sink = TerminalProgressSink::new(&mut buffer, true, false, "m", "e");
            sink.emit(event(ProgressPhase::RunStarted, None));
            sink.emit(event(ProgressPhase::ModuleStarted, Some("ingress")));
        }
        let output = String::from_utf8(buffer).unwrap();
        assert!(output.contains("AgentCheck"));
        assert!(output.contains("\x1b["));
        assert!(output.contains("服务接入"));
    }

    #[test]
    fn running_module_shows_sample_counts() {
        let sink = TerminalProgressSink::new(Vec::new(), true, false, "m", "e");
        sink.inner
            .lock()
            .unwrap()
            .handle(&event(ProgressPhase::RunStarted, None));
        sink.inner
            .lock()
            .unwrap()
            .handle(&event(ProgressPhase::ModuleStarted, Some("specification")));
        let mut progress = event(ProgressPhase::ModuleProgress, Some("specification"));
        progress.detail_index = Some(5);
        progress.detail_total = Some(18);
        let mut inner = sink.inner.lock().unwrap();
        inner.handle(&progress);
        let line = inner.module_line(0);
        assert!(line.contains("进行中 5/18"));
    }

    #[test]
    fn done_module_shows_three_way_counts() {
        // 统一口径：完成后不再出现"待确认"字样，改为三段计数。
        let sink = TerminalProgressSink::new(Vec::new(), true, false, "m", "e");
        sink.inner
            .lock()
            .unwrap()
            .handle(&event(ProgressPhase::RunStarted, None));
        sink.inner
            .lock()
            .unwrap()
            .handle(&event(ProgressPhase::ModuleStarted, Some("capability")));
        let mut completed = event(ProgressPhase::ModuleCompleted, Some("capability"));
        completed.state = Some("inconclusive".into());
        completed.item_stats = Some(ProgressItemStats {
            passed: 141,
            failed: 1,
            needs_manual: 2,
        });
        let mut inner = sink.inner.lock().unwrap();
        inner.handle(&completed);
        let line = inner.module_line(0);
        assert!(line.contains("141通过 1未通过 2需人工确认"));
        assert!(!line.contains("待确认"));
    }
}
