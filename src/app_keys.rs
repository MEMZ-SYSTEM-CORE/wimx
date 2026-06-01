impl App {
    fn handle_command_palette_key(&mut self, key: KeyEvent) -> Result<bool> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return Ok(true),
                KeyCode::Char('p') => {
                    self.command_palette = None;
                    self.set_status("command palette closed", "命令面板已关闭");
                    return Ok(false);
                }
                _ => {}
            }
        }

        match key.code {
            KeyCode::Esc => {
                self.command_palette = None;
                self.set_status("command palette closed", "命令面板已关闭");
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.move_palette_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                self.move_palette_selection(1);
            }
            KeyCode::PageUp => {
                self.move_palette_selection(-5);
            }
            KeyCode::PageDown => {
                self.move_palette_selection(5);
            }
            KeyCode::Home => {
                if let Some(palette) = self.command_palette.as_mut() {
                    palette.selected = 0;
                }
            }
            KeyCode::End => {
                let len = self.palette_commands().len();
                if let Some(palette) = self.command_palette.as_mut() {
                    palette.selected = len.saturating_sub(1);
                }
            }
            KeyCode::Enter => {
                let commands = self.palette_commands();
                let selected = self
                    .command_palette
                    .as_ref()
                    .map(|palette| palette.selected)
                    .unwrap_or(0)
                    .min(commands.len().saturating_sub(1));
                let action = commands.get(selected).map(|command| command.action);
                self.command_palette = None;
                if let Some(action) = action {
                    self.run_palette_action(action)?;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_file_browser_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.file_browser_focused = false;
                self.set_status("file browser blurred", "文件浏览器已取消聚焦");
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('K') => {
                self.file_browser.move_selection(-1);
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('J') => {
                self.file_browser.move_selection(1);
            }
            KeyCode::PageUp => {
                self.file_browser.move_selection(-8);
            }
            KeyCode::PageDown => {
                self.file_browser.move_selection(8);
            }
            KeyCode::Home => {
                self.file_browser.selected = 0;
            }
            KeyCode::End => {
                self.file_browser.selected = self.file_browser.entries.len().saturating_sub(1);
            }
            KeyCode::Left | KeyCode::Backspace => {
                if self.file_browser.parent() {
                    self.status = format!(
                        "{} {}",
                        ui_text(self.language, "folder", "目录"),
                        self.file_browser.cwd.display()
                    );
                }
            }
            KeyCode::Enter | KeyCode::Right => {
                if self.file_browser.open_selected() {
                    self.status = format!(
                        "{} {}",
                        ui_text(self.language, "folder", "目录"),
                        self.file_browser.cwd.display()
                    );
                } else if let Some(entry) = self.file_browser.selected_entry() {
                    self.status = format!(
                        "{} {}",
                        ui_text(self.language, "selected", "已选择"),
                        entry.path.display()
                    );
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.file_browser.refresh();
                self.set_status("file browser refreshed", "文件浏览器已刷新");
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                self.open_command_palette();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.install_prompt.is_some() {
            return self.handle_install_prompt_key(key);
        }
        if self.path_prompt.is_some() {
            return self.handle_path_prompt_key(key);
        }
        if self.command_palette.is_some() {
            return self.handle_command_palette_key(key);
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            if let KeyCode::Char(c) = key.code {
                if let Some(group) = digit_group(c) {
                    self.switch_group(group)?;
                    return Ok(false);
                }
                match c {
                    '[' | '{' => {
                        self.adjust_file_browser_width(-4);
                        return Ok(false);
                    }
                    ']' | '}' => {
                        self.adjust_file_browser_width(4);
                        return Ok(false);
                    }
                    _ => {}
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
                    self.toggle_layout();
                    return Ok(false);
                }
                KeyCode::Char('p') => {
                    self.open_command_palette();
                    return Ok(false);
                }
                KeyCode::Char('e') => {
                    self.toggle_file_browser();
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
                KeyCode::Char('f') => {
                    if !self.file_browser_visible {
                        self.focus_file_browser();
                    } else if self.file_browser_focused {
                        self.file_browser_focused = false;
                        self.set_status("file browser blurred", "文件浏览器已取消聚焦");
                    } else {
                        self.focus_file_browser();
                    }
                    return Ok(false);
                }
                KeyCode::Char('a') => {
                    self.refresh_agents();
                    return Ok(false);
                }
                KeyCode::Char('b') => {
                    self.toggle_broadcast();
                    return Ok(false);
                }
                _ => {}
            }
        }

        if self.file_browser_focused {
            return self.handle_file_browser_key(key);
        }

        if let Some(bytes) = key_to_pty_bytes(key) {
            self.send_to_input_targets(&bytes)?;
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
}
