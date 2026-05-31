use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::{Receiver, Sender, unbounded};
use crossterm::{
    cursor::{Hide, SetCursorStyle, Show},
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    },
    execute,
    terminal::{
        self as crossterm_terminal, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SCROLLBACK: usize = 2_000;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 100;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if handle_cli_flags(&args)? {
        return Ok(());
    }

    let shell = ShellSpec::from_args_or_env(&args).unwrap_or_else(ShellSpec::default_for_platform);
    let mut terminal = TerminalGuard::enter()?;
    let (tx, rx) = unbounded();
    let mut app = App::new(shell, tx)?;

    if let Err(err) = run_app(terminal.terminal_mut(), &mut app, rx) {
        terminal.restore()?;
        eprintln!("{err:?}");
        std::process::exit(1);
    }

    terminal.restore()?;
    Ok(())
}

fn handle_cli_flags(args: &[String]) -> Result<bool> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{HELP_TEXT}");
        return Ok(true);
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("wimx {}", env!("CARGO_PKG_VERSION"));
        return Ok(true);
    }
    if args.iter().any(|arg| arg == "--smoke-test") {
        pty_smoke_test()?;
        println!("smoke test passed");
        return Ok(true);
    }
    Ok(false)
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
    rx: Receiver<AppEvent>,
) -> Result<()> {
    let mut last_tick = Instant::now();

    loop {
        app.drain_events(&rx);
        terminal.draw(|frame| draw(frame, app))?;
        apply_terminal_cursor(terminal, app)?;

        let timeout = Duration::from_millis(32)
            .saturating_sub(last_tick.elapsed().min(Duration::from_millis(32)));
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press && app.handle_key(key)? {
                        break;
                    }
                }
                Event::Mouse(mouse) => app.handle_mouse(mouse)?,
                Event::Paste(text) => app.send_text(&text)?,
                Event::Resize(_, _) => app.status = "resized".to_string(),
                _ => {}
            }
        }

        if last_tick.elapsed() >= Duration::from_millis(250) {
            app.spinner = (app.spinner + 1) % SPINNER.len();
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn apply_terminal_cursor(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &App,
) -> Result<()> {
    if app.path_prompt.is_some() {
        execute!(terminal.backend_mut(), SetCursorStyle::SteadyBar, Show)?;
        return Ok(());
    }

    let Some(pane) = app.panes.get(app.active) else {
        execute!(terminal.backend_mut(), Hide)?;
        return Ok(());
    };

    execute!(
        terminal.backend_mut(),
        pane.parser.callbacks().cursor_style()
    )?;
    if pane.exited || pane.parser.screen().hide_cursor() {
        execute!(terminal.backend_mut(), Hide)?;
    } else {
        execute!(terminal.backend_mut(), Show)?;
    }
    Ok(())
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<std::io::Stdout>>,
    restored: bool,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<std::io::Stdout>> {
        &mut self.terminal
    }

    fn restore(&mut self) -> Result<()> {
        if !self.restored {
            disable_raw_mode()?;
            execute!(
                self.terminal.backend_mut(),
                SetCursorStyle::DefaultUserShape,
                Show,
                DisableBracketedPaste,
                DisableMouseCapture,
                LeaveAlternateScreen
            )?;
            self.terminal.show_cursor()?;
            self.restored = true;
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[derive(Clone)]
struct ShellSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl ShellSpec {
    fn from_args_or_env(args: &[String]) -> Option<Self> {
        if let Some(index) = args.iter().position(|arg| arg == "--shell") {
            if let Some(program) = args.get(index + 1) {
                return Some(Self {
                    program: normalize_shell_program(program),
                    args: Vec::new(),
                    cwd: None,
                });
            }
        }

        std::env::var("WIMX_SHELL").ok().map(|program| Self {
            program: normalize_shell_program(&program),
            args: std::env::var("WIMX_SHELL_ARGS")
                .ok()
                .map(|args| args.split_whitespace().map(ToOwned::to_owned).collect())
                .unwrap_or_default(),
            cwd: None,
        })
    }

    fn default_for_platform() -> Self {
        if cfg!(windows) {
            if let Some(program) = find_command_path("pwsh.exe") {
                Self {
                    program,
                    args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
                    cwd: None,
                }
            } else {
                Self {
                    program: windows_powershell_path()
                        .unwrap_or_else(|| "powershell.exe".to_string()),
                    args: vec![
                        "-NoLogo".to_string(),
                        "-NoProfile".to_string(),
                        "-NoExit".to_string(),
                    ],
                    cwd: None,
                }
            }
        } else {
            Self {
                program: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
                args: Vec::new(),
                cwd: None,
            }
        }
    }

    fn label(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }

    fn command_builder(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.program);
        for arg in &self.args {
            command.arg(arg);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("FORCE_COLOR", "1");
        command.env("CLICOLOR_FORCE", "1");
        if let Some(cwd) = &self.cwd {
            command.cwd(normalize_command_cwd(cwd.clone()));
        } else if let Ok(cwd) = std::env::current_dir() {
            command.cwd(normalize_command_cwd(cwd));
        }
        command
    }
}

fn find_command_path(command: &str) -> Option<String> {
    if cfg!(windows) {
        return Command::new("where.exe")
            .arg(command)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                let candidates: Vec<String> = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                select_windows_command_path(candidates)
            });
    }

    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {}", shell_quote(command)))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn select_windows_command_path(candidates: Vec<String>) -> Option<String> {
    let mut expanded = Vec::new();
    for candidate in candidates {
        expanded.extend(windows_launchable_variants(&candidate));
        expanded.push(candidate);
    }

    expanded
        .iter()
        .filter_map(|path| windows_command_priority(path).map(|priority| (priority, path)))
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, path)| path.clone())
        .or_else(|| expanded.into_iter().next())
}

fn windows_launchable_path(path: &str) -> bool {
    windows_command_priority(path).is_some()
}

fn windows_command_priority(path: &str) -> Option<u8> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        Some(0)
    } else if lower.ends_with(".com") {
        Some(1)
    } else if lower.ends_with(".cmd") {
        Some(2)
    } else if lower.ends_with(".bat") {
        Some(3)
    } else if lower.ends_with(".ps1") {
        Some(4)
    } else {
        None
    }
}

fn windows_launchable_variants(path: &str) -> Vec<String> {
    if windows_launchable_path(path) {
        return Vec::new();
    }

    [".exe", ".com", ".cmd", ".bat", ".ps1"]
        .into_iter()
        .map(|extension| format!("{path}{extension}"))
        .filter(|candidate| Path::new(candidate).exists())
        .collect()
}

fn normalize_command_cwd(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn windows_powershell_path() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| "C:\\Windows".to_string());
    let path = format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    std::path::Path::new(&path).exists().then_some(path)
}

fn windows_cmd_path() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| "C:\\Windows".to_string());
    let path = format!("{root}\\System32\\cmd.exe");
    std::path::Path::new(&path).exists().then_some(path)
}

fn normalize_shell_program(program: &str) -> String {
    if !cfg!(windows) {
        return program.to_string();
    }
    let has_path = program.contains('\\') || program.contains('/');
    if has_path || std::path::Path::new(program).exists() {
        return select_windows_command_path(vec![program.to_string()])
            .unwrap_or_else(|| program.to_string());
    }
    find_command_path(program)
        .or_else(|| {
            program
                .eq_ignore_ascii_case("powershell.exe")
                .then(windows_powershell_path)
                .flatten()
        })
        .or_else(|| {
            program
                .eq_ignore_ascii_case("cmd.exe")
                .then(windows_cmd_path)
                .flatten()
        })
        .unwrap_or_else(|| program.to_string())
}

#[derive(Clone, Copy)]
struct AgentDefinition {
    label: &'static str,
    commands: &'static [&'static str],
    args: &'static [&'static str],
    install: Option<&'static [&'static str]>,
}

const AGENT_DEFINITIONS: &[AgentDefinition] = &[
    AgentDefinition {
        label: "OpenCode",
        commands: &["opencode", "opencode.cmd", "opencode.exe"],
        args: &[],
        install: Some(&["npm", "install", "-g", "opencode-ai"]),
    },
    AgentDefinition {
        label: "Codex",
        commands: &["codex", "codex.cmd", "codex.exe"],
        args: &[],
        install: Some(&["npm", "install", "-g", "@openai/codex"]),
    },
    AgentDefinition {
        label: "Claude Code",
        commands: &["claude", "claude.cmd", "claude.exe", "claude-code"],
        args: &[],
        install: Some(&["npm", "install", "-g", "@anthropic-ai/claude-code"]),
    },
    AgentDefinition {
        label: "Hermes",
        commands: &["hermes", "hermes.cmd", "hermes.exe", "hermes-agent"],
        args: &[],
        install: None,
    },
    AgentDefinition {
        label: "OpenClaw",
        commands: &["openclaw", "openclaw.cmd", "openclaw.exe", "open-claw"],
        args: &[],
        install: None,
    },
];

fn detect_agents() -> Vec<AgentTool> {
    AGENT_DEFINITIONS
        .iter()
        .copied()
        .map(AgentTool::detected)
        .collect()
}

fn agent_shell_spec(commands: &[&str], extra_args: &[&str]) -> Option<(String, ShellSpec)> {
    for command in commands {
        let Some(program) = find_command_path(command) else {
            continue;
        };
        let args: Vec<String> = extra_args.iter().map(|arg| (*arg).to_string()).collect();
        let spec = command_spec_for_program(&program, args);
        return Some(((*command).to_string(), spec));
    }
    None
}

