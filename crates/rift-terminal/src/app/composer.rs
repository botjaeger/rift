//! Rich input (`crate::composer`): opening and closing a pane's composer,
//! its keys, sending its text, and closing it when its program ends.

use super::*;
use crate::composer::{self, ComposerKey};

/// The pane's program owns its keys (raw passthrough): `Some` with the
/// running block, if there is one; `None` at the shell prompt (the command
/// editor is there, or a shell widget it handed the command line to is
/// still answering).
fn raw_target(p: &PaneState) -> Option<Option<rift_core::BlockId>> {
    if App::editor_visible(p) || p.session.widgets.lock().unwrap().busy() {
        return None;
    }
    let last = p
        .session
        .blocks
        .lock()
        .unwrap()
        .last()
        .map(|b| (b.id, matches!(b.state, BlockState::Running { .. })));
    Some(match last {
        Some((id, true)) => Some(id),
        _ => None,
    })
}

/// The program the open composer was opened for still owns the pane.
fn still_valid(p: &PaneState) -> bool {
    match raw_target(p) {
        None => false,
        Some(block) => p.composer.block.is_none() || block == p.composer.block,
    }
}

fn output_generation(p: &PaneState) -> u64 {
    p.session
        .generation
        .load(std::sync::atomic::Ordering::Acquire)
}

impl App {
    /// The active pane, when its composer is open.
    pub(super) fn composing_pane(&self) -> Option<PaneId> {
        let id = self.active_pane_id()?;
        self.win
            .tabs
            .get(self.win.active)?
            .pane(id)
            .filter(|p| p.composer.open)
            .map(|_| id)
    }

    /// `rich_input`: open the active pane's composer over its program, or
    /// close it again. At a shell prompt it does nothing (the command
    /// editor is already there).
    pub(super) fn toggle_rich_input(&mut self) {
        let Some(id) = self.active_pane_id() else {
            return;
        };
        if self.composing_pane() == Some(id) {
            self.close_composer(id);
        } else {
            self.open_composer(id);
        }
    }

    /// Open pane `id`'s composer (its draft back in place). False when the
    /// pane is at a prompt or gone.
    pub(super) fn open_composer(&mut self, id: PaneId) -> bool {
        let vim = self.config.vim_mode;
        let active = self.active_pane_id() == Some(id);
        let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
            return false;
        };
        if p.composer.open {
            return true;
        }
        let Some(block) = raw_target(p) else {
            return false;
        };
        let command = p
            .session
            .blocks
            .lock()
            .unwrap()
            .last()
            .filter(|b| matches!(b.state, BlockState::Running { .. }))
            .map(|b| b.command.clone());
        let agent = Self::pane_agent(p);
        let job = if agent.is_none() && command.is_none() {
            p.session.foreground_job_name()
        } else {
            None
        };
        p.composer.label =
            composer::label(agent.map(|a| a.name()), command.as_deref(), job.as_deref());
        p.composer.block = block;
        std::mem::swap(&mut p.editor, &mut p.composer.stash);
        p.editor.set_vim_enabled(vim);
        p.composer.open = true;
        p.clear_selection();
        p.selected_block = None;
        p.view_offset = 0;
        log::info!("rich input: open for {:?} in pane {id}", p.composer.label);
        if active {
            self.win.editor_active = true;
            self.win.completion.close();
            self.completion_pending = None;
            self.completion_intent = None;
            self.blink_epoch = Instant::now();
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        true
    }

