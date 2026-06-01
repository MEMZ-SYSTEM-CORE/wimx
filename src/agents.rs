#[derive(Clone, Copy)]
struct AgentDefinition {
    label: &'static str,
    commands: &'static [&'static str],
    args: &'static [&'static str],
    install: Option<&'static [&'static str]>,
}

const AGENT_DEFINITIONS: &[AgentDefinition] = &[
    AgentDefinition {
        label: "OpenCode",
        commands: &["opencode", "opencode.cmd", "opencode.exe"],
        args: &[],
        install: Some(&["npm", "install", "-g", "opencode-ai"]),
    },
    AgentDefinition {
        label: "Codex",
        commands: &["codex", "codex.cmd", "codex.exe"],
        args: &[],
        install: Some(&["npm", "install", "-g", "@openai/codex"]),
    },
    AgentDefinition {
        label: "Claude Code",
        commands: &["claude", "claude.cmd", "claude.exe", "claude-code"],
        args: &[],
        install: Some(&["npm", "install", "-g", "@anthropic-ai/claude-code"]),
    },
    AgentDefinition {
        label: "Hermes",
        commands: &["hermes", "hermes.cmd", "hermes.exe", "hermes-agent"],
        args: &[],
        install: None,
    },
    AgentDefinition {
        label: "OpenClaw",
        commands: &["openclaw", "openclaw.cmd", "openclaw.exe", "open-claw"],
        args: &[],
        install: None,
    },
];

fn detect_agents() -> Vec<AgentTool> {
    AGENT_DEFINITIONS
        .iter()
        .copied()
        .map(AgentTool::detected)
        .collect()
}

fn agent_shell_spec(commands: &[&str], extra_args: &[&str]) -> Option<(String, ShellSpec)> {
    for command in commands {
        let Some(program) = find_command_path(command) else {
            continue;
        };
        let args: Vec<String> = extra_args.iter().map(|arg| (*arg).to_string()).collect();
        let spec = command_spec_for_program(&program, args);
        return Some(((*command).to_string(), spec));
    }
    None
}

fn shell_spec_from_parts(parts: &[&str]) -> ShellSpec {
    let program = parts.first().copied().unwrap_or_default();
    let args = parts.iter().skip(1).map(|arg| (*arg).to_string()).collect();
    command_spec_for_program(program, args)
}

fn command_spec_for_program(program: &str, args: Vec<String>) -> ShellSpec {
    if cfg!(windows) && is_windows_batch_shim(program) {
        ShellSpec {
            program: windows_cmd_path().unwrap_or_else(|| "cmd.exe".to_string()),
            args: windows_cmd_script_args(program, &args),
            cwd: None,
        }
    } else if cfg!(windows) && is_windows_powershell_shim(program) {
        let mut shell_args = vec![
            "-NoLogo".to_string(),
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            program.to_string(),
        ];
        shell_args.extend(args);
        ShellSpec {
            program: windows_powershell_path().unwrap_or_else(|| "powershell.exe".to_string()),
            args: shell_args,
            cwd: None,
        }
    } else {
        ShellSpec {
            program: program.to_string(),
            args,
            cwd: None,
        }
    }
}

fn windows_cmd_script_args(program: &str, args: &[String]) -> Vec<String> {
    let mut command = quote_for_cmd(program);
    for arg in args {
        command.push(' ');
        command.push_str(&quote_for_cmd(arg));
    }
    vec![
        "/D".to_string(),
        "/S".to_string(),
        "/C".to_string(),
        command,
    ]
}

fn quote_for_cmd(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '\\' | '/'))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\"\""))
    }
}

fn is_windows_batch_shim(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".cmd") || lower.ends_with(".bat")
}

fn is_windows_powershell_shim(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".ps1")
}

#[derive(Clone)]
struct AgentTool {
    label: &'static str,
    command: String,
    spec: Option<ShellSpec>,
    install_spec: Option<ShellSpec>,
}

impl AgentTool {
    fn detected(definition: AgentDefinition) -> Self {
        let detected = agent_shell_spec(definition.commands, definition.args);
        let install_spec = definition.install.map(shell_spec_from_parts);
        let command = detected
            .as_ref()
            .map(|(command, _)| command.clone())
            .unwrap_or_else(|| definition.commands[0].to_string());
        Self {
            label: definition.label,
            command,
            spec: detected.map(|(_, spec)| spec),
            install_spec,
        }
    }

    fn installed(&self) -> bool {
        self.spec.is_some()
    }
}