fn shell_spec_from_parts(parts: &[&str]) -> ShellSpec {
    let program = parts.first().copied().unwrap_or_default();
    let args = parts.iter().skip(1).map(|arg| (*arg).to_string()).collect();
    command_spec_for_program(program, args)
}

fn command_spec_for_program(program: &str, args: Vec<String>) -> ShellSpec {
    if cfg!(windows) && is_windows_batch_shim(program) {
        ShellSpec {
            program: windows_cmd_path().unwrap_or_else(|| "cmd.exe".to_string()),
            args: windows_cmd_script_args(program, &args),
            cwd: None,
        }
    } else if cfg!(windows) && is_windows_powershell_shim(program) {
        let mut shell_args = vec![
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            program.to_string(),
        ];
        shell_args.extend(args);
        ShellSpec {
            program: windows_powershell_path().unwrap_or_else(|| "powershell.exe".to_string()),
            args: shell_args,
            cwd: None,
        }
    } else {
        ShellSpec {
            program: program.to_string(),
            args,
            cwd: None,
        }
    }
}

fn windows_cmd_script_args(program: &str, args: &[String]) -> Vec<String> {
    let mut command = quote_for_cmd(program);
    for arg in args {
        command.push(' ');
        command.push_str(&quote_for_cmd(arg));
    }
    vec![
        "/D".to_string(),
        "/S".to_string(),
        "/C".to_string(),
        command,
    ]
}

