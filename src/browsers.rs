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
    Drive,
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

        entries.extend(
            windows_drive_roots()
                .into_iter()
                .map(|path| PathBrowserEntry {
                    label: format!("[V] {}", path.display()),
                    path,
                    kind: PathBrowserEntryKind::Drive,
                }),
        );

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

struct FileBrowser {
    cwd: PathBuf,
    entries: Vec<FileBrowserEntry>,
    selected: usize,
}

struct FileBrowserEntry {
    label: String,
    name: String,
    path: PathBuf,
    kind: FileBrowserEntryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileBrowserEntryKind {
    Parent,
    Drive,
    Directory,
    File,
}

impl FileBrowser {
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
        if !self.cwd.is_dir() {
            self.cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
        self.cwd = normalize_command_cwd(self.cwd.clone());

        let mut entries = Vec::new();
        if let Some(parent) = self.cwd.parent() {
            entries.push(FileBrowserEntry {
                label: format!("[..] {}", parent.display()),
                name: "..".to_string(),
                path: parent.to_path_buf(),
                kind: FileBrowserEntryKind::Parent,
            });
        }

        entries.extend(
            windows_drive_roots()
                .into_iter()
                .map(|path| FileBrowserEntry {
                    label: format!("[V] {}", path.display()),
                    name: path.display().to_string(),
                    path,
                    kind: FileBrowserEntryKind::Drive,
                }),
        );

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&self.cwd) {
            for entry in read_dir.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().trim().to_string();
                if name.is_empty() {
                    continue;
                }
                let sort_name = name.to_ascii_lowercase();
                if file_type.is_dir() {
                    dirs.push((sort_name, path, name));
                } else if file_type.is_file() {
                    files.push((sort_name, path, name));
                }
            }
        }
        dirs.sort_by(|a, b| a.0.cmp(&b.0));
        files.sort_by(|a, b| a.0.cmp(&b.0));

        entries.extend(dirs.into_iter().map(|(_, path, name)| FileBrowserEntry {
            label: format!("[D] {name}"),
            name,
            path,
            kind: FileBrowserEntryKind::Directory,
        }));
        entries.extend(files.into_iter().map(|(_, path, name)| FileBrowserEntry {
            label: format!("[F] {name}"),
            name,
            path,
            kind: FileBrowserEntryKind::File,
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
        self.cwd = normalize_command_cwd(cwd);
        self.selected = 0;
        self.refresh();
    }

    fn open_selected(&mut self) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        let kind = entry.kind;
        let path = entry.path.clone();
        if matches!(
            kind,
            FileBrowserEntryKind::Parent
                | FileBrowserEntryKind::Drive
                | FileBrowserEntryKind::Directory
        ) {
            self.set_cwd(path);
            true
        } else {
            false
        }
    }

    fn parent(&mut self) -> bool {
        let Some(parent) = self.cwd.parent() else {
            return false;
        };
        self.set_cwd(parent.to_path_buf());
        true
    }

    fn selected_entry(&self) -> Option<&FileBrowserEntry> {
        self.entries.get(self.selected)
    }

    fn agent_cwd(&self) -> PathBuf {
        self.selected_entry()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    FileBrowserEntryKind::Parent
                        | FileBrowserEntryKind::Drive
                        | FileBrowserEntryKind::Directory
                )
            })
            .map(|entry| entry.path.clone())
            .unwrap_or_else(|| self.cwd.clone())
    }
}

fn windows_drive_roots() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }

    (b'C'..=b'Z')
        .filter_map(|letter| {
            let root = format!("{}:\\", char::from(letter));
            Path::new(&root).is_dir().then(|| PathBuf::from(root))
        })
        .collect()
}

fn browser_visible_start(browser: &PathBrowser, visible_rows: usize) -> usize {
    if visible_rows == 0 || browser.entries.len() <= visible_rows {
        return 0;
    }
    browser
        .selected
        .saturating_sub(visible_rows / 2)
        .min(browser.entries.len() - visible_rows)
}

fn palette_visible_start(selected: usize, command_count: usize, visible_rows: usize) -> usize {
    if visible_rows == 0 || command_count <= visible_rows {
        return 0;
    }
    selected
        .saturating_sub(visible_rows / 2)
        .min(command_count - visible_rows)
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

struct CommandPalette {
    selected: usize,
}

#[derive(Clone, Copy)]
enum PaletteAction {
    NewPane,
    ClosePane,
    NextPane,
    PreviousPane,
    NewGroup,
    MovePaneToNextGroup,
    ToggleLayout,
    ToggleBroadcast,
    RefreshAgents,
    RespawnPane,
    SwitchLanguage,
    ToggleHelp,
    FocusFileBrowser,
    ToggleFileBrowser,
    RefreshFileBrowser,
    Agent(usize),
}

struct PaletteCommand {
    label: String,
    detail: String,
    action: PaletteAction,
}
