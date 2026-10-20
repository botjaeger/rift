//! The subshell offer (`crate::subshell`): a command that starts a subshell
//! Rift's hooks do not reach gets a bar at the bottom of its pane once it
//! has run for [`SHOW_AFTER`]; Enable types the hooks into that shell.

use super::*;
use crate::views::subshell::Part;

/// How long the command runs before the bar shows (its shell has started).
const SHOW_AFTER: Duration = Duration::from_millis(800);

/// A pane's pending offer.
#[derive(Debug, Clone)]
pub struct SubshellOffer {
    pub command: String,
    pub since: Instant,
    /// The bar is up: its command's block runs, out of the alternate
    /// screen (a full-screen program in the subshell hides it).
    pub shown: bool,
    /// The block of the command that started the subshell, once running.
    pub block: Option<rift_core::BlockId>,
    /// The prompt block the command was submitted in: it turns into the
    /// command's block once the shell says the command started (OSC 133;C),
    /// which can come several frames after the submit.
    pub submitted_in: Option<rift_core::BlockId>,
}

/// Where the command that may start a subshell is.
enum Started {
    /// Submitted; the shell has not said it started yet.
    Waiting,
    Running,
    /// It ran and ended (or its block is gone): the offer is over.
    Over,
}

/// The running block of `p`, when its command still runs on the primary
/// screen.
fn running_block(p: &crate::state::PaneState) -> Option<rift_core::BlockId> {
    if p.session
        .term
        .lock()
        .mode()
        .contains(rift_pty::alacritty_terminal::term::TermMode::ALT_SCREEN)
    {
        return None;
    }
    p.session
        .blocks
        .lock()
        .unwrap()
        .last()
        .filter(|b| matches!(b.state, rift_core::BlockState::Running { .. }))
        .map(|b| b.id)
}

impl App {
    /// `command` was just submitted in `pane`: offer the hooks if it starts
    /// a subshell they do not reach.
    pub(super) fn note_subshell(&mut self, pane: PaneId, command: &str) {
        let c = &self.config;
        if !c.subshell_offer
            || !crate::subshell::offers(command, &c.subshell_commands, &c.subshell_denylist)
        {
            return;
        }
        if let Some(p) = self
            .find_tab_of_pane(pane)
            .and_then(|(_, t)| t.pane_mut(pane))
        {
            let submitted_in = p
                .session
                .blocks
                .lock()
                .unwrap()
                .last()
                .filter(|b| matches!(b.state, rift_core::BlockState::Prompt))
                .map(|b| b.id);
            p.subshell = Some(SubshellOffer {
                command: command.trim().to_string(),
                since: Instant::now(),
                shown: false,
                block: None,
                submitted_in,
            });
        }
    }

    /// Show the offers whose time has come, drop the ones whose command is
    /// over (or whose subshell already has hooks: a prompt of its own);
    /// the next time one is due.
    pub(super) fn tick_subshell_offers(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let mut repaint = false;
        let mut due = None;
        for tab in &mut self.win.tabs {
            for p in &mut tab.panes {
                let Some(offer) = &mut p.subshell else {
                    continue;
                };
                let started = {
                    let blocks = p.session.blocks.lock().unwrap();
                    match offer.submitted_in {
                        // its own block: still a prompt until the shell
                        // starts the command, so a slow start never ends it
                        Some(id) => {
                            match blocks.iter().rev().find(|b| b.id == id).map(|b| &b.state) {
                                Some(rift_core::BlockState::Prompt) => Started::Waiting,
                                Some(rift_core::BlockState::Running { .. }) => Started::Running,
                                _ => Started::Over,
                            }
                        }
                        None if blocks.any_running() => Started::Running,
                        None => Started::Over,
                    }
                };
                match started {
                    Started::Over => {
                        repaint |= offer.shown;
                        p.subshell = None;
                        continue;
                    }
                    // output (the start marker) wakes the next frame
                    Started::Waiting => continue,
                    Started::Running => {}
                }
                let at = offer.since + SHOW_AFTER;
                if now < at {
                    due = earliest(due, Some(at));
                    continue;
                }
                let running = running_block(p);
                // the first running block is the subshell's command; a later
                // one (or the alternate screen) hides the bar until it is back
                let offer = p.subshell.as_mut().expect("checked above");
                if offer.block.is_none() {
                    offer.block = running;
                }
                let show = running.is_some() && running == offer.block;
                if show != offer.shown {
                    offer.shown = show;
                    repaint = true;
                }
            }
        }
        (repaint, due)
    }

    /// The bar's Enable, Don't ask again or ✕ in `pane`.
    pub(super) fn answer_subshell(&mut self, pane: PaneId, part: Part) {
        let Some(p) = self
            .find_tab_of_pane(pane)
            .and_then(|(_, t)| t.pane_mut(pane))
        else {
            return;
        };
        let Some(offer) = p.subshell.take() else {
            return;
        };
        // only into the subshell the bar offered it for, never into a
        // program running in it
        if part == Part::Enable && (!offer.shown || running_block(p) != offer.block) {
            p.subshell = Some(offer);
            return;
        }
        match part {
            Part::Enable => {
                p.session
                    .write(crate::subshell::bootstrap(&offer.command).as_bytes());
                // what the shell echoes of the hooks is noise: the command's
                // own block folds; its subshell's commands get blocks of their own
                let running = p
                    .session
                    .blocks
                    .lock()
                    .unwrap()
                    .last()
                    .filter(|b| matches!(b.state, rift_core::BlockState::Running { .. }))
                    .map(|b| b.id);
                if let Some(id) = running {
                    p.session.blocks.lock().unwrap().fold(id);
                }
            }
            Part::Never => {
                if !self.config.subshell_denylist.contains(&offer.command) {
                    self.config.subshell_denylist.push(offer.command);
                    self.save_config();
                }
            }
            Part::Dismiss => {}
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// ⌘⏎ while the active pane shows the bar: Enable.
    pub(super) fn subshell_key(&mut self, event: &input::KeyPress) -> bool {
        let enter = matches!(
            event.logical_key,
            winit::keyboard::Key::Named(winit::keyboard::NamedKey::Enter)
        );
        let m = self.win.modifiers;
        let chord = if cfg!(target_os = "macos") {
            m.super_key() && !m.control_key() && !m.shift_key()
        } else {
            m.control_key() && m.shift_key()
        };
        if !enter || !chord || m.alt_key() {
            return false;
        }
        let Some(id) = self.active_pane_id() else {
            return false;
        };
        let shown = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.pane(id))
            .is_some_and(|p| p.subshell.as_ref().is_some_and(|o| o.shown));
        if shown {
            self.answer_subshell(id, Part::Enable);
        }
        shown
    }
}

/// The Enable button's shortcut, as drawn.
pub(super) fn enable_keys() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘⏎"
    } else {
        "Ctrl+Shift+Enter"
    }
}