fn quote_for_cmd(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '\\' | '/'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

fn is_windows_batch_shim(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

fn is_windows_powershell_shim(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".ps1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_windows_cmd_shim_over_extensionless_npm_shim() {
        let selected = select_windows_command_path(vec![
            r"C:\Users\h\AppData\Roaming\npm\codex".to_string(),
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd".to_string(),
        ]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Users\h\AppData\Roaming\npm\codex.cmd")
        );
    }

    #[test]
    fn selects_windows_exe_over_extensionless_windowsapps_alias() {
        let selected = select_windows_command_path(vec![
            r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex".to_string(),
            r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex.exe".to_string(),
        ]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex.exe")
        );
    }

    #[test]
    fn selects_windows_powershell_shim_when_it_is_the_only_launchable_path() {
        let selected =
            select_windows_command_path(vec![r"C:\Users\h\AppData\Roaming\npm\tool.ps1".into()]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Users\h\AppData\Roaming\npm\tool.ps1")
        );
    }

    #[test]
    fn builds_cmd_wrapper_for_windows_batch_shim_with_quoted_path() {
        let args = windows_cmd_script_args(
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd",
            &["--color".to_string(), "always".to_string()],
        );

        assert_eq!(args[0], "/D");
        assert_eq!(args[1], "/S");
        assert_eq!(args[2], "/C");
        assert_eq!(
            args[3],
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd --color always"
        );

        let spaced = windows_cmd_script_args(r"C:\Program Files\Agent\agent.cmd", &[]);
        assert_eq!(spaced[3], r#""C:\Program Files\Agent\agent.cmd""#);
    }

    #[test]
    fn tracks_cursor_style_sequences_used_by_node_tuis() {
        let mut parser = vt100::Parser::new_with_callbacks(
            DEFAULT_ROWS,
            DEFAULT_COLS,
            SCROLLBACK,
            TermCallbacks::default(),
        );

        parser.process(b"\x1b[5 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::BlinkingBar
        );

        parser.process(b"\x1b[2 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::SteadyBlock
        );

        parser.process(b"\x1b[0 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::DefaultUserShape
        );
    }

    #[test]
    fn tracks_cursor_visibility_sequences_used_by_fullscreen_tuis() {
        let mut parser = vt100::Parser::new_with_callbacks(
            DEFAULT_ROWS,
            DEFAULT_COLS,
            SCROLLBACK,
            TermCallbacks::default(),
        );

        parser.process(b"\x1b[?25l");
        assert!(parser.screen().hide_cursor());

        parser.process(b"\x1b[?25h");
        assert!(!parser.screen().hide_cursor());
    }

    #[test]
    fn strips_windows_verbatim_drive_prefix_from_cwd() {
        let path = normalize_command_cwd(PathBuf::from(r"\\?\D:\desktop\code\wimx"));

        assert_eq!(path.to_string_lossy(), r"D:\desktop\code\wimx");
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix_from_cwd() {
        let path = normalize_command_cwd(PathBuf::from(r"\\?\UNC\server\share\repo"));

        assert_eq!(path.to_string_lossy(), r"\\server\share\repo");
    }
}

enum AppEvent {
    PtyOutput { pane_id: usize, bytes: Vec<u8> },
    PaneExited { pane_id: usize },
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LayoutMode {
    Grid,
    Stack,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Language {
    En,
    Zh,
}

impl Language {
    fn from_env() -> Self {
        let value = std::env::var("WIMX_LANG")
            .or_else(|_| std::env::var("WIMX_LANGUAGE"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if value.starts_with("zh") || value.contains("chinese") {
            Self::Zh
        } else {
            Self::En
        }
    }

    fn toggle(self) -> Self {
        match self {
            Self::En => Self::Zh,
            Self::Zh => Self::En,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "中文",
        }
    }
}

fn ui_text(language: Language, en: &'static str, zh: &'static str) -> &'static str {
    match language {
        Language::En => en,
        Language::Zh => zh,
    }
}

struct Pane {
    id: usize,
    title: String,
    group: usize,
    command: ShellSpec,
    parser: vt100::Parser<TermCallbacks>,
    writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    size: (u16, u16),
    exited: bool,
    unread: usize,
}

impl Pane {
    fn new(id: usize, title: String, group: usize, command: ShellSpec) -> Self {
        Self {
            id,
            title,
            group,
            command,
            parser: vt100::Parser::new_with_callbacks(
                DEFAULT_ROWS,
                DEFAULT_COLS,
                SCROLLBACK,
                TermCallbacks::default(),
            ),
            writer: None,
            master: None,
            child: None,
            size: (DEFAULT_ROWS, DEFAULT_COLS),
            exited: true,
            unread: 0,
        }
    }

    fn write_status(&mut self, text: &str) {
        self.parser
            .process(format!("\r\nwimx: {text}\r\n").as_bytes());
    }

    fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if self.exited {
            self.write_status("pane exited; press Ctrl+R to respawn");
            return Ok(());
        }
        let Some(writer) = &self.writer else {
            self.write_status("no pty writer");
            return Ok(());
        };
        let mut writer = writer
            .lock()
            .map_err(|_| anyhow!("pty writer lock poisoned"))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        if self.size == (rows, cols) {
            return;
        }
        self.size = (rows, cols);
        self.parser.screen_mut().set_size(rows, cols);
        if let Some(master) = &self.master {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    fn scroll_scrollback(&mut self, delta: i32) {
        let current = self.scrollback() as i32;
        let next = (current + delta).max(0) as usize;
        self.parser.screen_mut().set_scrollback(next);
    }

    fn reset_scrollback(&mut self) {
        self.parser.screen_mut().set_scrollback(0);
    }
}

struct TermCallbacks {
    replies: Vec<Vec<u8>>,
    cursor_style: SetCursorStyle,
}

impl Default for TermCallbacks {
    fn default() -> Self {
        Self {
            replies: Vec::new(),
            cursor_style: SetCursorStyle::DefaultUserShape,
        }
    }
}

impl TermCallbacks {
    fn take_replies(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.replies)
    }

    fn cursor_style(&self) -> SetCursorStyle {
        self.cursor_style
    }
}

impl vt100::Callbacks for TermCallbacks {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (i1, i2, c, first_param(params)) {
            (None, None, 'n', 5) => self.replies.push(b"\x1b[0n".to_vec()),
            (None, None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.replies
                    .push(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
            }
            (None, None, 'c', _) => self.replies.push(b"\x1b[?1;2c".to_vec()),
            (Some(b'?'), None, 'c', _) => self.replies.push(b"\x1b[?1;2c".to_vec()),
            (Some(b' '), None, 'q', style) => {
                self.cursor_style = cursor_style_from_id(style);
            }
            _ => {}
        }
    }
}

fn first_param(params: &[&[u16]]) -> u16 {
    params
        .first()
        .and_then(|values| values.first())
        .copied()
        .unwrap_or(0)
}

fn cursor_style_from_id(style: u16) -> SetCursorStyle {
    match style {
        1 => SetCursorStyle::BlinkingBlock,
        2 => SetCursorStyle::SteadyBlock,
        3 => SetCursorStyle::BlinkingUnderScore,
        4 => SetCursorStyle::SteadyUnderScore,
        5 => SetCursorStyle::BlinkingBar,
        6 => SetCursorStyle::SteadyBar,
        _ => SetCursorStyle::DefaultUserShape,
    }
}

#[derive(Clone)]
struct AgentTool {
    label: &'static str,
    command: String,
    spec: Option<ShellSpec>,
    install_spec: Option<ShellSpec>,
}

impl AgentTool {
    fn detected(definition: AgentDefinition) -> Self {
        let detected = agent_shell_spec(definition.commands, definition.args);
        let install_spec = definition.install.map(shell_spec_from_parts);
        let command = detected
            .as_ref()
            .map(|(command, _)| command.clone())
            .unwrap_or_else(|| definition.commands[0].to_string());
        Self {
            label: definition.label,
            command,
            spec: detected.map(|(_, spec)| spec),
            install_spec,
        }
    }

    fn installed(&self) -> bool {
        self.spec.is_some()
    }
}

struct PathPrompt {
    agent_index: usize,
    input: String,
    browser: PathBrowser,
    browser_mode: bool,
}

struct PathBrowser {
    cwd: PathBuf,
    entries: Vec<PathBrowserEntry>,
    selected: usize,
}

struct PathBrowserEntry {
    label: String,
    path: PathBuf,
    kind: PathBrowserEntryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathBrowserEntryKind {
    Current,
    Parent,
    Directory,
}

impl PathBrowser {
    fn new(cwd: PathBuf) -> Self {
        let mut browser = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
        };
        browser.refresh();
        browser
    }

    fn refresh(&mut self) {
        let mut entries = Vec::new();
        entries.push(PathBrowserEntry {
            label: format!("[.] {}", self.cwd.display()),
            path: self.cwd.clone(),
            kind: PathBrowserEntryKind::Current,
        });

        if let Some(parent) = self.cwd.parent() {
            entries.push(PathBrowserEntry {
                label: format!("[..] {}", parent.display()),
                path: parent.to_path_buf(),
                kind: PathBrowserEntryKind::Parent,
            });
        }

        let mut dirs = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&self.cwd) {
            for entry in read_dir.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().trim().to_string();
                dirs.push((name.to_ascii_lowercase(), path, name));
            }
        }
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        entries.extend(dirs.into_iter().map(|(_, path, name)| PathBrowserEntry {
            label: format!("[D] {name}"),
            path,
            kind: PathBrowserEntryKind::Directory,
        }));

        self.entries = entries;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: i32) {
        if self.entries.is_empty() {
            self.selected = 0;
            return;
        }
        let max_index = self.entries.len().saturating_sub(1) as i32;
        self.selected = (self.selected as i32 + delta).clamp(0, max_index) as usize;
    }

    fn set_cwd(&mut self, cwd: PathBuf) {
        self.cwd = cwd;
        self.selected = 0;
        self.refresh();
    }

    fn selected_entry(&self) -> Option<&PathBrowserEntry> {
        self.entries.get(self.selected)
    }
}

impl PathPrompt {
    fn new(agent_index: usize, input: String) -> Self {
        let cwd = path_from_prompt(&input);
        let browser = PathBrowser::new(normalize_command_cwd(cwd));
        Self {
            agent_index,
            input,
            browser,
            browser_mode: false,
        }
    }

    fn sync_browser_from_input(&mut self) {
        let cwd = path_from_prompt(&self.input);
        if cwd.is_dir() {
            self.browser.set_cwd(normalize_command_cwd(cwd));
        }
    }
}

struct InstallPrompt {
    agent_index: usize,
}

#[derive(Clone, Copy)]
enum ResizeAxis {
    Row,
    Col,
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    axis: ResizeAxis,
    index: usize,
    start_position: u16,
    start_before: u16,
    start_after: u16,
}

struct App {
    panes: Vec<Pane>,
    active: usize,
    active_group: usize,
    group_count: usize,
    next_id: usize,
    layout: LayoutMode,
    show_help: bool,
    row_weights: Vec<u16>,
    col_weights: Vec<u16>,
    resize_drag: Option<ResizeDrag>,
    agents: Vec<AgentTool>,
    path_prompt: Option<PathPrompt>,
    install_prompt: Option<InstallPrompt>,
    shell: ShellSpec,
    tx: Sender<AppEvent>,
    language: Language,
    status: String,
    spinner: usize,
}

impl App {
    fn new(shell: ShellSpec, tx: Sender<AppEvent>) -> Result<Self> {
        let language = Language::from_env();
        let mut app = Self {
            panes: Vec::new(),
            active: 0,
            active_group: 0,
            group_count: 1,
            next_id: 1,
            layout: LayoutMode::Grid,
            show_help: false,
            row_weights: vec![100],
            col_weights: vec![100, 100],
            resize_drag: None,
            agents: detect_agents(),
            path_prompt: None,
            install_prompt: None,
            shell,
            tx,
            language,
            status: ui_text(language, "ready", "就绪").to_string(),
            spinner: 0,
        };
        app.new_pane()?;
        Ok(app)
    }

    fn drain_events(&mut self, rx: &Receiver<AppEvent>) {
        while let Ok(event) = rx.try_recv() {
            match event {
                AppEvent::PtyOutput { pane_id, bytes } => {
                    let is_active = self.active_pane_id() == Some(pane_id);
                    if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == pane_id) {
                        pane.parser.process(&bytes);
                        let replies = pane.parser.callbacks_mut().take_replies();
                        for reply in replies {
                            let _ = pane.send(&reply);
                        }
                        if !is_active {
                            pane.unread = pane.unread.saturating_add(1);
                        }
                    }
                }
                AppEvent::PaneExited { pane_id } => {
                    if let Some(pane) = self.panes.iter_mut().find(|pane| pane.id == pane_id) {
                        pane.exited = true;
                        pane.write_status("process exited; press Ctrl+R to respawn");
                    }
                }
            }
        }
    }

    fn active_pane_id(&self) -> Option<usize> {
        self.panes.get(self.active).map(|pane| pane.id)
    }

    fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(self.active)
    }

    fn visible_indices(&self) -> Vec<usize> {
        self.panes
            .iter()
            .enumerate()
            .filter_map(|(index, pane)| (pane.group == self.active_group).then_some(index))
            .collect()
    }

    fn group_pane_count(&self, group: usize) -> usize {
        self.panes.iter().filter(|pane| pane.group == group).count()
    }

    fn group_unread_count(&self, group: usize) -> usize {
        self.panes
            .iter()
            .filter(|pane| pane.group == group)
            .map(|pane| pane.unread)
            .sum()
    }

    fn focus_global_index(&mut self, index: usize) {
        if let Some(pane) = self.panes.get_mut(index) {
            self.active = index;
            pane.unread = 0;
        }
    }

    fn new_pane(&mut self) -> Result<()> {
        self.new_process_pane(self.shell.clone(), "agent")
    }

    fn new_process_pane(&mut self, command: ShellSpec, title_prefix: &str) -> Result<()> {
        let id = self.next_id;
        self.next_id += 1;
        let mut pane = Pane::new(
            id,
            format!("{title_prefix}-{id}"),
            self.active_group,
            command.clone(),
        );
        match spawn_pty_for_pane(&command, id, self.tx.clone()) {
            Ok(process) => {
                pane.writer = Some(process.writer);
                pane.master = Some(process.master);
                pane.child = Some(process.child);
                pane.exited = false;
            }
            Err(err) => {
                pane.write_status(&format!("failed to start process: {err:#}"));
                pane.write_status("set WIMX_SHELL, use --shell, or check the agent install");
                pane.exited = true;
            }
        }
        self.panes.push(pane);
        self.active = self.panes.len() - 1;
        self.group_count = self.group_count.max(self.active_group + 1);
        self.reset_layout_weights();
        self.status = format!(
            "{} {}",
            ui_text(self.language, "new pane in group", "新面板在分组"),
            self.active_group + 1
        );
        Ok(())
    }

    fn launch_agent(&mut self, agent_index: usize) -> Result<()> {
        let Some(agent) = self.agents.get(agent_index).cloned() else {
            return Ok(());
        };
        if agent.spec.is_none() {
            self.install_prompt = Some(InstallPrompt { agent_index });
            self.status = format!(
                "{} {}",
                agent.label,
                ui_text(self.language, "not installed", "未安装")
            );
            return Ok(());
        }
        let cwd =
            normalize_command_cwd(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        self.path_prompt = Some(PathPrompt::new(
            agent_index,
            cwd.to_string_lossy().to_string(),
        ));
        self.status = format!(
            "{} {}",
            ui_text(self.language, "path for", "路径"),
            agent.label
        );
        Ok(())
    }

    fn launch_agent_in_path(&mut self, agent_index: usize, cwd: PathBuf) -> Result<()> {
        let Some(agent) = self.agents.get(agent_index).cloned() else {
            return Ok(());
        };
        let Some(mut spec) = agent.spec else {
            self.status = format!(
                "{} {}",
                agent.label,
                ui_text(self.language, "not installed", "未安装")
            );
            return Ok(());
        };
        spec.cwd = Some(normalize_command_cwd(cwd));
        self.new_process_pane(spec, &agent.command)?;
        self.status = format!(
            "{} {}",
            ui_text(self.language, "launched", "已启动"),
            agent.label
        );
        Ok(())
    }

    fn install_agent(&mut self, agent_index: usize) -> Result<()> {
        let Some(agent) = self.agents.get(agent_index).cloned() else {
            return Ok(());
        };
        let Some(spec) = agent.install_spec else {
            self.status = format!(
                "{} {}",
                agent.label,
                ui_text(self.language, "no install command", "没有安装命令")
            );
            return Ok(());
        };
        self.install_prompt = None;
        self.new_process_pane(spec, "install")?;
        self.status = format!(
            "{} {}",
            ui_text(self.language, "installing", "正在安装"),
            agent.label
        );
        Ok(())
    }

    fn refresh_agents(&mut self) {
        self.agents = detect_agents();
        let installed = self.agents.iter().filter(|agent| agent.installed()).count();
        self.status = format!(
            "{} {installed}/{}",
            ui_text(self.language, "agents refreshed", "Agent 已刷新"),
            self.agents.len()
        );
    }

    fn close_active(&mut self) {
        if self.panes.len() <= 1 {
            self.set_status("keep one pane", "至少保留一个面板");
            return;
        }
        let mut pane = self.panes.remove(self.active);
        if let Some(child) = pane.child.as_mut() {
            let _ = child.kill();
        }
        if let Some(index) = self.visible_indices().first().copied() {
            self.focus_global_index(index);
        } else if let Some((index, pane)) = self.panes.iter().enumerate().next() {
            self.active_group = pane.group;
            self.focus_global_index(index);
        }
        self.reset_layout_weights();
        self.set_status("pane closed", "面板已关闭");
    }

    fn respawn_active(&mut self) {
        let Some(index) = self.panes.get(self.active).map(|_| self.active) else {
            return;
        };
        if let Some(child) = self.panes[index].child.as_mut() {
            let _ = child.kill();
        }
        let id = self.panes[index].id;
        let group = self.panes[index].group;
        let title = self.panes[index].title.clone();
        let command = self.panes[index].command.clone();
        self.panes[index] = Pane::new(id, title, group, command.clone());
        match spawn_pty_for_pane(&command, id, self.tx.clone()) {
            Ok(process) => {
                let pane = &mut self.panes[index];
                pane.writer = Some(process.writer);
                pane.master = Some(process.master);
                pane.child = Some(process.child);
                pane.exited = false;
                self.set_status("pane respawned", "面板已重启");
            }
            Err(err) => {
                self.panes[index].write_status(&format!("failed to respawn: {err:#}"));
                self.set_status("respawn failed", "重启失败");
            }
        }
    }

    fn focus_next(&mut self) {
        let visible = self.visible_indices();
        if !visible.is_empty() {
            let current = visible
                .iter()
                .position(|index| *index == self.active)
                .unwrap_or(0);
            self.focus_global_index(visible[(current + 1) % visible.len()]);
        }
    }

    fn focus_prev(&mut self) {
        let visible = self.visible_indices();
        if !visible.is_empty() {
            let current = visible
                .iter()
                .position(|index| *index == self.active)
                .unwrap_or(0);
            let next = if current == 0 {
                visible.len() - 1
            } else {
                current - 1
            };
            self.focus_global_index(visible[next]);
        }
    }

    fn switch_group(&mut self, group: usize) -> Result<()> {
        self.group_count = self.group_count.max(group + 1);
        self.active_group = group;
        if let Some(index) = self.visible_indices().first().copied() {
            self.focus_global_index(index);
        } else {
            self.new_pane()?;
        }
        self.reset_layout_weights();
        self.status = format!(
            "{} {}",
            ui_text(self.language, "group", "分组"),
            self.active_group + 1
        );
        Ok(())
    }

    fn new_group(&mut self) -> Result<()> {
        let group = self.group_count;
        self.group_count += 1;
        self.active_group = group;
        self.new_pane()?;
        self.status = format!(
            "{} {}",
            ui_text(self.language, "new group", "新分组"),
            group + 1
        );
        Ok(())
    }

    fn move_active_to_next_group(&mut self) -> Result<()> {
        let next_group = if self.group_count <= 1 {
            self.group_count = 2;
            1
        } else {
            (self.active_group + 1) % self.group_count
        };
        let moved_index = self.active;
        if let Some(pane) = self.panes.get_mut(moved_index) {
            pane.group = next_group;
        }
        self.active_group = next_group;
        self.group_count = self.group_count.max(next_group + 1);
        self.focus_global_index(moved_index);
        self.reset_layout_weights();
        self.status = format!(
            "{} {}",
            ui_text(self.language, "moved pane to group", "面板已移动到分组"),
            next_group + 1
        );
        Ok(())
    }

    fn reset_layout_weights(&mut self) {
        let visible_count = self.visible_indices().len().max(1);
        let rows = layout_rows(visible_count, self.layout);
        let cols = layout_cols(visible_count, self.layout);
        self.row_weights = vec![100; rows.max(1)];
        self.col_weights = vec![100; cols.max(1)];
        if self.col_weights.len() == 1 {
            self.col_weights.push(100);
        }
    }

    fn set_status(&mut self, en: &'static str, zh: &'static str) {
        self.status = ui_text(self.language, en, zh).to_string();
    }

    fn toggle_language(&mut self) {
        self.language = self.language.toggle();
        self.status = match self.language {
            Language::En => "language English".to_string(),
            Language::Zh => "语言 中文".to_string(),
        };
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.install_prompt.is_some() {
            return self.handle_install_prompt_key(key);
        }
        if self.path_prompt.is_some() {
            return self.handle_path_prompt_key(key);
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            if let KeyCode::Char(c) = key.code {
                if let Some(group) = digit_group(c) {
                    self.switch_group(group)?;
                    return Ok(false);
                }
            }
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char(c) if digit_group(c).is_some() => {
                    self.switch_group(digit_group(c).unwrap())?;
                    return Ok(false);
                }
                KeyCode::Char('n') => {
                    self.new_pane()?;
                    return Ok(false);
                }
                KeyCode::Char('w') => {
                    self.close_active();
                    return Ok(false);
                }
                KeyCode::Char('j') => {
                    self.focus_next();
                    return Ok(false);
                }
                KeyCode::Char('k') => {
                    self.focus_prev();
                    return Ok(false);
                }
                KeyCode::Char('l') => {
                    self.layout = if self.layout == LayoutMode::Grid {
                        LayoutMode::Stack
                    } else {
                        LayoutMode::Grid
                    };
                    self.reset_layout_weights();
                    return Ok(false);
                }
                KeyCode::Char('g') => {
                    self.new_group()?;
                    return Ok(false);
                }
                KeyCode::Char('o') => {
                    self.move_active_to_next_group()?;
                    return Ok(false);
                }
                KeyCode::Char('h') => {
                    self.show_help = !self.show_help;
                    return Ok(false);
                }
                KeyCode::Char('r') => {
                    self.respawn_active();
                    return Ok(false);
                }
                KeyCode::Char('t') => {
                    self.toggle_language();
                    return Ok(false);
                }
                KeyCode::Char('a') => {
                    self.refresh_agents();
                    return Ok(false);
                }
                _ => {}
            }
        }

        if let Some(bytes) = key_to_pty_bytes(key) {
            if let Some(pane) = self.active_pane_mut() {
                pane.reset_scrollback();
                pane.send(&bytes)?;
            }
        }
        Ok(false)
    }

    fn handle_path_prompt_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('u') => {
                    if let Some(prompt) = self.path_prompt.as_mut() {
                        prompt.input.clear();
                        prompt.browser_mode = false;
                    }
                    return Ok(false);
                }
                KeyCode::Char('h') => {
                    if let Some(prompt) = self.path_prompt.as_mut() {
                        prompt.input.pop();
                        prompt.browser_mode = false;
                    }
                    return Ok(false);
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => {
                self.path_prompt = None;
                self.set_status("agent launch canceled", "已取消启动 Agent");
            }
            KeyCode::Tab => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.sync_browser_from_input();
                    prompt.browser_mode = true;
                }
            }
            KeyCode::Up => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.browser.move_selection(-1);
                    prompt.browser_mode = true;
                }
            }
            KeyCode::Down => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.browser.move_selection(1);
                    prompt.browser_mode = true;
                }
            }
            KeyCode::PageUp => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.browser.move_selection(-5);
                    prompt.browser_mode = true;
                }
            }
            KeyCode::PageDown => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.browser.move_selection(5);
                    prompt.browser_mode = true;
                }
            }
            KeyCode::Left => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    if let Some(parent) = prompt.browser.cwd.parent() {
                        prompt.browser.set_cwd(parent.to_path_buf());
                        prompt.input = prompt.browser.cwd.to_string_lossy().to_string();
                        prompt.browser_mode = true;
                    }
                }
            }
            KeyCode::Right => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    if let Some(entry) = prompt.browser.selected_entry() {
                        if entry.kind != PathBrowserEntryKind::Current {
                            prompt.browser.set_cwd(entry.path.clone());
                            prompt.input = prompt.browser.cwd.to_string_lossy().to_string();
                            prompt.browser_mode = true;
                        }
                    }
                }
            }
            KeyCode::Enter => {
                let Some(prompt) = self.path_prompt.as_ref() else {
                    return Ok(false);
                };
                let agent_index = prompt.agent_index;
                let cwd = if prompt.browser_mode {
                    prompt
                        .browser
                        .selected_entry()
                        .map(|entry| entry.path.clone())
                        .unwrap_or_else(|| path_from_prompt(&prompt.input))
                } else {
                    path_from_prompt(&prompt.input)
                };
                if !cwd.exists() {
                    self.status = format!(
                        "{}: {}",
                        ui_text(self.language, "path not found", "路径不存在"),
                        cwd.display()
                    );
                    return Ok(false);
                }
                if !cwd.is_dir() {
                    self.status = format!(
                        "{}: {}",
                        ui_text(self.language, "not a directory", "不是目录"),
                        cwd.display()
                    );
                    return Ok(false);
                }
                let cwd = cwd
                    .canonicalize()
                    .map(normalize_command_cwd)
                    .unwrap_or_else(|_| normalize_command_cwd(cwd));
                self.path_prompt = None;
                self.launch_agent_in_path(agent_index, cwd)?;
            }
            KeyCode::Backspace => {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.input.pop();
                    prompt.browser_mode = false;
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                if let Some(prompt) = self.path_prompt.as_mut() {
                    prompt.input.push(c);
                    prompt.browser_mode = false;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_install_prompt_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.install_prompt = None;
                self.set_status("install canceled", "已取消安装");
            }
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                let Some(prompt) = self.install_prompt.take() else {
                    return Ok(false);
                };
                self.install_agent(prompt.agent_index)?;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_path_prompt_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        let (cols, rows) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let area = centered_fixed_rect(96, 20, Rect::new(0, 0, cols.max(1), rows.max(1)));
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(4),
            ])
            .split(area);
        let browser_area = layout[1];

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
            && point_in_rect(browser_area, mouse.column, mouse.row)
        {
            let inner = Rect::new(
                browser_area.x + 1,
                browser_area.y + 1,
                browser_area.width.saturating_sub(2),
                browser_area.height.saturating_sub(2),
            );
            if point_in_rect(inner, mouse.column, mouse.row) {
                let index = usize::from(mouse.row.saturating_sub(inner.y));
                if let Some(prompt) = self.path_prompt.as_mut() {
                    if let Some(entry) = prompt.browser.entries.get(index) {
                        let path = entry.path.clone();
                        let kind = entry.kind;
                        prompt.browser.selected = index;
                        prompt.browser_mode = true;
                        match kind {
                            PathBrowserEntryKind::Current => {
                                prompt.input = prompt.browser.cwd.to_string_lossy().to_string();
                            }
                            PathBrowserEntryKind::Parent | PathBrowserEntryKind::Directory => {
                                prompt.browser.set_cwd(path);
                                prompt.input = prompt.browser.cwd.to_string_lossy().to_string();
                            }
                        }
                    }
                }
            }
            return Ok(());
        }

        Ok(())
    }

    fn resize_drag_from_point(&self, pane_area: Rect, x: u16, y: u16) -> Option<ResizeDrag> {
        let visible_count = self.visible_indices().len();
        if visible_count <= 1 {
            return None;
        }
        let rects = pane_rects(
            visible_count,
            self.layout,
            pane_area,
            &self.row_weights,
            &self.col_weights,
        );
        if self.layout == LayoutMode::Stack {
            for index in 0..rects.len().saturating_sub(1) {
                let sep_y = rects[index].y + rects[index].height.saturating_sub(1);
                if is_near(y, sep_y) {
                    return Some(ResizeDrag {
                        axis: ResizeAxis::Row,
                        index,
                        start_position: y,
                        start_before: self.row_weights.get(index).copied().unwrap_or(100),
                        start_after: self.row_weights.get(index + 1).copied().unwrap_or(100),
                    });
                }
            }
            return None;
        }

        let cols = layout_cols(visible_count, self.layout);
        let rows = layout_rows(visible_count, self.layout);
        if cols > 1 {
            for row in 0..rows {
                let left = row * cols;
                let right = left + 1;
                if right >= rects.len() {
                    continue;
                }
                let sep_x = rects[left].x + rects[left].width.saturating_sub(1);
                if is_near(x, sep_x) && y >= rects[left].y && y < rects[left].y + rects[left].height
                {
                    return Some(ResizeDrag {
                        axis: ResizeAxis::Col,
                        index: 0,
                        start_position: x,
                        start_before: self.col_weights.first().copied().unwrap_or(100),
                        start_after: self.col_weights.get(1).copied().unwrap_or(100),
                    });
                }
            }
        }
        for row in 0..rows.saturating_sub(1) {
            let top = row * cols;
            let bottom = (row + 1) * cols;
            if bottom >= rects.len() {
                continue;
            }
            let sep_y = rects[top].y + rects[top].height.saturating_sub(1);
            if is_near(y, sep_y) && x >= pane_area.x && x < pane_area.x + pane_area.width {
                return Some(ResizeDrag {
                    axis: ResizeAxis::Row,
                    index: row,
                    start_position: y,
                    start_before: self.row_weights.get(row).copied().unwrap_or(100),
                    start_after: self.row_weights.get(row + 1).copied().unwrap_or(100),
                });
            }
        }
        None
    }

    fn apply_resize_drag(&mut self, mouse: MouseEvent, pane_area: Rect) {
        let Some(drag) = self.resize_drag else {
            return;
        };
        match drag.axis {
            ResizeAxis::Row => {
                adjust_weight_pair(
                    &mut self.row_weights,
                    drag.index,
                    i32::from(mouse.row) - i32::from(drag.start_position),
                    pane_area.height.max(1),
                    drag.start_before,
                    drag.start_after,
                );
            }
            ResizeAxis::Col => {
                adjust_weight_pair(
                    &mut self.col_weights,
                    drag.index,
                    i32::from(mouse.column) - i32::from(drag.start_position),
                    pane_area.width.max(1),
                    drag.start_before,
                    drag.start_after,
                );
            }
        }
        self.set_status("resized panes", "已调整面板大小");
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if self.install_prompt.is_some() {
            return Ok(());
        }
        if self.path_prompt.is_some() {
            return self.handle_path_prompt_mouse(mouse);
        }

        let (cols, rows) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let area = Rect::new(0, 0, cols.max(1), rows.max(1));
        let ui = compute_ui_regions(area);

        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) if self.resize_drag.is_some() => {
                self.apply_resize_drag(mouse, ui.pane_area);
                return Ok(());
            }
            MouseEventKind::Up(MouseButton::Left) if self.resize_drag.is_some() => {
                self.resize_drag = None;
                self.set_status("resize done", "调整完成");
                return Ok(());
            }
            _ => {}
        }

        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            if let Some(group) =
                group_index_from_point(ui.group_list, self.group_count, mouse.column, mouse.row)
            {
                self.switch_group(group)?;
                return Ok(());
            }
            if let Some(agent_index) =
                agent_index_from_point(ui.agent_list, self.agents.len(), mouse.column, mouse.row)
            {
                self.launch_agent(agent_index)?;
                return Ok(());
            }
            if let Some(index) = pane_list_index_from_point(
                ui.pane_list,
                self.visible_indices().len(),
                mouse.column,
                mouse.row,
            ) {
                let visible = self.visible_indices();
                if let Some(pane_index) = visible.get(index).copied() {
                    self.focus_global_index(pane_index);
                }
                self.status = format!(
                    "{} {}",
                    ui_text(self.language, "focus", "聚焦"),
                    self.panes[self.active].title
                );
                return Ok(());
            }
            if let Some(drag) = self.resize_drag_from_point(ui.pane_area, mouse.column, mouse.row) {
                self.resize_drag = Some(drag);
                self.set_status("resize drag", "拖动调整大小");
                return Ok(());
            }
        }

        let visible = self.visible_indices();
        let pane_rects = pane_rects(
            visible.len(),
            self.layout,
            ui.pane_area,
            &self.row_weights,
            &self.col_weights,
        );
        let Some((pane_index, pane_rect)) =
            pane_index_from_point(&pane_rects, mouse.column, mouse.row)
        else {
            return Ok(());
        };
        let Some(global_pane_index) = visible.get(pane_index).copied() else {
            return Ok(());
        };

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if global_pane_index != self.active {
                    self.focus_global_index(global_pane_index);
                    self.status = format!(
                        "{} {}",
                        ui_text(self.language, "focus", "聚焦"),
                        self.panes[self.active].title
                    );
                    return Ok(());
                }
                if let Some(bytes) =
                    mouse_to_pty_bytes(mouse, pane_rect, &self.panes[global_pane_index])
                {
                    if let Some(pane) = self.panes.get_mut(global_pane_index) {
                        pane.send(&bytes)?;
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                self.focus_global_index(global_pane_index);
                if let Some(pane) = self.active_pane_mut() {
                    pane.scroll_scrollback(3);
                }
                self.set_status("scrollback up", "向上滚动历史");
            }
            MouseEventKind::ScrollDown => {
                self.focus_global_index(global_pane_index);
                if let Some(pane) = self.active_pane_mut() {
                    pane.scroll_scrollback(-3);
                    if pane.scrollback() == 0 {
                        self.set_status("scrollback live", "回到实时输出");
                    } else {
                        self.set_status("scrollback down", "向下滚动历史");
                    }
                }
            }
            MouseEventKind::Drag(_)
            | MouseEventKind::Moved
            | MouseEventKind::Up(_)
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                if global_pane_index == self.active {
                    if let Some(bytes) =
                        mouse_to_pty_bytes(mouse, pane_rect, &self.panes[global_pane_index])
                    {
                        if let Some(pane) = self.panes.get_mut(global_pane_index) {
                            pane.send(&bytes)?;
                        }
                    }
                }
            }
            MouseEventKind::Down(_) => {}
        }

        Ok(())
    }

    fn send_text(&mut self, text: &str) -> Result<()> {
        if let Some(prompt) = self.path_prompt.as_mut() {
            let pasted = text.trim_end_matches(['\r', '\n']);
            prompt.input.push_str(pasted);
            return Ok(());
        }

        if let Some(pane) = self.active_pane_mut() {
            pane.reset_scrollback();
            pane.send(text.as_bytes())?;
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        for pane in &mut self.panes {
            if let Some(child) = pane.child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}

struct SpawnedPty {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

fn spawn_pty_for_pane(
    shell: &ShellSpec,
    pane_id: usize,
    tx: Sender<AppEvent>,
) -> Result<SpawnedPty> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;
    let child = pair
        .slave
        .spawn_command(shell.command_builder())
        .with_context(|| format!("spawn {}", shell.label()))?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("clone reader")?;
    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().context("take writer")?,
    ));
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(AppEvent::PaneExited { pane_id });
                    break;
                }
                Ok(n) => {
                    let _ = tx.send(AppEvent::PtyOutput {
                        pane_id,
                        bytes: buf[..n].to_vec(),
                    });
                }
                Err(_) => {
                    let _ = tx.send(AppEvent::PaneExited { pane_id });
                    break;
                }
            }
        }
    });

    Ok(SpawnedPty {
        writer,
        master: pair.master,
        child,
    })
}

