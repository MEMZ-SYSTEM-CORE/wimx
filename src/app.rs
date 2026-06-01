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

#[derive(Clone, Copy)]
struct FileBrowserResizeDrag {
    start_column: u16,
    start_width: u16,
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
    command_palette: Option<CommandPalette>,
    file_browser: FileBrowser,
    file_browser_focused: bool,
    file_browser_visible: bool,
    file_browser_width: u16,
    file_browser_resize_drag: Option<FileBrowserResizeDrag>,
    broadcast_input: bool,
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
            command_palette: None,
            file_browser: FileBrowser::new(normalize_command_cwd(
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            )),
            file_browser_focused: false,
            file_browser_visible: true,
            file_browser_width: DEFAULT_FILE_BROWSER_WIDTH,
            file_browser_resize_drag: None,
            broadcast_input: false,
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

    fn send_to_input_targets(&mut self, bytes: &[u8]) -> Result<()> {
        if self.broadcast_input {
            for index in self.visible_indices() {
                if let Some(pane) = self.panes.get_mut(index) {
                    if pane.exited {
                        continue;
                    }
                    pane.reset_scrollback();
                    pane.send(bytes)?;
                }
            }
        } else if let Some(pane) = self.active_pane_mut() {
            pane.reset_scrollback();
            pane.send(bytes)?;
        }
        Ok(())
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
}
