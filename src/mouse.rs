fn mouse_to_pty_bytes(mouse: MouseEvent, pane_rect: Rect, pane: &Pane) -> Option<Vec<u8>> {
    if pane.exited || pane_rect.width < 3 || pane_rect.height < 3 {
        return None;
    }
    let inner = Rect::new(
        pane_rect.x + 1,
        pane_rect.y + 1,
        pane_rect.width.saturating_sub(2),
        pane_rect.height.saturating_sub(2),
    );
    if !point_in_rect(inner, mouse.column, mouse.row) {
        return None;
    }

    let screen = pane.parser.screen();
    let mode = screen.mouse_protocol_mode();
    if mode == vt100::MouseProtocolMode::None {
        return None;
    }
    if !mouse_kind_allowed(mode, mouse.kind) {
        return None;
    }

    let (mut code, release_suffix) = mouse_code(mouse.kind)?;
    code += mouse_modifier_bits(mouse.modifiers);

    let x = mouse.column.saturating_sub(inner.x) + 1;
    let y = mouse.row.saturating_sub(inner.y) + 1;

    let bytes = match screen.mouse_protocol_encoding() {
        vt100::MouseProtocolEncoding::Sgr => format!(
            "\x1b[<{};{};{}{}",
            code,
            x,
            y,
            if release_suffix { "m" } else { "M" }
        )
        .into_bytes(),
        vt100::MouseProtocolEncoding::Utf8 => encode_x10_mouse(code, x, y, true)?,
        vt100::MouseProtocolEncoding::Default => encode_x10_mouse(code, x, y, false)?,
    };
    Some(bytes)
}

fn mouse_kind_allowed(mode: vt100::MouseProtocolMode, kind: MouseEventKind) -> bool {
    match mode {
        vt100::MouseProtocolMode::None => false,
        vt100::MouseProtocolMode::Press => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::PressRelease => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::ButtonMotion => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
        vt100::MouseProtocolMode::AnyMotion => matches!(
            kind,
            MouseEventKind::Down(_)
                | MouseEventKind::Up(_)
                | MouseEventKind::Drag(_)
                | MouseEventKind::Moved
                | MouseEventKind::ScrollUp
                | MouseEventKind::ScrollDown
                | MouseEventKind::ScrollLeft
                | MouseEventKind::ScrollRight
        ),
    }
}

fn mouse_code(kind: MouseEventKind) -> Option<(u16, bool)> {
    match kind {
        MouseEventKind::Down(MouseButton::Left) => Some((0, false)),
        MouseEventKind::Down(MouseButton::Middle) => Some((1, false)),
        MouseEventKind::Down(MouseButton::Right) => Some((2, false)),
        MouseEventKind::Up(_) => Some((3, true)),
        MouseEventKind::Drag(MouseButton::Left) => Some((32, false)),
        MouseEventKind::Drag(MouseButton::Middle) => Some((33, false)),
        MouseEventKind::Drag(MouseButton::Right) => Some((34, false)),
        MouseEventKind::Moved => Some((35, false)),
        MouseEventKind::ScrollUp => Some((64, false)),
        MouseEventKind::ScrollDown => Some((65, false)),
        MouseEventKind::ScrollLeft => Some((66, false)),
        MouseEventKind::ScrollRight => Some((67, false)),
    }
}

fn mouse_modifier_bits(modifiers: KeyModifiers) -> u16 {
    let mut bits = 0u16;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits += 16;
    }
    bits
}

fn encode_x10_mouse(code: u16, x: u16, y: u16, utf8_mode: bool) -> Option<Vec<u8>> {
    let mut out = b"\x1b[M".to_vec();
    encode_x10_field(&mut out, code + 32, utf8_mode)?;
    encode_x10_field(&mut out, x + 32, utf8_mode)?;
    encode_x10_field(&mut out, y + 32, utf8_mode)?;
    Some(out)
}

fn encode_x10_field(out: &mut Vec<u8>, value: u16, utf8_mode: bool) -> Option<()> {
    if utf8_mode {
        let ch = char::from_u32(u32::from(value))?;
        out.extend(ch.to_string().into_bytes());
        return Some(());
    }

    let clamped = value.min(255);
    out.push(u8::try_from(clamped).ok()?);
    Some(())
}