fn key_to_pty_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                control_byte(c).map(|byte| vec![byte])
            } else {
                Some(c.to_string().into_bytes())
            }
        }
        KeyCode::Enter => Some(b"\r".to_vec()),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        _ => None,
    }
}

fn control_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    if c.is_ascii_lowercase() {
        Some((c as u8) - b'a' + 1)
    } else {
        None
    }
}

fn digit_group(c: char) -> Option<usize> {
    c.to_digit(10)
        .and_then(|digit| (1..=9).contains(&digit).then_some(digit as usize - 1))
}

fn path_from_prompt(input: &str) -> PathBuf {
    let trimmed = unquote_path_input(input.trim());
    let path = if trimmed.is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        expand_user_path(trimmed)
    };

    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn unquote_path_input(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn expand_user_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(value)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let ui = compute_ui_regions(area);

    draw_sidebar(frame, app, ui.sidebar);
    draw_workspace(frame, app, ui.workspace);

    if app.show_help {
        draw_help(frame, centered_rect(58, 42, area), app.language);
    }
    if app.install_prompt.is_some() {
        draw_install_prompt(frame, app, centered_fixed_rect(64, 9, area));
    }
    if app.path_prompt.is_some() {
        draw_path_prompt(frame, app, centered_fixed_rect(96, 20, area));
    }
}

