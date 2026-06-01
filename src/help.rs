static SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

const HELP_TEXT: &str = r#"wimx - CMUX-inspired raw terminal multiplexer

USAGE:
  wimx
  wimx --shell pwsh.exe
  wimx --smoke-test

ENV:
  WIMX_SHELL       override shell executable
  WIMX_SHELL_ARGS  optional whitespace-separated shell args
  WIMX_LANG        set UI language: en or zh

KEYS:
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

MOUSE:
  Left click group     switch group
  Left click agent     ask path, then launch installed agent there
  Left click file      select in right file browser
  Right click folder   open folder in right file browser
  Drag file border     resize right file browser
  Left click pane      focus pane
  Drag pane border     resize panes
  Wheel up/down        scroll pane history
"#;
