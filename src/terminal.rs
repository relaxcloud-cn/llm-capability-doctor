use crate::{
    config::Target,
    model::{CheckId, EventEnvelope, RunEvent, RunResult, Status, Verdict},
};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    time::Instant,
};
use tokio::sync::mpsc::UnboundedReceiver;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SPIN: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveState {
    Waiting,
    Running,
    WaitingNext,
    Final,
}
#[derive(Clone, Debug)]
struct TaskView {
    state: LiveState,
    step: String,
    status: Status,
    ended: Option<u64>,
    finished: bool,
}
impl Default for TaskView {
    fn default() -> Self {
        Self {
            state: LiveState::Waiting,
            step: "等待执行".into(),
            status: Status::NotExecuted,
            ended: None,
            finished: false,
        }
    }
}
#[derive(Clone, Debug)]
struct CheckView {
    state: LiveState,
    title: String,
    step: String,
    status: Status,
    started: Option<u64>,
    ended: Option<u64>,
    finished: bool,
    tasks: BTreeMap<u8, TaskView>,
}
impl Default for CheckView {
    fn default() -> Self {
        Self {
            state: LiveState::Waiting,
            title: "等待中".into(),
            step: "准备中".into(),
            status: Status::NotExecuted,
            started: None,
            ended: None,
            finished: false,
            tasks: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct TerminalState {
    target: Option<Target>,
    checks: BTreeMap<CheckId, CheckView>,
    plan: Vec<crate::judgment::CheckSpec>,
    order: Vec<CheckId>,
    reporting: bool,
    done: Option<RunResult>,
}
impl TerminalState {
    pub fn apply(&mut self, envelope: &EventEnvelope) {
        let time = envelope.elapsed_ms;
        match &envelope.event {
            RunEvent::Started { run } => {
                *self = Self::default();
                self.target = Some(run.target.clone());
                self.plan = run.plan.clone();
                self.order = run.checks.iter().map(|c| c.id).collect();
                self.checks = self
                    .order
                    .iter()
                    .map(|id| (*id, CheckView::default()))
                    .collect();
            }
            RunEvent::CheckStarted { id } => {
                let view = self.checks.get_mut(id).unwrap();
                view.state = LiveState::Running;
                view.started.get_or_insert(time);
            }
            RunEvent::Step { id, message } => {
                self.checks.get_mut(id).unwrap().step = message.clone();
            }
            RunEvent::TaskStarted {
                kind, repetition, ..
            } => {
                let view = self.checks.get_mut(kind).unwrap();
                view.state = LiveState::Running;
                view.started.get_or_insert(time);
                view.tasks.insert(
                    *repetition,
                    TaskView {
                        state: LiveState::Running,
                        step: "启动运行环境".into(),
                        ..TaskView::default()
                    },
                );
            }
            RunEvent::TaskStep {
                kind,
                repetition,
                message,
                ..
            } => {
                if let Some(task) = self.checks.get_mut(kind).unwrap().tasks.get_mut(repetition) {
                    task.step = message.clone();
                }
            }
            RunEvent::TaskFinished { task } => {
                let view = self.checks.get_mut(&task.kind).unwrap();
                let task_view = view.tasks.entry(task.repetition).or_default();
                task_view.state = LiveState::Final;
                task_view.step = if task.status == Status::Passed {
                    "结果正确".into()
                } else {
                    task.title.clone()
                };
                task_view.status = task.status;
                task_view.ended = Some(time);
                task_view.finished = task.finished;
                if !view.tasks.values().any(|t| t.state == LiveState::Running)
                    && view.tasks.len()
                        < self
                            .plan
                            .iter()
                            .find(|s| s.id == task.kind)
                            .map(|s| s.repetitions as usize)
                            .unwrap_or(2)
                {
                    view.state = LiveState::WaitingNext;
                }
            }
            RunEvent::CheckProgress { check } => {
                let view = self.checks.get_mut(&check.id).unwrap();
                view.title = check.title.clone();
                view.status = check.status;
                if !view.tasks.values().any(|t| t.state == LiveState::Running) {
                    view.state = LiveState::WaitingNext;
                }
            }
            RunEvent::CheckFinished { check } => {
                let view = self.checks.get_mut(&check.id).unwrap();
                view.state = LiveState::Final;
                view.title = check.title.clone();
                view.status = check.status;
                view.finished = check.finished;
                view.ended = Some(time);
            }
            RunEvent::ReportStarted => self.reporting = true,
            RunEvent::Finished { run } => {
                self.reporting = false;
                for check in &run.checks {
                    let view = self.checks.get_mut(&check.id).unwrap();
                    view.state = LiveState::Final;
                    view.title = check.title.clone();
                    view.status = check.status;
                    view.finished = check.finished;
                    view.ended.get_or_insert(time);
                }
                self.done = Some(run.clone());
            }
            RunEvent::Error { .. } => {}
        }
    }
    pub fn completed(&self) -> usize {
        self.checks.values().filter(|c| c.finished).count()
    }
    pub fn active_tasks(&self) -> usize {
        self.checks
            .values()
            .flat_map(|c| c.tasks.values())
            .filter(|t| t.state == LiveState::Running)
            .count()
    }
    pub fn render(&self, time: u64, width: usize, color: bool) -> String {
        let mut lines = vec![];
        let Some(target) = &self.target else {
            return String::new();
        };
        lines.push(format!(
            "{} {}",
            paint("AgentCheck", "white", color),
            paint("模型服务能力检测", "dim", color)
        ));
        lines.push(paint(
            &format!(
                "模型 {}　地址 {}",
                safe_text(&target.model),
                safe_text(&target.endpoint)
            ),
            "dim",
            color,
        ));
        lines.push(String::new());
        for &id in &self.order {
            let check = &self.checks[&id];
            let spec = self.plan.iter().find(|s| s.id == id);
            let repetitions = spec.map(|s| s.repetitions).unwrap_or(2);
            let agent = spec
                .map(|s| s.group == crate::judgment::CheckGroup::Agent)
                .unwrap_or_else(|| id.is_agent());
            let done = check.tasks.values().filter(|t| t.finished).count();
            let (title, tone) = match check.state {
                LiveState::Waiting => ("等待中".into(), "dim"),
                LiveState::WaitingNext => (
                    format!("等待下次执行 · 已完成 {done} / {repetitions} 次"),
                    "dim",
                ),
                LiveState::Running => (
                    if agent {
                        format!("进行中 · 已完成 {done} / {repetitions} 次")
                    } else {
                        "进行中".into()
                    },
                    "yellow",
                ),
                LiveState::Final => (check.title.clone(), status_tone(check.status)),
            };
            let mark = marker(check.state, check.status, time);
            let name = pad(
                spec.map(|s| s.name.as_str()).unwrap_or_else(|| id.name()),
                20,
            );
            let duration = check
                .started
                .map(|start| duration(check.ended.unwrap_or(time).saturating_sub(start)))
                .unwrap_or_default();
            let content = format!(
                "{}  {} {}",
                paint(&mark, tone, color),
                name,
                paint(&safe_text(&title), tone, color)
            );
            let content = right_duration(content, &duration, width, color);
            lines.push(content);
            if check.state == LiveState::Running {
                if agent {
                    for repetition in 1..=repetitions {
                        let task = check.tasks.get(&repetition).cloned().unwrap_or_default();
                        let mark = marker(task.state, task.status, time);
                        let tone = if task.state == LiveState::Running {
                            "cyan"
                        } else {
                            status_tone(task.status)
                        };
                        lines.push(format!(
                            "   {}  第 {} 次   {}",
                            paint(&mark, tone, color),
                            repetition,
                            paint(&safe_text(&task.step), tone, color)
                        ));
                    }
                } else {
                    lines.push(paint(
                        &format!("   └ {}", safe_text(&check.step)),
                        "dim",
                        color,
                    ));
                }
            }
        }
        lines.push(String::new());
        let count = self.completed();
        let total = self.checks.len();
        let fill = (count * 24 / total.max(1)).min(24);
        lines.push(format!(
            "{}{}  {}  {}",
            paint(&"█".repeat(fill), "cyan", color),
            paint(&"░".repeat(24 - fill), "dim", color),
            paint(
                &format!(
                    "{}%",
                    (count as f64 / total.max(1) as f64 * 100.0).round() as u64
                ),
                "cyan",
                color
            ),
            paint(
                &format!("已完成 {count} / {total} 项 · 已用 {}", duration(time)),
                "dim",
                color
            )
        ));
        if self.reporting {
            lines.push(format!(
                "{} 正在生成报告和整理检测记录…",
                paint(
                    &SPIN[(time / 330 % 10) as usize].to_string(),
                    "yellow",
                    color
                )
            ));
        }
        if let Some(run) = &self.done {
            let passed = run
                .checks
                .iter()
                .filter(|c| c.status == Status::Passed)
                .count();
            let failed = run
                .checks
                .iter()
                .filter(|c| c.status == Status::Failed)
                .count();
            let pending = run.checks.len().saturating_sub(passed + failed);
            lines.push(String::new());
            lines.push(paint(
                &format!(
                    "检测结果　{passed} 项通过 · {failed} 项未通过{}",
                    if pending > 0 {
                        format!(" · {pending} 项未确认或未执行")
                    } else {
                        String::new()
                    }
                ),
                "white",
                color,
            ));
            let tone = match run.verdict {
                Verdict::Passed => "green",
                Verdict::Failed => "red",
                Verdict::Incomplete => "dim",
            };
            lines.push(paint(
                &format!("结论　{}", run.verdict.title()),
                tone,
                color,
            ));
            if let Some(check) = run
                .checks
                .iter()
                .find(|c| c.status == Status::Failed)
                .or_else(|| run.checks.iter().find(|c| c.status != Status::Passed))
            {
                lines.push(paint(
                    &format!("问题　{}：{}", check.id.name(), safe_text(&check.title)),
                    "dim",
                    color,
                ));
            }
            if let Some(path) = &run.report_path {
                lines.push(paint(
                    &format!("[报告] HTML：{}", safe_text(path)),
                    "cyan",
                    color,
                ));
                let zip = std::path::Path::new(path).with_file_name("records.zip");
                lines.push(paint(
                    &format!("[记录] ZIP ：{}", safe_text(&zip.to_string_lossy())),
                    "cyan",
                    color,
                ));
            }
        }
        // Wrap by terminal cell width, preserving ANSI sequences and Chinese characters.
        lines
            .into_iter()
            .flat_map(|line| wrap_ansi(&line, width.max(36)))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}
fn marker(state: LiveState, status: Status, time: u64) -> String {
    match state {
        LiveState::Running => SPIN[(time / 330 % 10) as usize].to_string(),
        LiveState::Final if status == Status::Passed => "✓".into(),
        LiveState::Final if status == Status::Failed => "✗".into(),
        _ => "○".into(),
    }
}
fn status_tone(status: Status) -> &'static str {
    match status {
        Status::Passed => "green",
        Status::Failed => "red",
        _ => "dim",
    }
}
fn paint(text: &str, tone: &str, color: bool) -> String {
    if !color {
        return text.into();
    }
    let rgb = match tone {
        "green" => (111, 225, 172),
        "red" => (255, 131, 143),
        "yellow" => (244, 205, 126),
        "cyan" => (132, 203, 244),
        "white" => (222, 231, 242),
        _ => (131, 147, 170),
    };
    format!("\x1b[38;2;{};{};{}m{text}\x1b[0m", rgb.0, rgb.1, rgb.2)
}
pub fn safe_text(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn pad(text: &str, width: usize) -> String {
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(UnicodeWidthStr::width(text)))
    )
}
fn duration(ms: u64) -> String {
    let sec = ms as f64 / 1000.0;
    if sec < 60.0 {
        format!("{:.1}秒", sec).replace(".0秒", "秒")
    } else {
        format!("{}分{}秒", ms / 60000, ms / 1000 % 60)
    }
}
fn visible_width(text: &str) -> usize {
    strip_ansi(text).width()
}
fn strip_ansi(text: &str) -> String {
    let mut output = String::new();
    let mut escape = false;
    for c in text.chars() {
        if c == '\x1b' {
            escape = true;
        } else if escape {
            if c == 'm' {
                escape = false;
            }
        } else {
            output.push(c);
        }
    }
    output
}
fn right_duration(content: String, time: &str, width: usize, color: bool) -> String {
    let occupied = visible_width(&content) + time.width() + 2;
    if time.is_empty() {
        content
    } else if occupied <= width {
        format!(
            "{content}{}{}",
            " ".repeat(width - visible_width(&content) - time.width()),
            paint(time, "dim", color)
        )
    } else {
        format!("{content}  {}", paint(time, "dim", color))
    }
}
fn wrap_ansi(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    let mut line = String::new();
    let mut cells = 0;
    let mut escape = false;
    for c in text.chars() {
        if c == '\x1b' {
            escape = true;
            line.push(c);
            continue;
        }
        if escape {
            line.push(c);
            if c == 'm' {
                escape = false;
            }
            continue;
        }
        let w = c.width().unwrap_or(0);
        if cells + w > width && !line.is_empty() {
            lines.push(line);
            line = "   ".into();
            cells = 3;
        }
        line.push(c);
        cells += w;
    }
    lines.push(line);
    lines
}
pub fn log_line(event: &RunEvent) -> Option<String> {
    Some(match event {
        RunEvent::Started { run } => format!(
            "[开始] 模型 {} · 共 {} 个检测项目",
            safe_text(&run.target.model),
            run.checks.len()
        ),
        RunEvent::CheckStarted { id } => format!("[开始] {}", id.name()),
        RunEvent::Step { id, message } => format!("[{}] {}", id.name(), safe_text(message)),
        RunEvent::TaskStarted {
            kind, repetition, ..
        } => format!("[开始] {} · 第{}次", kind.name(), repetition),
        RunEvent::TaskStep {
            kind,
            repetition,
            message,
            ..
        } => format!(
            "[{} · 第{}次] {}",
            kind.name(),
            repetition,
            safe_text(message)
        ),
        RunEvent::TaskFinished { task } => format!(
            "[{} · 第{}次] {}",
            task.kind.name(),
            task.repetition,
            safe_text(&task.title)
        ),
        RunEvent::CheckFinished { check } => format!(
            "[{}] {}：{}",
            check.status.label(),
            check.id.name(),
            safe_text(&check.title)
        ),
        RunEvent::CheckProgress { .. } => return None,
        RunEvent::ReportStarted => "[报告] 正在生成报告和整理检测记录…".into(),
        RunEvent::Finished { run } => {
            let mut text = format!("[结论] {}", run.verdict.title());
            if let Some(decision) = &run.decision {
                text.push_str(&format!("\n[依据] {}", safe_text(&decision.reason)));
                for condition in &decision.conditions {
                    text.push_str(&format!("\n[条件] {}", safe_text(condition)));
                }
                if let Some(report) = &run.report_path {
                    text.push_str(&format!(
                        "\n[证据] {}",
                        safe_text(
                            &std::path::Path::new(report)
                                .with_file_name("evidence-index.json")
                                .to_string_lossy()
                        )
                    ));
                }
            }
            text
        }
        RunEvent::Error { message } => format!("[错误] {}", safe_text(message)),
    })
}
pub fn startup(text: &str, mark: &str, tone: &str, no_color: bool) {
    let color = io::stderr().is_terminal() && !no_color && std::env::var_os("NO_COLOR").is_none();
    eprintln!("{} {}", paint(mark, tone, color), safe_text(text));
}
fn vt_available() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::*;
        let handle = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode = 0;
        return GetConsoleMode(handle, &mut mode) != 0
            && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0;
    }
    #[cfg(not(windows))]
    {
        std::env::var("TERM").is_ok_and(|t| t != "dumb")
    }
}
pub async fn display(
    mut events: UnboundedReceiver<EventEnvelope>,
    no_color: bool,
) -> io::Result<()> {
    let interactive = io::stderr().is_terminal() && vt_available();
    let color = interactive && !no_color && std::env::var_os("NO_COLOR").is_none();
    let started = Instant::now();
    let mut state = TerminalState::default();
    let mut previous_lines = 0;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(120));
    if interactive {
        eprint!("\x1b[?25l");
    }
    loop {
        tokio::select! {
            event=events.recv()=>match event {
                Some(event)=>{
                    state.apply(&event);
                    if !interactive{if let Some(line)=log_line(&event.event){eprintln!("{line}");}}
                    if interactive{draw(&state,event.elapsed_ms,color,&mut previous_lines)?;}
                },
                None=>break,
            },
            _=tick.tick(),if interactive=>draw(&state,started.elapsed().as_millis() as u64,color,&mut previous_lines)?,
        }
    }
    if interactive {
        eprint!("\x1b[?25h");
        io::stderr().flush()?;
    }
    Ok(())
}
fn draw(state: &TerminalState, time: u64, color: bool, previous: &mut usize) -> io::Result<()> {
    let width = terminal_size::terminal_size_of(io::stderr())
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100);
    let text = state.render(time, width, color);
    if text.is_empty() {
        return Ok(());
    }
    let mut out = io::stderr().lock();
    if *previous > 0 {
        write!(out, "\x1b[{}A\r\x1b[J", *previous)?;
    }
    write!(out, "{text}")?;
    out.flush()?;
    *previous = text.lines().count();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Protocol;
    use crate::model::TaskResult;
    fn envelope(event: RunEvent, time: u64) -> EventEnvelope {
        EventEnvelope {
            event,
            elapsed_ms: time,
        }
    }
    fn initial() -> TerminalState {
        let mut state = TerminalState::default();
        let run = RunResult::new(Target {
            model: "m".into(),
            endpoint: "https://example.org/v1/chat/completions".into(),
            protocol: Protocol::ChatCompletions,
        });
        state.apply(&envelope(RunEvent::Started { run }, 0));
        state
    }
    #[test]
    fn two_parallel_tasks_and_waiting_second_repetition_are_truthful() {
        let mut state = initial();
        state.apply(&envelope(
            RunEvent::TaskStarted {
                id: "a".into(),
                kind: CheckId::Continuous,
                repetition: 1,
            },
            0,
        ));
        state.apply(&envelope(
            RunEvent::TaskStarted {
                id: "b".into(),
                kind: CheckId::Recovery,
                repetition: 1,
            },
            0,
        ));
        assert_eq!(state.active_tasks(), 2);
        assert_eq!(state.completed(), 0);
        let task = TaskResult {
            id: "a".into(),
            finished: true,
            kind: CheckId::Continuous,
            repetition: 1,
            status: Status::Passed,
            title: "结果正确".into(),
            elapsed_ms: 1000,
            normal_end: true,
            fault_delivered: false,
            recovery_observed: false,
            tool_events: vec![],
            evidence_ids: vec![],
            findings: vec![],
        };
        state.apply(&envelope(RunEvent::TaskFinished { task }, 1000));
        assert_eq!(state.active_tasks(), 1);
        assert_eq!(state.completed(), 0);
        assert!(
            state
                .render(1000, 100, false)
                .contains("等待下次执行 · 已完成 1 / 2 次")
        );
    }
    #[test]
    fn plain_log_has_no_control_sequences_and_narrow_render_stays_bounded() {
        let state = initial();
        assert!(!state.render(0, 48, false).contains('\x1b'));
        for line in state.render(0, 48, false).lines() {
            assert!(line.width() <= 48);
        }
        assert_eq!(safe_text("model\x1b[2J\nname"), "model [2J name");
    }
}