fn draw_sidebar(frame: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(" wx ", Style::default().fg(Color::Black).bg(Color::Green)),
            Span::raw(ui_text(app.language, "  Wimx raw", "  Wimx 原生")),
        ]),
        Line::from(Span::styled(
            format!("{}  {}", app.shell.label(), app.language.code()),
            Style::default().fg(Color::DarkGray),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title(" cmux "));
    frame.render_widget(header, chunks[0]);

    let group_items: Vec<ListItem> = (0..app.group_count)
        .map(|group| {
            let marker = if group == app.active_group { ">" } else { " " };
            let unread = app.group_unread_count(group);
            let unread = if unread > 0 {
                format!(" +{unread}")
            } else {
                String::new()
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(Color::Cyan)),
                Span::raw(" "),
                Span::styled(
                    format!("{}-{}", ui_text(app.language, "group", "分组"), group + 1),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" [{}]", app.group_pane_count(group)),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(unread, Style::default().fg(Color::Yellow)),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(group_items).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " groups ",
            " 分组 ",
        ))),
        chunks[1],
    );

    let agent_items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|agent| {
            let (state, style) = if agent.installed() {
                ("ok", Style::default().fg(Color::Green))
            } else {
                ("--", Style::default().fg(Color::DarkGray))
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{state}] "), style),
                Span::styled(
                    agent.label,
                    if agent.installed() {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(agent_items).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " agents ",
            " Agent ",
        ))),
        chunks[2],
    );

    let visible = app.visible_indices();
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(visible_index, pane_index)| {
            let pane = &app.panes[*pane_index];
            let marker = if *pane_index == app.active { ">" } else { " " };
            let unread = if pane.unread > 0 {
                format!(" +{}", pane.unread)
            } else {
                String::new()
            };
            let state = if pane.exited {
                ui_text(app.language, "off", "停")
            } else {
                ui_text(app.language, "run", "运行")
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Cyan)),
                    Span::raw(" "),
                    Span::styled(
                        format!("{} {}", visible_index + 1, pane.title),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(unread, Style::default().fg(Color::Yellow)),
                ]),
                Line::from(vec![
                    Span::styled(
                        format!("  {state} "),
                        if pane.exited {
                            Style::default().fg(Color::Red)
                        } else {
                            Style::default().fg(Color::Green)
                        },
                    ),
                    Span::styled(
                        format!("{} {}", ui_text(app.language, "pane", "面板"), pane.id),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            ])
        })
        .collect();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " panes ",
            " 面板 ",
        ))),
        chunks[3],
    );

    let layout = if app.layout == LayoutMode::Grid {
        ui_text(app.language, "grid", "网格")
    } else {
        ui_text(app.language, "stack", "堆叠")
    };
    let footer = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "layout ", "布局 "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(layout),
        ]),
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "status ", "状态 "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(&app.status),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL).title(ui_text(
        app.language,
        " status ",
        " 状态 ",
    )));
    frame.render_widget(footer, chunks[4]);
}

