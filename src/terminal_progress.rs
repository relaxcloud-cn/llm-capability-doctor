//! 终端进度渲染：把 ProgressEvent 流渲染成与 GUI 一致的进度界面。
//!
//! 交互终端（TTY）下维护一个原地重绘的区块：每个检测模块一行，
//! 正在执行的模块下方显示当前小项，底部是总进度条。
//! 非 TTY（管道、CI、重定向）退化为每个事件一行纯文本。
//! 全部输出走调用方给的 writer（main 里接 stderr），不污染 stdout。

use crate::cli::{
    ProgressEvent, ProgressPhase, ProgressPlanItem, ProgressSink, module_display_name,
    module_state_label,
};
use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

const BAR_WIDTH: usize = 24;
const MAX_LINE_WIDTH: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModuleStatus {
    Waiting,
    Running,
    Done { state: String, reason: String },
}

#[derive(Debug, Clone)]
struct ModuleView {
    id: String,
    status: ModuleStatus,
    items: Vec<ProgressPlanItem>,
    started: Option<Instant>,
    elapsed: Option<Duration>,
}

impl ModuleView {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            status: ModuleStatus::Waiting,
            items: Vec::new(),
            started: None,
            elapsed: None,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveDetail {
    module_pos: usize,
    label: String,
    /// 模块内已推进的样本位置与样本总量（来自 ProgressEvent.detail_*）。
    index: usize,
    total: usize,
}

/// 把 ProgressEvent 渲染到终端的进度显示端。
pub struct TerminalProgressSink<W: Write> {
    writer: W,
    interactive: bool,
    color: bool,
    model: String,
    endpoint: String,
    run_started: Option<Instant>,
    modules: Vec<ModuleView>,
    positions: BTreeMap<String, usize>,
    total: usize,
    completed: usize,
    active: Option<ActiveDetail>,
    drawn: usize,
    last_plain_detail: Option<String>,
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
            writer,
            interactive,
            color,
            model: model.into(),
            endpoint: endpoint.into(),
            run_started: None,
            modules: Vec::new(),
            positions: BTreeMap::new(),
            total: 0,
            completed: 0,
            active: None,
            drawn: 0,
            last_plain_detail: None,
        }
    }

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
                let pos = self.module_mut(id);
                let module = &mut self.modules[pos];
                module.status = ModuleStatus::Running;
                module.started = Some(Instant::now());
                if let Some(items) = &event.items {
                    module.items = items.clone();
                }
                self.active = Some(ActiveDetail {
                    module_pos: pos,
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
                self.active = Some(ActiveDetail {
                    module_pos: pos,
                    label,
                    index: event.detail_index.unwrap_or(0),
                    total: event.detail_total.unwrap_or(0),
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
                };
                self.modules[pos].elapsed = elapsed;
                self.completed = event.index;
                self.active = None;
            }
            ProgressPhase::RunCompleted | ProgressPhase::RunStopped => {
                self.active = None;
            }
        }
    }

    fn module_line(&self, pos: usize) -> String {
        let module = &self.modules[pos];
        let name = module_display_name(&module.id);
        let line = match &module.status {
            ModuleStatus::Waiting => {
                let marker = self.paint("2", "○");
                let state = self.paint("2", "等待中");
                format!("{marker} {name} {}  {state}", module.id)
            }
            ModuleStatus::Running => {
                let marker = self.paint("33", "●");
                let state = self.paint("33", "进行中");
                format!("{marker} {name} {}  {state}", module.id)
            }
            ModuleStatus::Done { state, reason } => {
                let (marker, styled) = match state.as_str() {
                    "pass" => (self.paint("32", "✓"), self.paint("32", "通过")),
                    "fail" => (self.paint("31", "✗"), self.paint("31", "失败")),
                    _ => (
                        self.paint("33", "●"),
                        self.paint("33", module_state_label(state)),
                    ),
                };
                let mut suffix = String::new();
                if let Some(elapsed) = module.elapsed {
                    suffix = format!(" {}", format_duration(elapsed));
                }
                let note = if reason.is_empty() {
                    String::new()
                } else {
                    format!(" {}", self.paint("2", &truncate(reason, 40)))
                };
                format!("{marker} {name} {}  {styled}{note}{suffix}", module.id)
            }
        };
        line
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
                let label = self
                    .active
                    .as_ref()
                    .map(|active| self.paint("36", &active.label))
                    .unwrap_or_default();
                lines.push(format!("   └ {label}"));
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
            .map(|active| self.detail_fraction(active))
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

    fn detail_fraction(&self, active: &ActiveDetail) -> f64 {
        if active.total == 0 {
            return 0.0;
        }
        (active.index as f64 / active.total as f64).min(1.0)
    }

    fn redraw(&mut self) {
        let lines = self.block();
        let mut output = String::new();
        if self.drawn > 0 {
            output.push_str(&format!("\x1b[{}A", self.drawn));
        }
        for line in &lines {
            output.push_str("\r\x1b[2K");
            output.push_str(&truncate(line, MAX_LINE_WIDTH));
            output.push('\n');
        }
        if lines.len() < self.drawn {
            for _ in lines.len()..self.drawn {
                output.push_str("\r\x1b[2K\n");
            }
            output.push_str(&format!("\x1b[{}A", self.drawn - lines.len()));
        }
        self.drawn = lines.len();
        let _ = self.writer.write_all(output.as_bytes());
        let _ = self.writer.flush();
    }

    fn print_line(&mut self, line: &str) {
        let _ = writeln!(self.writer, "{line}");
        let _ = self.writer.flush();
    }

    fn emit_plain(&mut self, event: &ProgressEvent) {
        match event.phase {
            ProgressPhase::RunStarted => {
                self.print_line(&format!(
                    "[开始] 模型 {} · 共 {} 个检测项目",
                    self.model, event.total
                ));
            }
            ProgressPhase::ModuleStarted => {
                let name = event
                    .module_id
                    .as_deref()
                    .map(module_display_name)
                    .unwrap_or("未知项目");
                self.print_line(&format!(
                    "[{}/{}] {name} 开始检测",
                    event.index + 1,
                    event.total
                ));
            }
            ProgressPhase::ModuleProgress => {
                if let Some(active) = &self.active {
                    let label = active.label.clone();
                    // 同一样本前后各发一次事件，跳过与上一条相同的行。
                    if self.last_plain_detail.as_deref() == Some(label.as_str()) {
                        return;
                    }
                    self.last_plain_detail = Some(label.clone());
                    self.print_line(&format!("[{}/{}]   {label}", event.index + 1, event.total));
                }
            }
            ProgressPhase::ModuleCompleted => {
                let name = event
                    .module_id
                    .as_deref()
                    .map(module_display_name)
                    .unwrap_or("未知项目");
                let state = event
                    .state
                    .as_deref()
                    .map(module_state_label)
                    .unwrap_or("未知");
                let reason = if event.message.is_empty() {
                    String::new()
                } else {
                    format!("（{}）", event.message)
                };
                self.print_line(&format!(
                    "[{}/{}] {name} {state}{reason}",
                    event.index, event.total
                ));
            }
            ProgressPhase::RunCompleted => {
                self.print_line(&format!(
                    "[完成] 全部检测结束 · 用时 {}",
                    self.elapsed_text()
                ));
            }
            ProgressPhase::RunStopped => {
                self.print_line(&format!(
                    "[停止] {} · 用时 {}",
                    event.message,
                    self.elapsed_text()
                ));
            }
        }
    }
}

