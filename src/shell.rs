#[derive(Clone)]
struct ShellSpec {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl ShellSpec {
    fn from_args_or_env(args: &[String]) -> Option<Self> {
        if let Some(index) = args.iter().position(|arg| arg == "--shell") {
            if let Some(program) = args.get(index + 1) {
                return Some(Self {
                    program: normalize_shell_program(program),
                    args: Vec::new(),
                    cwd: None,
                });
            }
        }

        std::env::var("WIMX_SHELL").ok().map(|program| Self {
            program: normalize_shell_program(&program),
            args: std::env::var("WIMX_SHELL_ARGS")
                .ok()
                .map(|args| args.split_whitespace().map(ToOwned::to_owned).collect())
                .unwrap_or_default(),
            cwd: None,
        })
    }

    fn default_for_platform() -> Self {
        if cfg!(windows) {
            if let Some(program) = find_command_path("pwsh.exe") {
                Self {
                    program,
                    args: vec!["-NoLogo".to_string(), "-NoProfile".to_string()],
                    cwd: None,
                }
            } else {
                Self {
                    program: windows_powershell_path()
                        .unwrap_or_else(|| "powershell.exe".to_string()),
                    args: vec![
                        "-NoLogo".to_string(),
                        "-NoProfile".to_string(),
                        "-NoExit".to_string(),
                    ],
                    cwd: None,
                }
            }
        } else {
            Self {
                program: std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()),
                args: Vec::new(),
                cwd: None,
            }
        }
    }

    fn label(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            format!("{} {}", self.program, self.args.join(" "))
        }
    }

    fn command_builder(&self) -> CommandBuilder {
        let mut command = CommandBuilder::new(&self.program);
        for arg in &self.args {
            command.arg(arg);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("FORCE_COLOR", "1");
        command.env("CLICOLOR_FORCE", "1");
        if let Some(cwd) = &self.cwd {
            command.cwd(normalize_command_cwd(cwd.clone()));
        } else if let Ok(cwd) = std::env::current_dir() {
            command.cwd(normalize_command_cwd(cwd));
        }
        command
    }
}

fn find_command_path(command: &str) -> Option<String> {
    if cfg!(windows) {
        return Command::new("where.exe")
            .arg(command)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                let candidates: Vec<String> = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(ToOwned::to_owned)
                    .collect();
                select_windows_command_path(candidates)
            });
    }

    Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {}", shell_quote(command)))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn select_windows_command_path(candidates: Vec<String>) -> Option<String> {
    let mut expanded = Vec::new();
    for candidate in candidates {
        expanded.extend(windows_launchable_variants(&candidate));
        expanded.push(candidate);
    }

    expanded
        .iter()
        .filter_map(|path| windows_command_priority(path).map(|priority| (priority, path)))
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, path)| path.clone())
        .or_else(|| expanded.into_iter().next())
}

fn windows_launchable_path(path: &str) -> bool {
    windows_command_priority(path).is_some()
}

fn windows_command_priority(path: &str) -> Option<u8> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".exe") {
        Some(0)
    } else if lower.ends_with(".com") {
        Some(1)
    } else if lower.ends_with(".cmd") {
        Some(2)
    } else if lower.ends_with(".bat") {
        Some(3)
    } else if lower.ends_with(".ps1") {
        Some(4)
    } else {
        None
    }
}

fn windows_launchable_variants(path: &str) -> Vec<String> {
    if windows_launchable_path(path) {
        return Vec::new();
    }

    [".exe", ".com", ".cmd", ".bat", ".ps1"]
        .into_iter()
        .map(|extension| format!("{path}{extension}"))
        .filter(|candidate| Path::new(candidate).exists())
        .collect()
}

fn normalize_command_cwd(path: PathBuf) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!("\\\\{rest}"));
    }
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn windows_powershell_path() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| "C:\\Windows".to_string());
    let path = format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    std::path::Path::new(&path).exists().then_some(path)
}

fn windows_cmd_path() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let root = std::env::var("SystemRoot")
        .or_else(|_| std::env::var("WINDIR"))
        .unwrap_or_else(|_| "C:\\Windows".to_string());
    let path = format!("{root}\\System32\\cmd.exe");
    std::path::Path::new(&path).exists().then_some(path)
}

fn normalize_shell_program(program: &str) -> String {
    if !cfg!(windows) {
        return program.to_string();
    }
    let has_path = program.contains('\\') || program.contains('/');
    if has_path || std::path::Path::new(program).exists() {
        return select_windows_command_path(vec![program.to_string()])
            .unwrap_or_else(|| program.to_string());
    }
    find_command_path(program)
        .or_else(|| {
            program
                .eq_ignore_ascii_case("powershell.exe")
                .then(windows_powershell_path)
                .flatten()
        })
        .or_else(|| {
            program
                .eq_ignore_ascii_case("cmd.exe")
                .then(windows_cmd_path)
                .flatten()
        })
        .unwrap_or_else(|| program.to_string())
}
