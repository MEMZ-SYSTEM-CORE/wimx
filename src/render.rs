fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let ui = compute_ui_regions(area, app.file_browser_visible, app.file_browser_width);

    draw_sidebar(frame, app, ui.sidebar);
    draw_workspace(frame, app, ui.workspace);
    if let Some(file_browser) = ui.file_browser {
        draw_file_browser(frame, app, file_browser);
    }

    if app.show_help {
        draw_help(frame, centered_rect(58, 42, area), app.language);
    }
    if app.command_palette.is_some() {
        draw_command_palette(frame, app, centered_fixed_rect(78, 18, area));
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

    let broadcast = if app.broadcast_input {
        ui_text(app.language, "[broadcast] ", "[编队] ")
    } else {
        ""
    };
    let title = app
        .panes
        .get(app.active)
        .map(|pane| {
            if pane.scrollback() > 0 {
                format!(
                    "{}{}-{} / {}  [{} {}]  |  Ctrl+H {}  Ctrl+T {}  Ctrl+Q {}",
                    broadcast,
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
                    "{}{}-{} / {}  |  Ctrl+H {}  Ctrl+T {}  Ctrl+Q {}",
                    broadcast,
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
        "{} {}   {}   Ctrl+P {}  Ctrl+F {}  Ctrl+B {}  Ctrl+G {}  Ctrl+O {}  Alt+1..9 {}",
        SPINNER[app.spinner],
        ui_text(app.language, "direct PTY input", "直接 PTY 输入"),
        ui_text(app.language, "drag borders resize", "拖动边框调整大小"),
        ui_text(app.language, "commands", "命令"),
        ui_text(app.language, "files", "文件"),
        ui_text(app.language, "broadcast", "编队"),
        ui_text(app.language, "group", "分组"),
        ui_text(app.language, "move", "移动"),
        ui_text(app.language, "switch", "切换")
    );
    frame.render_widget(Paragraph::new(hint), chunks[2]);
}

fn draw_file_browser(frame: &mut Frame, app: &App, area: Rect) {
    let browser = &app.file_browser;
    let border_style = if app.file_browser_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(8),
            Constraint::Length(2),
        ])
        .split(area);

    let path_width = usize::from(chunks[0].width.saturating_sub(4));
    let cwd = tail_to_width(&browser.cwd.display().to_string(), path_width);
    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            ui_text(app.language, "Agent project files", "Agent 项目文件"),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(cwd, Style::default().fg(Color::DarkGray))),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(ui_text(app.language, " files ", " 文件 ")),
    );
    frame.render_widget(header, chunks[0]);

    let selected = browser
        .selected
        .min(browser.entries.len().saturating_sub(1));
    let visible_rows = usize::from(chunks[1].height.saturating_sub(2)).max(1);
    let visible_start = palette_visible_start(selected, browser.entries.len(), visible_rows);
    let items: Vec<ListItem> = browser
        .entries
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_rows)
        .map(|(index, entry)| {
            let is_selected = index == selected;
            let base_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(if app.file_browser_focused {
                        Color::Cyan
                    } else {
                        Color::DarkGray
                    })
                    .add_modifier(Modifier::BOLD)
            } else {
                match entry.kind {
                    FileBrowserEntryKind::Parent => Style::default().fg(Color::Yellow),
                    FileBrowserEntryKind::Drive => Style::default().fg(Color::Magenta),
                    FileBrowserEntryKind::Directory => Style::default().fg(Color::Cyan),
                    FileBrowserEntryKind::File => Style::default(),
                }
            };
            let marker = if is_selected { ">" } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(marker, base_style),
                Span::raw(" "),
                Span::styled(entry.label.clone(), base_style),
            ]))
        })
        .collect();
    let list_title = format!(
        " {} {}/{} ",
        ui_text(app.language, "yazi", "浏览"),
        selected.saturating_add(1).min(browser.entries.len().max(1)),
        browser.entries.len().max(1)
    );
    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(list_title),
        ),
        chunks[1],
    );

    let preview = Paragraph::new(file_browser_preview_lines(
        browser,
        app.language,
        chunks[2].width,
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(ui_text(app.language, " preview ", " 预览 ")),
    );
    frame.render_widget(preview, chunks[2]);

    let footer = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Ctrl+F", Style::default().fg(Color::Yellow)),
            Span::raw(ui_text(app.language, " focus  ", " 聚焦  ")),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::raw(ui_text(app.language, " open  A agents", " 打开  A Agent")),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+E", Style::default().fg(Color::Yellow)),
            Span::raw(ui_text(app.language, " hide  ", " 隐藏  ")),
            Span::styled("Alt+[ ]", Style::default().fg(Color::Yellow)),
            Span::raw(ui_text(app.language, " width", " 宽度")),
        ]),
    ]);
    frame.render_widget(footer, chunks[3]);
}

fn file_browser_preview_lines(
    browser: &FileBrowser,
    language: Language,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(entry) = browser.selected_entry() else {
        return vec![Line::from(ui_text(language, "empty", "空"))];
    };
    let text_width = usize::from(width.saturating_sub(4)).max(8);
    let kind = match entry.kind {
        FileBrowserEntryKind::Parent => ui_text(language, "parent", "上级"),
        FileBrowserEntryKind::Drive => ui_text(language, "drive", "盘符"),
        FileBrowserEntryKind::Directory => ui_text(language, "directory", "目录"),
        FileBrowserEntryKind::File => ui_text(language, "file", "文件"),
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                ui_text(language, "type ", "类型 "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(kind),
        ]),
        Line::from(vec![
            Span::styled(
                ui_text(language, "name ", "名称 "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(tail_to_width(&entry.name, text_width)),
        ]),
        Line::from(vec![
            Span::styled(
                ui_text(language, "agent cwd ", "Agent 目录 "),
                Style::default().fg(Color::Green),
            ),
            Span::raw(tail_to_width(
                &browser.agent_cwd().display().to_string(),
                text_width,
            )),
        ]),
    ];

    if let Ok(metadata) = std::fs::metadata(&entry.path) {
        if metadata.is_file() {
            lines.push(Line::from(vec![
                Span::styled(
                    ui_text(language, "size ", "大小 "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(file_size_label(metadata.len())),
            ]));
        }
    }

    if entry.path.is_dir() {
        let mut children = Vec::new();
        if let Ok(read_dir) = std::fs::read_dir(&entry.path) {
            for child in read_dir.flatten().take(4) {
                children.push(child.file_name().to_string_lossy().to_string());
            }
        }
        if !children.is_empty() {
            lines.push(Line::from(Span::styled(
                ui_text(language, "contains", "包含"),
                Style::default().fg(Color::DarkGray),
            )));
            lines.extend(
                children
                    .into_iter()
                    .map(|child| Line::from(format!("  {}", tail_to_width(&child, text_width)))),
            );
        }
    }

    lines
}

fn file_size_label(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size = size as f64;
    if size >= GB {
        format!("{:.1} GB", size / GB)
    } else if size >= MB {
        format!("{:.1} MB", size / MB)
    } else if size >= KB {
        format!("{:.1} KB", size / KB)
    } else {
        format!("{} B", size as u64)
    }
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