    /// Close pane `id`'s composer, keeping its draft; auto-open waits for
    /// the program's next output.
    pub(super) fn close_composer(&mut self, id: PaneId) {
        let vim = self.config.vim_mode;
        let active = self.active_pane_id() == Some(id);
        let now = Instant::now();
        let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
            return;
        };
        if !p.composer.open {
            return;
        }
        std::mem::swap(&mut p.editor, &mut p.composer.stash);
        p.editor.set_vim_enabled(vim);
        p.composer.open = false;
        let generation = output_generation(p);
        p.composer.observe(generation, now);
        p.composer.quiet_now(now, p.session.inputs());
        let editor = Self::editor_visible(p);
        if active {
            self.win.editor_active = editor;
            self.win.completion.close();
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Send the composer's text to its own pane's program and close it.
    /// Empty: nothing is sent and it stays open. Its program gone: nothing
    /// is sent, the draft is offered instead.
    pub(super) fn composer_send(&mut self, id: PaneId) {
        let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
            return;
        };
        if !p.composer.open {
            return;
        }
        if !still_valid(p) {
            self.end_composer(id, false);
            return;
        }
        let Some(bytes) = composer::send_bytes(&p.editor.buffer, bracketed_paste_on(p)) else {
            return;
        };
        p.editor.submit();
        p.view_offset = 0;
        self.close_composer(id);
        if let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) {
            p.prepare_program_grid();
            p.session.write(&bytes);
        }
    }

    /// The composer's program ended (its block finished) or its pane is
    /// closing: close it without sending, and offer a non-empty draft in a
    /// toast with Copy.
    pub(super) fn end_composer(&mut self, id: PaneId, pane_closing: bool) {
        let Some(p) = self.find_tab_of_pane(id).and_then(|(_, t)| t.pane_mut(id)) else {
            return;
        };
        let label = p.composer.label.clone();
        let draft = if p.composer.open {
            let draft = p.editor.buffer.clone();
            p.editor.clear();
            self.close_composer(id);
            draft
        } else if pane_closing {
            // a closed draft goes with its pane too
            let draft = p.composer.stash.buffer.clone();
            p.composer.stash.clear();
            draft
        } else {
            return;
        };
        if draft.trim().is_empty() {
            return;
        }
        let msg = if pane_closing {
            format!("Pane closed: your draft for {label} was not sent")
        } else {
            format!("{label} ended: your draft was not sent")
        };
        self.toast_draft(msg, draft);
    }

    /// A key while the active pane's composer is open, before vim and the
    /// editor: the send key, the newline key, Esc. True when handled.
    pub(super) fn composer_on_key(&mut self, event: &input::KeyPress) -> bool {
        if event.released {
            return false;
        }
        let Some(id) = self.composing_pane() else {
            return false;
        };
        let (vim_on, vim_normal) = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(id))
            .map_or((false, false), |p| {
                (
                    p.editor.vim_enabled(),
                    p.editor.vim_mode() == Some(crate::vim::VimMode::Normal),
                )
            });
        let submit = self.config.rich_input_submit;
        match composer::composer_key(&event.logical_key, self.win.modifiers, submit) {
            // vim: Esc leaves Insert first; in Normal it closes
            Some(ComposerKey::Close) if vim_on && !vim_normal => false,
            Some(ComposerKey::Close) if self.win.completion.open => false,
            Some(ComposerKey::Close) => {
                let multi = self
                    .win
                    .tabs
                    .get(self.win.active)
                    .and_then(|t| t.pane(id))
                    .is_some_and(|p| p.editor.has_extra());
                if multi {
                    // several carets: Esc goes back to one first
                    return false;
                }
                self.close_composer(id);
                true
            }
            Some(ComposerKey::Send) => {
                self.composer_send(id);
                true
            }
            Some(ComposerKey::Newline) => {
                self.editor_key_on(id, EditorKey::Insert("\n".into()), false);
                self.blink_epoch = Instant::now();
                true
            }
            None => false,
        }
    }

    /// An editor key in the composer: editing and motions as in the command
    /// editor; nothing of the shell's (completions, history search, ⌃L,
    /// ⌃D) and a submit sends to the program.
    pub(super) fn composer_editor_key(&mut self, id: PaneId, key: EditorKey) {
        match key {
            EditorKey::Submit => self.composer_send(id),
            EditorKey::Interrupt => self.editor_key_on(id, EditorKey::ClearLine, false),
            EditorKey::Dismiss => {
                let multi = self
                    .find_tab_of_pane(id)
                    .and_then(|(_, t)| t.pane(id))
                    .is_some_and(|p| p.editor.has_extra());
                if multi {
                    self.editor_key_on(id, EditorKey::Dismiss, false);
                } else {
                    self.close_composer(id);
                }
            }
            EditorKey::Complete
            | EditorKey::PrevCompletion
            | EditorKey::OpenCompletions
            | EditorKey::HistorySearch
            | EditorKey::AcceptGhostWord
            | EditorKey::ClearScreen
            | EditorKey::Eof => {}
            k => self.editor_key_on(id, k, false),
        }
        self.blink_epoch = Instant::now();
    }

    /// A coding agent in `pane` said it waits for input (its own
    /// notification): open the composer there (`rich_input_auto`), unless
    /// it was closed and the agent has said nothing since.
    pub(super) fn composer_agent_waiting(&mut self, pane: PaneId) {
        if !self.config.rich_input_auto {
            return;
        }
        let now = Instant::now();
        let Some(p) = self
            .find_tab_of_pane(pane)
            .and_then(|(_, t)| t.pane_mut(pane))
        else {
            return;
        };
        let generation = output_generation(p);
        p.composer.observe(generation, now);
        // asking again lifts an Esc's suppression, typing or not
        p.composer.rearm();
        if p.composer.may_auto_open(p.session.inputs()) {
            self.open_composer(pane);
        }
    }

    /// Each pane of this window: close a composer whose program ended
    /// (offering the draft), and with `rich_input_auto` open one over a
    /// coding agent quiet for [`composer::AUTO_IDLE`] after output. Returns
    /// when a quiet agent is next due.
    pub(super) fn tick_composers(&mut self, now: Instant) -> Option<Instant> {
        use rift_core::notifications::{Agent, agent_title_working};
        let auto = self.config.rich_input_auto;
        let (mut ended, mut waiting, mut due) = (Vec::new(), Vec::new(), None);
        let mut stale = Vec::new();
        for tab in &mut self.win.tabs {
            for p in &mut tab.panes {
                let generation = output_generation(p);
                p.composer.observe(generation, now);
                let id = p.session.id;
                if p.composer.open {
                    if !still_valid(p) {
                        ended.push(id);
                    }
                    continue;
                }
                // a closed draft never carries over to the pane's next
                // program: its own program's end offers it
                if p.composer.block.is_some()
                    && !p.composer.stash.buffer.trim().is_empty()
                    && !still_valid(p)
                {
                    let draft = p.composer.stash.buffer.clone();
                    p.composer.stash.clear();
                    p.composer.block = None;
                    stale.push((p.composer.label.clone(), draft));
                    continue;
                }
                if !auto {
                    continue;
                }
                if agent_title_working(&p.title) {
                    // it works on something: its next wait may open again
                    p.composer.rearm();
                    continue;
                }
                if !p.composer.may_auto_open(p.session.inputs()) {
                    continue;
                }
                // the typed command names a coding agent that still runs (no
                // process lookup: this runs on every wake)
                let agent = p
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .last()
                    .filter(|b| matches!(b.state, BlockState::Running { .. }))
                    .and_then(|b| Agent::from_command(&b.command));
                if agent.is_none() || raw_target(p).is_none() {
                    continue;
                }
                let quiet_at = p.composer.changed_at + composer::AUTO_IDLE;
                if now >= quiet_at {
                    waiting.push(id);
                } else {
                    due = earliest(due, Some(quiet_at));
                }
            }
        }
        for id in ended {
            self.end_composer(id, false);
        }
        for (label, draft) in stale {
            self.toast_draft(format!("{label} ended: your draft was not sent"), draft);
        }
        for id in waiting {
            self.open_composer(id);
        }
        due
    }
}
