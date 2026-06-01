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
const DEFAULT_FILE_BROWSER_WIDTH: u16 = 38;
const MIN_FILE_BROWSER_WIDTH: u16 = 24;
const MAX_FILE_BROWSER_WIDTH: u16 = 64;

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
    if app.install_prompt.is_some() || app.command_palette.is_some() || app.show_help {
        execute!(terminal.backend_mut(), Hide)?;
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

include!("shell.rs");
include!("core.rs");
include!("agents.rs");
include!("browsers.rs");
include!("pty.rs");
include!("input.rs");
include!("app.rs");
include!("app_actions.rs");
include!("app_keys.rs");
include!("app_mouse.rs");
include!("app_io.rs");
include!("render.rs");
include!("layout.rs");
include!("mouse.rs");
include!("overlays.rs");
include!("help.rs");
include!("tests.rs");