fn draw_workspace(frame: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    let title = app
        .panes
        .get(app.active)
        .map(|pane| {
            if pane.scrollback() > 0 {
                format!(
                    "{}-{} / {}  [{} {}]  |  Ctrl+H {}  Ctrl+T {}  Ctrl+Q {}",
                    ui_text(app.language, "group", "分组"),
                    app.active_group + 1,
                    pane.title,
                    ui_text(app.language, "scroll", "历史"),
                    pane.scrollback(),
                    ui_text(app.language, "help", "帮助"),
                    ui_text(app.language, "lang", "语言"),
                    ui_text(app.language, "quit", "退出")
                )
            } else {
                format!(
                    "{}-{} / {}  |  Ctrl+H {}  Ctrl+T {}  Ctrl+Q {}",
                    ui_text(app.language, "group", "分组"),
                    app.active_group + 1,
                    pane.title,
                    ui_text(app.language, "help", "帮助"),
                    ui_text(app.language, "lang", "语言"),
                    ui_text(app.language, "quit", "退出")
                )
            }
        })
        .unwrap_or_else(|| "no pane".to_string());
    frame.render_widget(
        Paragraph::new(title).block(Block::default().borders(Borders::ALL).title(" active ")),
        chunks[0],
    );

    draw_panes(frame, app, chunks[1]);

    let hint = format!(
        "{} {}   {}   Ctrl+G {}  Ctrl+O {}  Alt+1..9 {}",
        SPINNER[app.spinner],
        ui_text(app.language, "direct PTY input", "直接 PTY 输入"),
        ui_text(app.language, "drag borders resize", "拖动边框调整大小"),
        ui_text(app.language, "group", "分组"),
        ui_text(app.language, "move", "移动"),
        ui_text(app.language, "switch", "切换")
    );
    frame.render_widget(Paragraph::new(hint), chunks[2]);
}

fn draw_panes(frame: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_indices();
    let rects = pane_rects(
        visible.len(),
        app.layout,
        area,
        &app.row_weights,
        &app.col_weights,
    );
    for (visible_index, rect) in rects.into_iter().enumerate() {
        let pane_index = visible[visible_index];
        let pane = &mut app.panes[pane_index];
        let inner_rows = rect.height.saturating_sub(2).max(1);
        let inner_cols = rect.width.saturating_sub(2).max(1);
        pane.resize(inner_rows, inner_cols);

        let contents = screen_to_lines(pane.parser.screen());
        let focused = pane_index == app.active;
        let border_style = if focused {
            Style::default().fg(Color::Cyan)
        } else if pane.exited {
            Style::default().fg(Color::Red)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let title = if focused {
            format!(" {} * ", pane.title)
        } else {
            format!(" {} ", pane.title)
        };
        let paragraph = Paragraph::new(contents).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        );
        frame.render_widget(paragraph, rect);

        if focused && !pane.exited && !pane.parser.screen().hide_cursor() {
            let (cursor_row, cursor_col) = pane.parser.screen().cursor_position();
            let cursor_x = rect.x + 1 + cursor_col.min(inner_cols.saturating_sub(1));
            let cursor_y = rect.y + 1 + cursor_row.min(inner_rows.saturating_sub(1));
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

fn screen_to_lines(screen: &vt100::Screen) -> Vec<Line<'static>> {
    let (rows, cols) = screen.size();
    let mut lines = Vec::with_capacity(usize::from(rows));
    for row in 0..rows {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut current_style: Option<Style> = None;
        let mut current_text = String::new();

        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }

            let style = cell_style(cell);
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };

            if current_style.is_some_and(|current| current == style) {
                current_text.push_str(text);
            } else {
                if let Some(style) = current_style {
                    spans.push(Span::styled(std::mem::take(&mut current_text), style));
                }
                current_style = Some(style);
                current_text.push_str(text);
            }
        }

        if let Some(style) = current_style {
            spans.push(Span::styled(current_text, style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut fg = vt_color_to_ratatui(cell.fgcolor());
    let mut bg = vt_color_to_ratatui(cell.bgcolor());
    if cell.inverse() {
        std::mem::swap(&mut fg, &mut bg);
        fg.get_or_insert(Color::Black);
        bg.get_or_insert(Color::White);
    }

    let mut style = Style::default();
    if let Some(color) = fg {
        style = style.fg(color);
    }
    if let Some(color) = bg {
        style = style.bg(color);
    }
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.dim() {
        style = style.add_modifier(Modifier::DIM);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn vt_color_to_ratatui(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(Color::Indexed(index)),
        vt100::Color::Rgb(red, green, blue) => Some(Color::Rgb(red, green, blue)),
    }
}

fn pane_rects(
    count: usize,
    mode: LayoutMode,
    area: Rect,
    row_weights: &[u16],
    col_weights: &[u16],
) -> Vec<Rect> {
    if count <= 1 {
        return vec![area];
    }
    if mode == LayoutMode::Stack {
        return Layout::default()
            .direction(Direction::Vertical)
            .constraints(weight_constraints(row_weights, count))
            .split(area)
            .to_vec();
    }

    let cols = layout_cols(count, mode);
    let rows = layout_rows(count, mode);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(weight_constraints(row_weights, rows))
        .split(area);
    let mut rects = Vec::with_capacity(count);
    for row in 0..rows {
        let col_count = (count - row * cols).min(cols);
        let col_rects = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(weight_constraints(col_weights, col_count))
            .split(row_rects[row]);
        rects.extend(col_rects.iter().take(col_count).copied());
    }
    rects
}

fn layout_cols(count: usize, mode: LayoutMode) -> usize {
    if mode == LayoutMode::Stack {
        1
    } else if count <= 2 {
        count.max(1)
    } else {
        2
    }
}

fn layout_rows(count: usize, mode: LayoutMode) -> usize {
    if mode == LayoutMode::Stack {
        count.max(1)
    } else {
        let cols = layout_cols(count, mode);
        (count + cols - 1) / cols
    }
}

fn weight_constraints(weights: &[u16], count: usize) -> Vec<Constraint> {
    if count <= 1 {
        return vec![Constraint::Percentage(100)];
    }
    let total: u32 = weights
        .iter()
        .take(count)
        .map(|weight| u32::from((*weight).max(1)))
        .sum();
    (0..count)
        .map(|index| {
            let weight = weights.get(index).copied().unwrap_or(100).max(1);
            Constraint::Ratio(u32::from(weight), total.max(1))
        })
        .collect()
}

fn is_near(value: u16, target: u16) -> bool {
    i32::from(value).abs_diff(i32::from(target)) <= 1
}

fn adjust_weight_pair(
    weights: &mut [u16],
    index: usize,
    delta_cells: i32,
    span: u16,
    start_before: u16,
    start_after: u16,
) {
    if index + 1 >= weights.len() {
        return;
    }
    let pair_total = i32::from(start_before) + i32::from(start_after);
    let delta_weight = delta_cells * pair_total / i32::from(span.max(1));
    let min_weight = 10;
    let before =
        (i32::from(start_before) + delta_weight).clamp(min_weight, pair_total - min_weight);
    let after = pair_total - before;
    weights[index] = before as u16;
    weights[index + 1] = after as u16;
}

#[derive(Clone, Copy)]
struct UiRegions {
    sidebar: Rect,
    workspace: Rect,
    group_list: Rect,
    agent_list: Rect,
    pane_list: Rect,
    pane_area: Rect,
}

fn compute_ui_regions(area: Rect) -> UiRegions {
    let root = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(31), Constraint::Min(50)])
        .split(area);
    let sidebar_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Min(8),
            Constraint::Length(5),
        ])
        .split(root[0]);
    let workspace_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(root[1]);
    UiRegions {
        sidebar: root[0],
        workspace: root[1],
        group_list: sidebar_chunks[1],
        agent_list: sidebar_chunks[2],
        pane_list: sidebar_chunks[3],
        pane_area: workspace_chunks[1],
    }
}

