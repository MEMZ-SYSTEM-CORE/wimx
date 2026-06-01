# Wimx

Wimx is a CMUX-inspired raw terminal multiplexer written in Rust.

It uses:

- `portable-pty` for real PTY-backed shell panes
- `vt100` for terminal screen parsing
- `ratatui` and `crossterm` for the raw TUI
- terminal-query replies for shells that ask for cursor/device status on startup

## Run

```powershell
cargo run
```

On Windows, Wimx prefers `pwsh.exe` when available, then falls back to Windows PowerShell. Override the shell:

```powershell
$env:WIMX_SHELL = "pwsh.exe"
$env:WIMX_SHELL_ARGS = "-NoLogo -NoProfile"
cargo run
```

## Keys

```text
Ctrl+N  new pane
Ctrl+W  close pane
Ctrl+J  next pane
Ctrl+K  previous pane
Ctrl+L  toggle grid/stack
Ctrl+R  respawn pane
Ctrl+G  create group
Ctrl+O  move pane to next group
Ctrl+A  refresh agent detection
Ctrl+B  toggle broadcast input to all panes in current group
Ctrl+P  command palette
Ctrl+F  focus right-side file browser
Ctrl+E  show/hide right-side file browser
Alt+[ / Alt+]  resize file browser width
Ctrl+T  switch language
Alt+1..9 or Ctrl+1..9  switch group
Ctrl+H  help
Ctrl+Q  quit
```

Normal typing is sent directly to the focused PTY. Windows duplicate key events are filtered by `KeyEventKind::Press`.
When broadcast input is enabled with `Ctrl+B`, ordinary typing and paste are sent to every running pane in the active group.
Press `Ctrl+P` to open the command palette, run common pane/group actions, or quickly start an installed Agent.
Press `Ctrl+F` to focus the right-side file browser. Use Up/Down to select, Enter/Right to enter a directory, Left to go to the parent directory, `R` to refresh, and `A` to open agent commands for the selected project directory. Use `Ctrl+E` to hide/show it, `Alt+[` / `Alt+]` to change its width, or drag its left border with the mouse.

Set `WIMX_LANG=zh` to start in Chinese, or press `Ctrl+T` inside Wimx to switch between English and Chinese.

## Mouse

```text
Left click group      switch group
Left click agent      ask project path, then launch installed agent there
Left click file       select in the right file browser
Right click folder    open a folder in the right file browser
Drag file border      resize the right file browser
Left click pane       focus pane
Drag pane border      resize panes
Wheel up/down         scroll pane history
```

Mouse events are also forwarded to the focused PTY when the running program enables xterm mouse mode.
ANSI colors, 256-color indexes, truecolor, bold, dim, italic, underline, inverse video, cursor visibility/style, and Chinese output are rendered from the PTY screen.

## Agents

Wimx detects these commands on startup:

```text
opencode  OpenCode
codex     Codex
claude    Claude Code
hermes    Hermes
openclaw  OpenClaw
```

Installed agents are marked `[ok]` in the sidebar. Click one, enter a project path, and Wimx starts the agent command directly with that path as its working directory. If the right-side file browser is on a project directory, the agent path prompt starts there by default.
Press `Ctrl+A` to refresh detection after installing or removing an agent while Wimx is open.

On Windows, Wimx prefers real launchable entries (`.exe`, `.cmd`, `.bat`, `.com`, `.ps1`) over extensionless npm/pnpm shims. Batch shims are launched through `cmd.exe`, and PowerShell shims are launched through Windows PowerShell.
Clicking an uninstalled agent now asks whether to install it, and the path prompt includes a scrollable directory browser. On Windows, the browser lists drive roots such as `C:\` and `D:\` so project selection can move across drive letters.

## Verify

```powershell
cargo run -- --smoke-test
```

The smoke test starts a real PTY shell, answers terminal cursor queries, runs a command, and checks that output is returned.

## Source Layout

The Rust code is split by responsibility under `src/`: shell and agent detection, browser state, app actions, keyboard handling, mouse handling, rendering, layout, PTY spawning, and tests live in separate files instead of one large `main.rs`.