impl<W: Write> ProgressSink for TerminalProgressSink<W> {
    fn emit(&mut self, event: ProgressEvent) {
        let first = self.run_started.is_none();
        self.handle(&event);
        if !self.interactive {
            self.emit_plain(&event);
            return;
        }
        if first && matches!(event.phase, ProgressPhase::RunStarted) {
            self.print_line(&format!(
                "{}  {}",
                self.paint("34;1", "AgentCheck"),
                self.paint("2", "模型服务能力检测")
            ));
            self.print_line(&format!(
                "{}  {}   {}  {}",
                self.paint("2", "模型"),
                self.model,
                self.paint("2", "地址"),
                self.endpoint
            ));
            self.print_line("");
        }
        self.redraw();
    }
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
    use crate::cli::ProgressPhase;

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
            progress.detail_index = Some(52);
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
        assert!(output.contains("[1/2] 模型能力跑分 失败（144 个样本未通过）"));
        assert!(output.contains("[完成] 全部检测结束"));
    }

    #[test]
    fn detail_label_uses_item_prefix_for_sub_turns() {
        let mut sink = TerminalProgressSink::new(Vec::new(), false, false, "m", "e");
        let mut started = event(ProgressPhase::ModuleStarted, Some("agent"));
        started.items = crate::cli::module_plan_items("agent");
        sink.handle(&started);
        let mut progress = event(ProgressPhase::ModuleProgress, Some("agent"));
        progress.detail_id = Some("T3-A-call-2".into());
        progress.detail_index = Some(5);
        progress.detail_total = Some(10);
        let label = sink.detail_label(&progress, 0);
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
}
