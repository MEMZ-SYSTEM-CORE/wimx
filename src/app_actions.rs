impl App {
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
        let cwd = self.file_browser.agent_cwd();
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
        let cwd = normalize_command_cwd(cwd);
        spec.cwd = Some(cwd.clone());
        self.file_browser.set_cwd(cwd);
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

    fn toggle_layout(&mut self) {
        self.layout = if self.layout == LayoutMode::Grid {
            LayoutMode::Stack
        } else {
            LayoutMode::Grid
        };
        self.reset_layout_weights();
        self.status = format!(
            "{} {}",
            ui_text(self.language, "layout", "布局"),
            if self.layout == LayoutMode::Grid {
                ui_text(self.language, "grid", "网格")
            } else {
                ui_text(self.language, "stack", "堆叠")
            }
        );
    }

    fn toggle_broadcast(&mut self) {
        self.broadcast_input = !self.broadcast_input;
        self.status = ui_text(
            self.language,
            if self.broadcast_input {
                "broadcast input on"
            } else {
                "broadcast input off"
            },
            if self.broadcast_input {
                "编队输入已开启"
            } else {
                "编队输入已关闭"
            },
        )
        .to_string();
    }

    fn toggle_file_browser(&mut self) {
        self.file_browser_visible = !self.file_browser_visible;
        if !self.file_browser_visible {
            self.file_browser_focused = false;
            self.file_browser_resize_drag = None;
            self.set_status("file browser hidden", "文件浏览器已隐藏");
        } else {
            self.set_status("file browser shown", "文件浏览器已显示");
        }
    }

    fn focus_file_browser(&mut self) {
        self.file_browser_visible = true;
        self.file_browser_focused = true;
        self.set_status("file browser focused", "文件浏览器已聚焦");
    }

    fn set_file_browser_width(&mut self, width: u16, total_cols: u16) {
        let available = total_cols.saturating_sub(31 + 50);
        let max_width = available
            .max(MIN_FILE_BROWSER_WIDTH)
            .min(MAX_FILE_BROWSER_WIDTH);
        self.file_browser_width = width.clamp(MIN_FILE_BROWSER_WIDTH, max_width);
    }

    fn adjust_file_browser_width(&mut self, delta: i16) {
        let (cols, _) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let next = if delta.is_negative() {
            self.file_browser_width.saturating_sub(delta.unsigned_abs())
        } else {
            self.file_browser_width.saturating_add(delta as u16)
        };
        self.file_browser_visible = true;
        self.set_file_browser_width(next, cols.max(1));
        self.status = format!(
            "{} {}",
            ui_text(self.language, "file browser width", "文件浏览器宽度"),
            self.file_browser_width
        );
    }

    fn apply_file_browser_resize_drag(&mut self, mouse: MouseEvent, total_cols: u16) {
        let Some(drag) = self.file_browser_resize_drag else {
            return;
        };
        let delta = i32::from(drag.start_column) - i32::from(mouse.column);
        let next = (i32::from(drag.start_width) + delta).clamp(
            i32::from(MIN_FILE_BROWSER_WIDTH),
            i32::from(MAX_FILE_BROWSER_WIDTH),
        ) as u16;
        self.set_file_browser_width(next, total_cols.max(1));
        self.status = format!(
            "{} {}",
            ui_text(self.language, "file browser width", "文件浏览器宽度"),
            self.file_browser_width
        );
    }

    fn palette_commands(&self) -> Vec<PaletteCommand> {
        let mut commands = vec![
            PaletteCommand {
                label: ui_text(self.language, "New pane", "新建面板").to_string(),
                detail: "Ctrl+N".to_string(),
                action: PaletteAction::NewPane,
            },
            PaletteCommand {
                label: ui_text(self.language, "Close focused pane", "关闭当前面板").to_string(),
                detail: "Ctrl+W".to_string(),
                action: PaletteAction::ClosePane,
            },
            PaletteCommand {
                label: ui_text(self.language, "Next pane", "下一个面板").to_string(),
                detail: "Ctrl+J".to_string(),
                action: PaletteAction::NextPane,
            },
            PaletteCommand {
                label: ui_text(self.language, "Previous pane", "上一个面板").to_string(),
                detail: "Ctrl+K".to_string(),
                action: PaletteAction::PreviousPane,
            },
            PaletteCommand {
                label: ui_text(self.language, "New group", "新建分组").to_string(),
                detail: "Ctrl+G".to_string(),
                action: PaletteAction::NewGroup,
            },
            PaletteCommand {
                label: ui_text(
                    self.language,
                    "Move pane to next group",
                    "移动面板到下一分组",
                )
                .to_string(),
                detail: "Ctrl+O".to_string(),
                action: PaletteAction::MovePaneToNextGroup,
            },
            PaletteCommand {
                label: ui_text(self.language, "Toggle layout", "切换布局").to_string(),
                detail: "Ctrl+L".to_string(),
                action: PaletteAction::ToggleLayout,
            },
            PaletteCommand {
                label: ui_text(self.language, "Toggle group broadcast", "切换编队输入").to_string(),
                detail: "Ctrl+B".to_string(),
                action: PaletteAction::ToggleBroadcast,
            },
            PaletteCommand {
                label: ui_text(self.language, "Respawn focused pane", "重启当前面板").to_string(),
                detail: "Ctrl+R".to_string(),
                action: PaletteAction::RespawnPane,
            },
            PaletteCommand {
                label: ui_text(self.language, "Refresh agents", "刷新 Agent 检测").to_string(),
                detail: "Ctrl+A".to_string(),
                action: PaletteAction::RefreshAgents,
            },
            PaletteCommand {
                label: ui_text(self.language, "Switch language", "切换语言").to_string(),
                detail: "Ctrl+T".to_string(),
                action: PaletteAction::SwitchLanguage,
            },
            PaletteCommand {
                label: ui_text(self.language, "Focus file browser", "聚焦文件浏览器").to_string(),
                detail: "Ctrl+F".to_string(),
                action: PaletteAction::FocusFileBrowser,
            },
            PaletteCommand {
                label: ui_text(
                    self.language,
                    "Show/hide file browser",
                    "显示/隐藏文件浏览器",
                )
                .to_string(),
                detail: "Ctrl+E".to_string(),
                action: PaletteAction::ToggleFileBrowser,
            },
            PaletteCommand {
                label: ui_text(self.language, "Refresh file browser", "刷新文件浏览器").to_string(),
                detail: "browser R".to_string(),
                action: PaletteAction::RefreshFileBrowser,
            },
            PaletteCommand {
                label: ui_text(self.language, "Toggle help", "显示/隐藏帮助").to_string(),
                detail: "Ctrl+H".to_string(),
                action: PaletteAction::ToggleHelp,
            },
        ];

        for (index, agent) in self.agents.iter().enumerate() {
            let state = if agent.installed() {
                ui_text(self.language, "installed", "已安装")
            } else {
                ui_text(self.language, "install", "安装")
            };
            commands.push(PaletteCommand {
                label: format!("Agent: {}", agent.label),
                detail: format!("{state} / {}", agent.command),
                action: PaletteAction::Agent(index),
            });
        }

        commands
    }

    fn open_command_palette(&mut self) {
        self.show_help = false;
        self.command_palette = Some(CommandPalette { selected: 0 });
        self.set_status("command palette", "命令面板");
    }

    fn move_palette_selection(&mut self, delta: isize) {
        let len = self.palette_commands().len();
        if len == 0 {
            return;
        }
        if let Some(palette) = self.command_palette.as_mut() {
            let current = palette.selected.min(len - 1);
            let next = if delta.is_negative() {
                current.saturating_sub(delta.unsigned_abs())
            } else {
                (current + delta as usize).min(len - 1)
            };
            palette.selected = next;
        }
    }

    fn run_palette_action(&mut self, action: PaletteAction) -> Result<()> {
        match action {
            PaletteAction::NewPane => self.new_pane()?,
            PaletteAction::ClosePane => self.close_active(),
            PaletteAction::NextPane => self.focus_next(),
            PaletteAction::PreviousPane => self.focus_prev(),
            PaletteAction::NewGroup => self.new_group()?,
            PaletteAction::MovePaneToNextGroup => self.move_active_to_next_group()?,
            PaletteAction::ToggleLayout => self.toggle_layout(),
            PaletteAction::ToggleBroadcast => self.toggle_broadcast(),
            PaletteAction::RefreshAgents => self.refresh_agents(),
            PaletteAction::RespawnPane => self.respawn_active(),
            PaletteAction::SwitchLanguage => self.toggle_language(),
            PaletteAction::ToggleHelp => self.show_help = !self.show_help,
            PaletteAction::FocusFileBrowser => {
                self.focus_file_browser();
            }
            PaletteAction::ToggleFileBrowser => {
                self.toggle_file_browser();
            }
            PaletteAction::RefreshFileBrowser => {
                self.file_browser.refresh();
                self.set_status("file browser refreshed", "文件浏览器已刷新");
            }
            PaletteAction::Agent(index) => self.launch_agent(index)?,
        }
        Ok(())
    }
}
