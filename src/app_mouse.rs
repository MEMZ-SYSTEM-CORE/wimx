impl App {
    fn handle_install_prompt_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
            return Ok(());
        }

        let (cols, rows) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let area = centered_fixed_rect(64, 9, Rect::new(0, 0, cols.max(1), rows.max(1)));
        if !point_in_rect(area, mouse.column, mouse.row) {
            return Ok(());
        }

        let Some(prompt) = self.install_prompt.take() else {
            return Ok(());
        };
        if mouse.column < area.x + area.width / 2 {
            self.install_agent(prompt.agent_index)?;
        } else {
            self.install_prompt = None;
            self.set_status("install canceled", "已取消安装");
        }
        Ok(())
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
                let row_index = usize::from(mouse.row.saturating_sub(inner.y));
                if let Some(prompt) = self.path_prompt.as_mut() {
                    let visible_rows = usize::from(inner.height).max(1);
                    let visible_start = browser_visible_start(&prompt.browser, visible_rows);
                    let index = visible_start + row_index;
                    if let Some(entry) = prompt.browser.entries.get(index) {
                        let path = entry.path.clone();
                        let kind = entry.kind;
                        prompt.browser.selected = index;
                        prompt.browser_mode = true;
                        match kind {
                            PathBrowserEntryKind::Current => {
                                prompt.input = prompt.browser.cwd.to_string_lossy().to_string();
                            }
                            PathBrowserEntryKind::Parent
                            | PathBrowserEntryKind::Drive
                            | PathBrowserEntryKind::Directory => {
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

    fn handle_command_palette_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
        let (cols, rows) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let area = centered_fixed_rect(78, 18, Rect::new(0, 0, cols.max(1), rows.max(1)));
        let list_area = command_palette_list_area(area);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_palette_selection(-1);
                return Ok(());
            }
            MouseEventKind::ScrollDown => {
                self.move_palette_selection(1);
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Right) => {
                self.command_palette = None;
                self.set_status("command palette closed", "命令面板已关闭");
                return Ok(());
            }
            MouseEventKind::Down(MouseButton::Left) => {}
            _ => return Ok(()),
        }

        if !point_in_rect(area, mouse.column, mouse.row) {
            self.command_palette = None;
            self.set_status("command palette closed", "命令面板已关闭");
            return Ok(());
        }

        let inner = Rect::new(
            list_area.x + 1,
            list_area.y + 1,
            list_area.width.saturating_sub(2),
            list_area.height.saturating_sub(2),
        );
        if !point_in_rect(inner, mouse.column, mouse.row) {
            return Ok(());
        }

        let commands = self.palette_commands();
        if commands.is_empty() {
            return Ok(());
        }
        let selected = self
            .command_palette
            .as_ref()
            .map(|palette| palette.selected)
            .unwrap_or(0)
            .min(commands.len().saturating_sub(1));
        let visible_rows = usize::from(inner.height).max(1);
        let visible_start = palette_visible_start(selected, commands.len(), visible_rows);
        let index = visible_start + usize::from(mouse.row.saturating_sub(inner.y));
        if let Some(command) = commands.get(index) {
            if let Some(palette) = self.command_palette.as_mut() {
                palette.selected = index;
            }
            let action = command.action;
            self.command_palette = None;
            self.run_palette_action(action)?;
        }
        Ok(())
    }

    fn handle_file_browser_mouse(&mut self, mouse: MouseEvent, area: Rect) -> Result<bool> {
        if !point_in_rect(area, mouse.column, mouse.row) {
            return Ok(false);
        }

        self.file_browser_focused = true;
        let list_area = file_browser_list_area(area);
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.file_browser.move_selection(-1);
                return Ok(true);
            }
            MouseEventKind::ScrollDown => {
                self.file_browser.move_selection(1);
                return Ok(true);
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Down(MouseButton::Right) => {}
            _ => return Ok(true),
        }

        let inner = Rect::new(
            list_area.x + 1,
            list_area.y + 1,
            list_area.width.saturating_sub(2),
            list_area.height.saturating_sub(2),
        );
        if !point_in_rect(inner, mouse.column, mouse.row) {
            return Ok(true);
        }

        let selected = self
            .file_browser
            .selected
            .min(self.file_browser.entries.len().saturating_sub(1));
        let visible_rows = usize::from(inner.height).max(1);
        let visible_start =
            palette_visible_start(selected, self.file_browser.entries.len(), visible_rows);
        let index = visible_start + usize::from(mouse.row.saturating_sub(inner.y));
        if index < self.file_browser.entries.len() {
            self.file_browser.selected = index;
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
                if self.file_browser.open_selected() {
                    self.status = format!(
                        "{} {}",
                        ui_text(self.language, "folder", "目录"),
                        self.file_browser.cwd.display()
                    );
                }
            }
        }
        Ok(true)
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
            return self.handle_install_prompt_mouse(mouse);
        }
        if self.path_prompt.is_some() {
            return self.handle_path_prompt_mouse(mouse);
        }
        if self.command_palette.is_some() {
            return self.handle_command_palette_mouse(mouse);
        }

        let (cols, rows) = crossterm_terminal::size().unwrap_or((DEFAULT_COLS, DEFAULT_ROWS));
        let area = Rect::new(0, 0, cols.max(1), rows.max(1));
        let ui = compute_ui_regions(area, self.file_browser_visible, self.file_browser_width);

        match mouse.kind {
            MouseEventKind::Drag(MouseButton::Left) if self.file_browser_resize_drag.is_some() => {
                self.apply_file_browser_resize_drag(mouse, cols);
                return Ok(());
            }
            MouseEventKind::Up(MouseButton::Left) if self.file_browser_resize_drag.is_some() => {
                self.file_browser_resize_drag = None;
                self.set_status("file browser resize done", "文件浏览器宽度已调整");
                return Ok(());
            }
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
            if let Some(split) = ui.file_browser_split {
                if is_near(mouse.column, split) {
                    self.file_browser_resize_drag = Some(FileBrowserResizeDrag {
                        start_column: mouse.column,
                        start_width: self.file_browser_width,
                    });
                    self.set_status("resize file browser", "拖动调整文件浏览器宽度");
                    return Ok(());
                }
            }
        }

        if let Some(file_browser) = ui.file_browser {
            if self.handle_file_browser_mouse(mouse, file_browser)? {
                return Ok(());
            } else if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                self.file_browser_focused = false;
            }
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
}
