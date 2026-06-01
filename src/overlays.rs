fn draw_command_palette(frame: &mut Frame, app: &App, area: Rect) {
    let Some(palette) = &app.command_palette else {
        return;
    };
    let commands = app.palette_commands();
    let command_count = commands.len();
    let selected = if command_count == 0 {
        0
    } else {
        palette.selected.min(command_count - 1)
    };

    frame.render_widget(Clear, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                ui_text(app.language, "Command palette", "命令面板"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Ctrl+P", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(ui_text(
            app.language,
            "Pick a command or launch an agent.",
            "选择命令，或快速启动 Agent。",
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title(ui_text(
        app.language,
        " command ",
        " 命令 ",
    )));
    frame.render_widget(header, chunks[0]);

    let visible_rows = usize::from(chunks[1].height.saturating_sub(2)).max(1);
    let visible_start = palette_visible_start(selected, command_count, visible_rows);
    let items: Vec<ListItem> = commands
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_rows)
        .map(|(index, command)| {
            let is_selected = index == selected;
            let label_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let detail_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let marker = if is_selected { ">" } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(marker, label_style),
                Span::raw(" "),
                Span::styled(command.label.clone(), label_style),
                Span::styled(format!("  {}", command.detail), detail_style),
            ]))
        })
        .collect();
    let title = format!(
        " {} {}/{} ",
        ui_text(app.language, "commands", "命令"),
        selected.saturating_add(1).min(command_count.max(1)),
        command_count.max(1)
    );
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        chunks[1],
    );

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(ui_text(app.language, " run  ", " 执行  ")),
        Span::styled("Up/Down", Style::default().fg(Color::Yellow)),
        Span::raw(ui_text(app.language, " select  ", " 选择  ")),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(ui_text(app.language, " close", " 关闭")),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
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

    let visible_rows = usize::from(browser_area.height.saturating_sub(2)).max(1);
    let visible_start = browser_visible_start(&prompt.browser, visible_rows);
    let browser_lines: Vec<ListItem> = prompt
        .browser
        .entries
        .iter()
        .enumerate()
        .skip(visible_start)
        .take(visible_rows)
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
    let browser_title = format!(
        " {} {}/{} ",
        ui_text(app.language, "browser", "浏览"),
        prompt.browser.selected.saturating_add(1),
        prompt.browser.entries.len().max(1)
    );
    frame.render_widget(
        List::new(browser_lines).block(Block::default().borders(Borders::ALL).title(browser_title)),
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
            "Up/Down browse, Left parent, Right open selected directory/drive.",
            "上下浏览，Left 返回上级，Right 打开选中目录/盘符。",
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
        Line::from(vec![
            Span::styled(
                ui_text(
                    app.language,
                    "[ left click / Y install ]",
                    "[ 左键左半 / Y 安装 ]",
                ),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                ui_text(
                    app.language,
                    "[ right half / N cancel ]",
                    "[ 右半 / N 取消 ]",
                ),
                Style::default().fg(Color::Yellow),
            ),
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
            Line::from("Ctrl+B  编队输入：把普通输入广播到当前分组所有面板"),
            Line::from("Ctrl+P  打开命令面板"),
            Line::from("Ctrl+F  聚焦右侧文件浏览器"),
            Line::from("Ctrl+E  显示/隐藏右侧文件浏览器"),
            Line::from("Alt+[ / Alt+]  调整文件浏览器宽度"),
            Line::from("Ctrl+T  切换语言"),
            Line::from("Alt+1..9 或 Ctrl+1..9  切换分组"),
            Line::from("鼠标拖动面板边框可调整大小"),
            Line::from("点击侧边栏 Agent，输入路径或用文件浏览器选择目录后按 Enter 启动"),
            Line::from(
                "路径弹窗：Up/Down 浏览，Left 返回上级，Right 进入选中目录/盘符，Tab 同步输入",
            ),
            Line::from(
                "文件浏览器：Up/Down 选择，Enter/Right 进入目录，Left 返回上级，A 打开 Agent 命令面板",
            ),
            Line::from("未安装的 Agent 会先弹出安装确认，左半点击安装，右半点击取消"),
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
            Line::from("Ctrl+B  broadcast normal input to all panes in current group"),
            Line::from("Ctrl+P  command palette"),
            Line::from("Ctrl+F  focus right-side file browser"),
            Line::from("Ctrl+E  show/hide right-side file browser"),
            Line::from("Alt+[ / Alt+]  resize file browser width"),
            Line::from("Ctrl+T  switch language"),
            Line::from("Alt+1..9 or Ctrl+1..9  switch group"),
            Line::from("Mouse drag pane borders to resize"),
            Line::from(
                "Click an agent in the sidebar, enter a path or browse a directory, then press Enter",
            ),
            Line::from(
                "Path prompt: Up/Down browse, Left parent, Right open selected directory/drive, Tab sync input",
            ),
            Line::from(
                "File browser: Up/Down select, Enter/Right open directory, Left parent, A agent commands",
            ),
            Line::from(
                "Missing agents first ask whether to install; click left half to install or right half to cancel",
            ),
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
