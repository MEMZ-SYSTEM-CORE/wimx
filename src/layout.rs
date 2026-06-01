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
    file_browser: Option<Rect>,
    file_browser_split: Option<u16>,
    group_list: Rect,
    agent_list: Rect,
    pane_list: Rect,
    pane_area: Rect,
}

fn compute_ui_regions(
    area: Rect,
    file_browser_visible: bool,
    file_browser_width: u16,
) -> UiRegions {
    let show_file_browser = file_browser_visible && area.width >= 31 + 50 + MIN_FILE_BROWSER_WIDTH;
    let file_browser_width = file_browser_width
        .clamp(MIN_FILE_BROWSER_WIDTH, MAX_FILE_BROWSER_WIDTH)
        .min(
            area.width
                .saturating_sub(31 + 50)
                .max(MIN_FILE_BROWSER_WIDTH),
        );
    let root = if show_file_browser {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(31),
                Constraint::Min(50),
                Constraint::Length(file_browser_width),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(31), Constraint::Min(50)])
            .split(area)
    };
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
        file_browser: if show_file_browser {
            Some(root[2])
        } else {
            None
        },
        file_browser_split: if show_file_browser {
            Some(root[2].x)
        } else {
            None
        },
        group_list: sidebar_chunks[1],
        agent_list: sidebar_chunks[2],
        pane_list: sidebar_chunks[3],
        pane_area: workspace_chunks[1],
    }
}

fn command_palette_list_area(area: Rect) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);
    chunks[1]
}

fn file_browser_list_area(area: Rect) -> Rect {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(8),
            Constraint::Length(2),
        ])
        .split(area);
    chunks[1]
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
