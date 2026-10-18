//! The Markdown viewer's shell code blocks and "Open in viewer".
//!
//! A rendered `sh` / `bash` / `zsh` / `fish` / `console` block carries a Run
//! button. It never runs on its own: the click asks first, showing the
//! command (masked while `mask_secrets` is on), and the command is the
//! block's own text, as the file holds it — never output from a pane. It
//! goes to the tab's terminal pane only at a prompt; a read-only pane
//! refuses it. A block that printed a Markdown file (`cat README.md`)
//! offers to open that file in the viewer.

use super::*;

impl App {
    /// The Run button of rendered code block `block`: ask before running.
    pub(super) fn ask_run_code_block(&mut self, block: usize) {
        let Some(rift_core::markdown::Block::Code { lang, lines }) =
            self.win.viewer.markdown.get(block)
        else {
            return;
        };
        let Some(command) = rift_core::markdown::shell_command(lang, lines) else {
            return;
        };
        let Some(pane) = self.run_target() else {
            self.toast_error("No terminal pane in this tab to run it in".into());
            return;
        };
        if self.refuse_read_only(pane) || !self.code_target_ready(pane) {
            return;
        }
        let masker = rift_core::secrets::active();
        let shown = command
            .lines()
            .map(|l| crate::block_export::masked(l.to_string(), masker.as_deref()))
            .collect();
        self.win.confirm = Some(ConfirmClose {
            opened: Instant::now(),
            // Enter alone does not run it
            focus_cancel: true,
            action: ConfirmAction::RunCode { pane, command },
            commands: shown,
        });
    }

    /// Confirmed: the command goes to `pane` as a submitted line (checked
    /// again: the pane may have closed, gone read-only or started a job).
    pub(super) fn run_code_block(&mut self, pane: PaneId, command: String) {
        if !self.win.tabs.iter().any(|t| t.pane(pane).is_some()) {
            self.toast_error("The pane closed: nothing ran".into());
            return;
        }
        if self.refuse_read_only(pane) || !self.code_target_ready(pane) {
            return;
        }
        self.submit_to(pane, &command);
        self.win.viewer_focus = false;
    }

    /// The terminal pane a code block runs in: the tab's active pane.
    fn run_target(&self) -> Option<PaneId> {
        let tab = self.win.tabs.get(self.win.active)?;
        tab.pane(tab.active_pane).map(|p| p.session.id)
    }

    /// `pane` takes a command now (its prompt is up); toasts why not.
    fn code_target_ready(&mut self, pane: PaneId) -> bool {
        let ready = self.pane_at_prompt(pane);
        if !ready {
            self.toast(
                "The pane is running a program: run the block once its prompt is back".into(),
            );
        }
        ready
    }

    /// The local Markdown file block `block` of `pane_id` printed
    /// (`cat README.md`), when it exists.
    pub(super) fn printed_markdown(
        &self,
        pane_id: PaneId,
        block: rift_core::BlockId,
    ) -> Option<std::path::PathBuf> {
        let pane = self.win.tabs.get(self.win.active)?.pane(pane_id)?;
        let (command, cwd, remote) = {
            let blocks = pane.session.blocks.lock().unwrap();
            let b = blocks.iter().find(|b| b.id == block)?;
            (b.command.clone(), b.cwd.clone(), b.cwd_remote)
        };
        if remote {
            return None;
        }
        let typed = rift_core::markdown::printed_file(&command)?;
        let path = crate::views::palette::expand_home(&typed);
        let path = if path.is_relative() {
            std::path::Path::new(&cwd?).join(path)
        } else {
            path
        };
        path.is_file().then_some(path)
    }

    /// "Open in viewer" on such a block.
    pub(super) fn open_printed_markdown(&mut self, pane_id: PaneId, block: rift_core::BlockId) {
        match self.printed_markdown(pane_id, block) {
            Some(path) => self.open_file_in(&path, rift_core::editors::RIFT),
            None => self.toast_error("The file it printed is not there any more".into()),
        }
    }
}