fn point_in_rect(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn pane_list_index_from_point(
    pane_list_rect: Rect,
    pane_count: usize,
    x: u16,
    y: u16,
) -> Option<usize> {
    if pane_count == 0 || pane_list_rect.width < 3 || pane_list_rect.height < 3 {
        return None;
    }
    if !point_in_rect(pane_list_rect, x, y) {
        return None;
    }
    let inner = Rect::new(
        pane_list_rect.x + 1,
        pane_list_rect.y + 1,
        pane_list_rect.width.saturating_sub(2),
        pane_list_rect.height.saturating_sub(2),
    );
    if !point_in_rect(inner, x, y) {
        return None;
    }
    let rel_y = y.saturating_sub(inner.y);
    let index = usize::from(rel_y / 2);
    (index < pane_count).then_some(index)
}

fn group_index_from_point(
    group_list_rect: Rect,
    group_count: usize,
    x: u16,
    y: u16,
) -> Option<usize> {
    if group_count == 0 || group_list_rect.width < 3 || group_list_rect.height < 3 {
        return None;
    }
    if !point_in_rect(group_list_rect, x, y) {
        return None;
    }
    let inner = Rect::new(
        group_list_rect.x + 1,
        group_list_rect.y + 1,
        group_list_rect.width.saturating_sub(2),
        group_list_rect.height.saturating_sub(2),
    );
    if !point_in_rect(inner, x, y) {
        return None;
    }
    let index = usize::from(y.saturating_sub(inner.y));
    (index < group_count).then_some(index)
}

fn agent_index_from_point(
    agent_list_rect: Rect,
    agent_count: usize,
    x: u16,
    y: u16,
) -> Option<usize> {
    if agent_count == 0 || agent_list_rect.width < 3 || agent_list_rect.height < 3 {
        return None;
    }
    if !point_in_rect(agent_list_rect, x, y) {
        return None;
    }
    let inner = Rect::new(
        agent_list_rect.x + 1,
        agent_list_rect.y + 1,
        agent_list_rect.width.saturating_sub(2),
        agent_list_rect.height.saturating_sub(2),
    );
    if !point_in_rect(inner, x, y) {
        return None;
    }
    let index = usize::from(y.saturating_sub(inner.y));
    (index < agent_count).then_some(index)
}

fn pane_index_from_point(pane_rects: &[Rect], x: u16, y: u16) -> Option<(usize, Rect)> {
    pane_rects
        .iter()
        .copied()
        .enumerate()
        .find(|(_, rect)| point_in_rect(*rect, x, y))
}

fn mouse_to_pty_bytes(mouse: MouseEvent, pane_rect: Rect, pane: &Pane) -> Option<Vec<u8>> {
    if pane.exited || pane_rect.width < 3 || pane_rect.height < 3 {
        return None;
    }
    let inner = Rect::new(
        pane_rect.x + 1,
        pane_rect.y + 1,
        pane_rect.width.saturating_sub(2),
        pane_rect.height.saturating_sub(2),
    );
    if !point_in_rect(inner, mouse.column, mouse.row) {
        return None;
    }

    let screen = pane.parser.screen();
    let mode = screen.mouse_protocol_mode();
    if mode == vt100::MouseProtocolMode::None {
        return None;
    }
    if !mouse_kind_allowed(mode, mouse.kind) {
        return None;
    }

    let (mut code, release_suffix) = mouse_code(mouse.kind)?;
    code += mouse_modifier_bits(mouse.modifiers);

    let x = mouse.column.saturating_sub(inner.x) + 1;
    let y = mouse.row.saturating_sub(inner.y) + 1;

    let bytes = match screen.mouse_protocol_encoding() {
        vt100::MouseProtocolEncoding::Sgr => format!(
            "\x1b[<{};{};{}{}",
            code,
            x,
            y,
            if release_suffix { "m" } else { "M" }
        )
        .into_bytes(),
        vt100::MouseProtocolEncoding::Utf8 => encode_x10_mouse(code, x, y, true)?,
        vt100::MouseProtocolEncoding::Default => encode_x10_mouse(code, x, y, false)?,
    };
    Some(bytes)
}

fn mouse_kind_allowed(mode: vt100::MouseProtocolMode, kind: MouseEventKind) -> bool {
    match mode {
        vt100::MouseProtocolMode::None => false,
        vt100::MouseProtocolMode::Press => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::PressRelease => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::ButtonMotion => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::AnyMotion => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::Moved
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
    }
}

fn mouse_code(kind: MouseEventKind) -> Option<(u16, bool)> {
    match kind {
        MouseEventKind::Down(MouseButton::Left) => Some((0, false)),
        MouseEventKind::Down(MouseButton::Middle) => Some((1, false)),
        MouseEventKind::Down(MouseButton::Right) => Some((2, false)),
        MouseEventKind::Up(_) => Some((3, true)),
        MouseEventKind::Drag(MouseButton::Left) => Some((32, false)),
        MouseEventKind::Drag(MouseButton::Middle) => Some((33, false)),
        MouseEventKind::Drag(MouseButton::Right) => Some((34, false)),
        MouseEventKind::Moved => Some((35, false)),
        MouseEventKind::ScrollUp => Some((64, false)),
        MouseEventKind::ScrollDown => Some((65, false)),
        MouseEventKind::ScrollLeft => Some((66, false)),
        MouseEventKind::ScrollRight => Some((67, false)),
    }
}

fn mouse_modifier_bits(modifiers: KeyModifiers) -> u16 {
    let mut bits = 0u16;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits += 16;
    }
    bits
}

fn encode_x10_mouse(code: u16, x: u16, y: u16, utf8_mode: bool) -> Option<Vec<u8>> {
    let mut out = b"\x1b[M".to_vec();
    encode_x10_field(&mut out, code + 32, utf8_mode)?;
    encode_x10_field(&mut out, x + 32, utf8_mode)?;
    encode_x10_field(&mut out, y + 32, utf8_mode)?;
    Some(out)
}

fn encode_x10_field(out: &mut Vec<u8>, value: u16, utf8_mode: bool) -> Option<()> {
    if utf8_mode {
        let ch = char::from_u32(u32::from(value))?;
        out.extend(ch.to_string().into_bytes());
        return Some(());
    }

    let clamped = value.min(255);
    out.push(u8::try_from(clamped).ok()?);
    Some(())
}

