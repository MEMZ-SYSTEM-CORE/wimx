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
