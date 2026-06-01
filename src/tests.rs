#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_windows_cmd_shim_over_extensionless_npm_shim() {
        let selected = select_windows_command_path(vec![
            r"C:\Users\h\AppData\Roaming\npm\codex".to_string(),
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd".to_string(),
        ]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Users\h\AppData\Roaming\npm\codex.cmd")
        );
    }

    #[test]
    fn selects_windows_exe_over_extensionless_windowsapps_alias() {
        let selected = select_windows_command_path(vec![
            r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex".to_string(),
            r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex.exe".to_string(),
        ]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Program Files\WindowsApps\OpenAI.Codex\resources\codex.exe")
        );
    }

    #[test]
    fn selects_windows_powershell_shim_when_it_is_the_only_launchable_path() {
        let selected =
            select_windows_command_path(vec![r"C:\Users\h\AppData\Roaming\npm\tool.ps1".into()]);

        assert_eq!(
            selected.as_deref(),
            Some(r"C:\Users\h\AppData\Roaming\npm\tool.ps1")
        );
    }

    #[test]
    fn builds_cmd_wrapper_for_windows_batch_shim_with_quoted_path() {
        let args = windows_cmd_script_args(
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd",
            &["--color".to_string(), "always".to_string()],
        );

        assert_eq!(args[0], "/D");
        assert_eq!(args[1], "/S");
        assert_eq!(args[2], "/C");
        assert_eq!(
            args[3],
            r"C:\Users\h\AppData\Roaming\npm\codex.cmd --color always"
        );

        let spaced = windows_cmd_script_args(r"C:\Program Files\Agent\agent.cmd", &[]);
        assert_eq!(spaced[3], r#""C:\Program Files\Agent\agent.cmd""#);
    }

    #[test]
    fn tracks_cursor_style_sequences_used_by_node_tuis() {
        let mut parser = vt100::Parser::new_with_callbacks(
            DEFAULT_ROWS,
            DEFAULT_COLS,
            SCROLLBACK,
            TermCallbacks::default(),
        );

        parser.process(b"\x1b[5 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::BlinkingBar
        );

        parser.process(b"\x1b[2 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::SteadyBlock
        );

        parser.process(b"\x1b[0 q");
        assert_eq!(
            parser.callbacks().cursor_style(),
            SetCursorStyle::DefaultUserShape
        );
    }

    #[test]
    fn tracks_cursor_visibility_sequences_used_by_fullscreen_tuis() {
        let mut parser = vt100::Parser::new_with_callbacks(
            DEFAULT_ROWS,
            DEFAULT_COLS,
            SCROLLBACK,
            TermCallbacks::default(),
        );

        parser.process(b"\x1b[?25l");
        assert!(parser.screen().hide_cursor());

        parser.process(b"\x1b[?25h");
        assert!(!parser.screen().hide_cursor());
    }

    #[test]
    fn strips_windows_verbatim_drive_prefix_from_cwd() {
        let path = normalize_command_cwd(PathBuf::from(r"\\?\D:\desktop\code\wimx"));

        assert_eq!(path.to_string_lossy(), r"D:\desktop\code\wimx");
    }

    #[test]
    fn strips_windows_verbatim_unc_prefix_from_cwd() {
        let path = normalize_command_cwd(PathBuf::from(r"\\?\UNC\server\share\repo"));

        assert_eq!(path.to_string_lossy(), r"\\server\share\repo");
    }
}