fn draw_path_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let Some(prompt) = &app.path_prompt else {
        return;
    };
    let agent = app.agents.get(prompt.agent_index);
    let agent_label =
        agent
            .map(|agent| agent.label)
            .unwrap_or(ui_text(app.language, "agent", "Agent"));
    let command = agent.map(|agent| agent.command.as_str()).unwrap_or("");

    frame.render_widget(Clear, area);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(area);
    let header_area = layout[0];
    let browser_area = layout[1];
    let footer_area = layout[2];

    let path_label = ui_text(app.language, "Path  ", "路径  ");
    let input_width = usize::from(header_area.width.saturating_sub(4))
        .saturating_sub(UnicodeWidthStr::width(path_label));
    let input = tail_to_width(&prompt.input, input_width);

    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "Agent ", "Agent "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(agent_label, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled(command, Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled(path_label, Style::default().fg(Color::DarkGray)),
            Span::raw(input.as_str()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(header_lines).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " agent path ",
            " Agent 路径 ",
        ))),
        header_area,
    );

    let browser_lines: Vec<ListItem> = prompt
        .browser
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let selected = index == prompt.browser.selected;
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(entry.label.clone(), style)))
        })
        .collect();
    frame.render_widget(
        List::new(browser_lines).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " browser ",
            " 浏览 ",
        ))),
        browser_area,
    );

    let footer_lines = vec![
        Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(ui_text(app.language, " launch  ", " 启动  ")),
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(ui_text(app.language, " sync  ", " 同步  ")),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(ui_text(app.language, " cancel", " 取消")),
        ]),
        Line::from(ui_text(
            app.language,
            "Up/Down browse, Left parent, Right open selected directory.",
            "上下浏览，Left 返回上级，Right 打开选中目录。",
        )),
    ];
    frame.render_widget(
        Paragraph::new(footer_lines).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " browser help ",
            " 浏览帮助 ",
        ))),
        footer_area,
    );

    if header_area.width > 4 && header_area.height > 2 {
        let cursor_offset =
            UnicodeWidthStr::width(path_label) + UnicodeWidthStr::width(input.as_str());
        let cursor_x = header_area
            .x
            .saturating_add(1)
            .saturating_add(u16::try_from(cursor_offset).unwrap_or(u16::MAX))
            .min(header_area.x + header_area.width.saturating_sub(2));
        let cursor_y =
            (header_area.y + 2).min(header_area.y + header_area.height.saturating_sub(2));
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

fn draw_install_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let Some(prompt) = &app.install_prompt else {
        return;
    };
    let agent = app.agents.get(prompt.agent_index);
    let label = agent.map(|agent| agent.label).unwrap_or("Agent");
    let command = agent
        .and_then(|agent| agent.install_spec.as_ref())
        .map(|spec| spec.label())
        .unwrap_or_else(|| {
            ui_text(app.language, "manual install only", "需要手动安装").to_string()
        });

    frame.render_widget(Clear, area);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "Install ", "安装 "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(label, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(ui_text(
            app.language,
            "Press Enter or Y to install, Esc or N to cancel.",
            "按 Enter 或 Y 安装，Esc 或 N 取消。",
        )),
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "Command: ", "命令: "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(command),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(ui_text(
            app.language,
            " install ",
            " 安装 ",
        ))),
        area,
    );
}

fn tail_to_width(value: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(value) <= max_width {
        return value.to_string();
    }

    let available = max_width.saturating_sub(1);
    let mut chars = Vec::new();
    let mut width = 0usize;
    for ch in value.chars().rev() {
        let ch_width = ch.width().unwrap_or(0);
        if width + ch_width > available {
            break;
        }
        width += ch_width;
        chars.push(ch);
    }
    chars.reverse();

    let mut out = String::from("<");
    out.extend(chars);
    out
}

fn draw_help(frame: &mut Frame, area: Rect, language: Language) {
    frame.render_widget(Clear, area);
    let lines = if language == Language::Zh {
        vec![
            Line::from(Span::styled(
                "Wimx 原生终端",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("普通输入会直接发送到当前 PTY。"),
            Line::from("终端发出 Unicode 字符时，中文输入会原样转发。"),
            Line::from(""),
            Line::from("Ctrl+N  新建面板"),
            Line::from("Ctrl+W  关闭面板"),
            Line::from("Ctrl+J  下一个面板"),
            Line::from("Ctrl+K  上一个面板"),
            Line::from("Ctrl+L  切换网格/堆叠布局"),
            Line::from("Ctrl+R  重启当前面板"),
            Line::from("Ctrl+G  创建分组"),
            Line::from("Ctrl+O  移动面板到下一个分组"),
            Line::from("Ctrl+A  刷新 Agent 检测"),
            Line::from("Ctrl+T  切换语言"),
            Line::from("Alt+1..9 或 Ctrl+1..9  切换分组"),
            Line::from("鼠标拖动面板边框可调整大小"),
            Line::from("点击侧边栏 Agent，输入路径或用文件浏览器选择目录后按 Enter 启动"),
            Line::from("路径弹窗：Up/Down 浏览，Left 返回上级，Right 进入选中目录，Tab 同步输入"),
            Line::from("未安装的 Agent 会先弹出安装确认"),
            Line::from("Ctrl+H  隐藏/显示帮助"),
            Line::from("Ctrl+Q  退出"),
            Line::from(""),
            Line::from("Windows 重复按键事件会通过 KeyEventKind::Press 过滤。"),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                "Wimx raw terminal",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Normal typing is sent directly to the focused PTY."),
            Line::from(
                "Chinese input is forwarded as Unicode chars when your terminal emits them.",
            ),
            Line::from(""),
            Line::from("Ctrl+N  new pane"),
            Line::from("Ctrl+W  close pane"),
            Line::from("Ctrl+J  next pane"),
            Line::from("Ctrl+K  previous pane"),
            Line::from("Ctrl+L  toggle grid/stack"),
            Line::from("Ctrl+R  respawn focused pane"),
            Line::from("Ctrl+G  create group"),
            Line::from("Ctrl+O  move pane to next group"),
            Line::from("Ctrl+A  refresh agent detection"),
            Line::from("Ctrl+T  switch language"),
            Line::from("Alt+1..9 or Ctrl+1..9  switch group"),
            Line::from("Mouse drag pane borders to resize"),
            Line::from(
                "Click an agent in the sidebar, enter a path or browse a directory, then press Enter",
            ),
            Line::from(
                "Path prompt: Up/Down browse, Left parent, Right open selected directory, Tab sync input",
            ),
            Line::from("Missing agents first ask whether to install"),
            Line::from("Ctrl+H  hide/show this help"),
            Line::from("Ctrl+Q  quit"),
            Line::from(""),
            Line::from("Duplicate Windows key events are filtered by KeyEventKind::Press."),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(ui_text(language, " help ", " 帮助 ")),
        ),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.max(1));
    let height = height.min(area.height.max(1));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn pty_smoke_test() -> Result<()> {
    let shell = ShellSpec::from_args_or_env(&[]).unwrap_or_else(ShellSpec::default_for_platform);
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open smoke pty")?;
    let mut child = pair
        .slave
        .spawn_command(shell.command_builder())
        .with_context(|| format!("spawn {}", shell.label()))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .context("clone smoke reader")?;
    let mut writer = pair.master.take_writer().context("take smoke writer")?;
    let (read_tx, read_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if read_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let command = smoke_command(&shell);

    let start = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut answered_cursor_query = false;
    let mut command_sent = false;
    while Instant::now() < deadline {
        match read_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                if !answered_cursor_query && contains_bytes(&output, b"\x1b[6n") {
                    writer.write_all(b"\x1b[1;1R")?;
                    writer.flush()?;
                    answered_cursor_query = true;
                }
                let text = String::from_utf8_lossy(&output);
                if !command_sent
                    && (text.contains('>')
                        || text.contains('$')
                        || start.elapsed() > Duration::from_millis(700))
                {
                    writer.write_all(command.as_bytes())?;
                    writer.flush()?;
                    command_sent = true;
                }
                if text.contains("WIMX_SMOKE") {
                    let _ = child.kill();
                    return Ok(());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !command_sent && start.elapsed() > Duration::from_millis(700) {
                    writer.write_all(command.as_bytes())?;
                    writer.flush()?;
                    command_sent = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    Err(anyhow!(
        "smoke test timed out using {}; output: {}",
        shell.label(),
        String::from_utf8_lossy(&output)
    ))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn smoke_command(shell: &ShellSpec) -> String {
    let program = shell.program.to_ascii_lowercase();
    let marker = "WIMX_SMOKE";
    let chinese = "\u{4e2d}\u{6587}";
    if cfg!(windows) && program.contains("cmd") {
        format!("echo {marker} {chinese}\r\nexit\r\n")
    } else if cfg!(windows) || program.contains("powershell") || program.contains("pwsh") {
        format!("Write-Output \"{marker} {chinese}\"\r\nexit\r\n")
    } else {
        format!("printf '%s\\n' '{marker} {chinese}'\nexit\n")
    }
}
static SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

const HELP_TEXT: &str = r#"wimx - CMUX-inspired raw terminal multiplexer

USAGE:
  wimx
  wimx --shell pwsh.exe
  wimx --smoke-test

ENV:
  WIMX_SHELL       override shell executable
  WIMX_SHELL_ARGS  optional whitespace-separated shell args
  WIMX_LANG        set UI language: en or zh

KEYS:
  Ctrl+N  new pane
  Ctrl+W  close pane
  Ctrl+J  next pane
  Ctrl+K  previous pane
  Ctrl+L  toggle grid/stack
  Ctrl+R  respawn pane
  Ctrl+G  create group
  Ctrl+O  move pane to next group
  Ctrl+A  refresh agent detection
  Ctrl+T  switch language
  Alt+1..9 or Ctrl+1..9  switch group
  Ctrl+H  help
  Ctrl+Q  quit

MOUSE:
  Left click group     switch group
  Left click agent     ask path, then launch installed agent there
  Left click pane      focus pane
  Drag pane border     resize panes
  Wheel up/down        scroll pane history
"#;
