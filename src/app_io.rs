impl App {
    fn send_text(&mut self, text: &str) -> Result<()> {
        if let Some(prompt) = self.path_prompt.as_mut() {
            let pasted = text.trim_end_matches(['\r', '\n']);
            prompt.input.push_str(pasted);
            return Ok(());
        }
        if self.install_prompt.is_some()
            || self.command_palette.is_some()
            || self.file_browser_focused
        {
            return Ok(());
        }

        self.send_to_input_targets(text.as_bytes())?;
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        for pane in &mut self.panes {
            if let Some(child) = pane.child.as_mut() {
                let _ = child.kill();
            }
        }
    }
}
