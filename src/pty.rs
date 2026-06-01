struct SpawnedPty {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}

fn spawn_pty_for_pane(
    shell: &ShellSpec,
    pane_id: usize,
    tx: Sender<AppEvent>,
) -> Result<SpawnedPty> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;
    let child = pair
        .slave
        .spawn_command(shell.command_builder())
        .with_context(|| format!("spawn {}", shell.label()))?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("clone reader")?;
    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().context("take writer")?,
    ));
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(AppEvent::PaneExited { pane_id });
                    break;
                }
                Ok(n) => {
                    let _ = tx.send(AppEvent::PtyOutput {
                        pane_id,
                        bytes: buf[..n].to_vec(),
                    });
                }
                Err(_) => {
                    let _ = tx.send(AppEvent::PaneExited { pane_id });
                    break;
                }
            }
        }
    });

    Ok(SpawnedPty {
        writer,
        master: pair.master,
        child,
    })
}

fn pty_smoke_test() -> Result<()> {
    let shell = ShellSpec::from_args_or_env(&[]).unwrap_or_else(ShellSpec::default_for_platform);
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open smoke pty")?;
    let mut child = pair
        .slave
        .spawn_command(shell.command_builder())
        .with_context(|| format!("spawn {}", shell.label()))?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .context("clone smoke reader")?;
    let mut writer = pair.master.take_writer().context("take smoke writer")?;
    let (read_tx, read_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if read_tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let command = smoke_command(&shell);

    let start = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut output = Vec::new();
    let mut answered_cursor_query = false;
    let mut command_sent = false;
    while Instant::now() < deadline {
        match read_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                if !answered_cursor_query && contains_bytes(&output, b"\x1b[6n") {
                    writer.write_all(b"\x1b[1;1R")?;
                    writer.flush()?;
                    answered_cursor_query = true;
                }
                let text = String::from_utf8_lossy(&output);
                if !command_sent
                    && (text.contains('>')
                        || text.contains('$')
                        || start.elapsed() > Duration::from_millis(700))
                {
                    writer.write_all(command.as_bytes())?;
                    writer.flush()?;
                    command_sent = true;
                }
                if text.contains("WIMX_SMOKE") {
                    let _ = child.kill();
                    return Ok(());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if !command_sent && start.elapsed() > Duration::from_millis(700) {
                    writer.write_all(command.as_bytes())?;
                    writer.flush()?;
                    command_sent = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = child.kill();
    Err(anyhow!(
        "smoke test timed out using {}; output: {}",
        shell.label(),
        String::from_utf8_lossy(&output)
    ))
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn smoke_command(shell: &ShellSpec) -> String {
    let program = shell.program.to_ascii_lowercase();
    let marker = "WIMX_SMOKE";
    let chinese = "\u{4e2d}\u{6587}";
    if cfg!(windows) && program.contains("cmd") {
        format!("echo {marker} {chinese}\r\nexit\r\n")
    } else if cfg!(windows) || program.contains("powershell") || program.contains("pwsh") {
        format!("Write-Output \"{marker} {chinese}\"\r\nexit\r\n")
    } else {
        format!("printf '%s\\n' '{marker} {chinese}'\nexit\n")
    }
}
