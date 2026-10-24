//! A pane's life beyond its shell: read-only panes (typing, pastes, drops
//! and every other input refused with a toast; the session refuses the
//! bytes too, `Session::set_read_only`), panes kept after their program
//! exits (`close_on_exit`, the Restart bar `views::exited`), and restarting
//! a pane in place: the same launch, pane id and place in the layout.

use super::*;

/// The toast a refused input shows.
pub(super) const READ_ONLY_TOAST: &str = "Pane is read-only";

/// Whether `key` changes a command line (or sends it): a read-only pane's
/// editor refuses it. Moves and selections go through.
pub(super) fn edits_command_line(key: &EditorKey) -> bool {
    !key.only_moves() && !matches!(key, EditorKey::SubwordLeft | EditorKey::SubwordRight)
}

impl App {
    /// Make `pane` read-only, or let it take input again.
    pub(super) fn toggle_read_only(&mut self, pane: PaneId) {
        let Some(p) = self.win.tabs.iter().find_map(|t| t.pane(pane)) else {
            return;
        };
        let on = !p.session.read_only();
        p.session.set_read_only(on);
        if self.active_pane_id() == Some(pane) {
            // Cancel pending acceptance when locking; ask for the preserved
            // draft's completions again when it becomes writable.
            self.update_completions();
        }
        self.toast(
            if on {
                READ_ONLY_TOAST
            } else {
                "Pane accepts input again"
            }
            .into(),
        );
        self.save_session();
    }

    /// Whether `pane` is read-only.
    pub(super) fn pane_read_only(&self, pane: PaneId) -> bool {
        self.win
            .tabs
            .iter()
            .find_map(|t| t.pane(pane))
            .is_some_and(|p| p.session.read_only())
    }

    /// Input meant for `pane` is refused when it is read-only (with the
    /// toast); true then.
    pub(super) fn refuse_read_only(&mut self, pane: PaneId) -> bool {
        let refused = self.pane_read_only(pane);
        if refused {
            self.toast(READ_ONLY_TOAST.into());
        }
        refused
    }

    /// Bytes a read-only session refused since the last frame (a paste, a
    /// drop, a key in raw mode, `ctl send`): one toast for all of them.
    pub(super) fn poll_refused_input(&mut self) {
        let mut refused = false;
        for p in self.win.tabs.iter().flat_map(|t| t.panes.iter()) {
            refused |= p.session.take_refused();
        }
        if refused {
            self.toast(READ_ONLY_TOAST.into());
        }
    }

    /// `pane`'s program ended: the pane closes, or stays with its last
    /// output and the Restart bar (`close_on_exit`). An exit of a session
    /// the pane no longer runs (it was restarted) is ignored.
    pub(super) fn on_pane_exit(&mut self, pane: PaneId, event_loop: &ActiveEventLoop) {
        if !self.enter_pane_window(pane) {
            return;
        }
        let close_on_exit = self.config.close_on_exit;
        let Some(p) = self.win.tabs.iter_mut().find_map(|t| t.pane_mut(pane)) else {
            return;
        };
        let Some(code) = p.session.exit_status() else {
            return;
        };
        if p.session.service_lost() {
            // no exit was seen: closing would hang up a shell that may
            // still run in the service; the bar offers to reconnect
            p.exited = Some(crate::views::exited::LOST);
            p.view_offset = 0;
            return;
        }
        if close_on_exit.closes(code) {
            self.close_pane(pane, event_loop);
            return;
        }
        p.exited = Some(code.unwrap_or(-1));
        p.view_offset = 0;
        // the command that ended the shell (`exit 3`) is done, with its code
        let abs = {
            let term = p.session.term.lock();
            let row = term.grid().cursor.point.line.0.max(0) as u64;
            p.session
                .scrolled
                .load(std::sync::atomic::Ordering::Acquire)
                + row
        };
        p.session.blocks.lock().unwrap().on_done(abs, code);
        // its composer has nothing left to send to
        self.end_composer(pane, true);
    }

    /// Restart `pane` (the action and the Restart bar): asks first while a
    /// job other than the shell runs in it.
    pub(super) fn request_restart(&mut self, pane: PaneId) {
        let commands = self.running_in(CloseScope::Pane(pane));
        if !commands.is_empty() {
            self.win.confirm = Some(ConfirmClose {
                opened: Instant::now(),
                focus_cancel: false,
                action: ConfirmAction::Restart(pane),
                commands,
            });
            return;
        }
        self.restart_pane(pane);
    }

    /// Start `pane`'s program again in place: its launch (profile, program,
    /// arguments) in its directory, the same pane id, read-only state and
    /// place in the layout; a durable session's in the session service.
    /// A running shell is ended first.
    pub(super) fn restart_pane(&mut self, pane: PaneId) {
        let Some(p) = self.win.tabs.iter().find_map(|t| t.pane(pane)) else {
            return;
        };
        let (size, launch, program, read_only) = (
            p.session.size(),
            p.launch.clone(),
            self.pane_program(p),
            p.session.read_only(),
        );
        let cwd = p
            .session
            .local_cwd()
            .or_else(|| p.last_git_cwd.clone())
            .map(std::path::PathBuf::from);
        // a pane whose service stopped answering reconnects to its shell,
        // which may still be running there
        let reattach = p
            .session
            .service_lost()
            .then(|| p.session.service_id())
            .flatten();
        // closing ends a service-owned shell; a local one ends as its PTY
        // drops with the old session below
        if reattach.is_none() {
            p.session.terminate();
        }
        let (resolved, program) = match self.resolve_launch(&launch) {
            Ok(r) => (r, program),
            Err(e) => {
                let global =
                    rift_core::shell_integration::resolve_shell(self.config.shell.as_deref());
                self.toast_error(format!("{e}; this pane runs {global} instead"));
                (Default::default(), global)
            }
        };
        let cwd = cwd
            .filter(|d| d.is_dir())
            .or_else(|| resolved.cwd.clone().filter(|d| d.is_dir()))
            .or_else(|| self.config.start_dir(None));
        let service = match reattach {
            Some(id) => Some(rift_pty::ServiceTarget::Attach(id)),
            None => (self.config.durable_sessions && self.durable_allowed)
                .then_some(rift_pty::ServiceTarget::Spawn),
        };
        let session = self.start_session(pane, size, cwd, service, None, (&program, &resolved));
        session.set_read_only(read_only);
        let vim = self.config.vim_mode;
        let Some(p) = self.win.tabs.iter_mut().find_map(|t| t.pane_mut(pane)) else {
            return;
        };
        let mut fresh = PaneState::new(session);
        fresh.editor.set_vim_enabled(vim);
        fresh.editor.word_delimiters = self.config.word_delimiters.clone();
        fresh.launch = std::mem::take(&mut p.launch);
        fresh.program = std::mem::take(&mut p.program);
        fresh.transcript_id = std::mem::take(&mut p.transcript_id);
        fresh.theme = p.theme.take();
        fresh.text_area = p.text_area;
        fresh.layout_window = p.layout_window;
        fresh.base_grid = p.base_grid;
        fresh.program_grid = p.program_grid;
        fresh.fit = p.fit;
        // the old session (and its PTY) goes with the old state; its grid is
        // freed off the event loop
        let old = std::mem::replace(p, fresh);
        let _grid = super::FreeLater(Some(old.session.term.clone()));
        drop(old);
        // the old session's drop cleared this pane id's query colours
        pane_theme::sync_query_colors(p);
        self.save_session();
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Keys on a pane kept after its program exited: Enter restarts it;
    /// typing goes nowhere (app shortcuts still work). True when handled.
    pub(super) fn exited_key(&mut self, event: &input::KeyPress) -> bool {
        let Some(pane) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .filter(|p| p.exited.is_some())
            .map(|p| p.session.id)
        else {
            return false;
        };
        if event.released {
            return true;
        }
        let mods = self.win.modifiers;
        if matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter)
        ) && !mods.super_key()
            && !mods.control_key()
            && !mods.alt_key()
        {
            self.restart_pane(pane);
            return true;
        }
        // shortcuts (⌘W closes it) fall through; plain typing is swallowed
        input::app_action(event, mods, &self.keymap).is_none()
    }
}
