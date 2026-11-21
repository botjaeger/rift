//! The review panel's base picker, commit and push, and "Send to agent…"
//! (from the review panel or the file viewer to another pane). The panel
//! draws in `views/review_panel.rs`; git runs in `rift_core::review` and
//! `rift_core::git_write`; the text sent is built in `views/send_agent.rs`.

use super::*;
use crate::views::palette::{Item, PaletteAction, PaletteMode, Section};
use crate::views::review_panel::{GitBusy, Load};
use crate::views::send_agent::{self, SendPart, SendSource};
use rift_core::review::Base;

/// Longest error text a toast shows (a hook can print pages).
const MAX_TOAST_ERROR: usize = 600;

impl App {
    /// The base chip: list Uncommitted, the default branch and every branch
    /// in the palette.
    pub(super) fn open_review_base_picker(&mut self) {
        let Some(root) = self.win.review.root.clone() else {
            return;
        };
        let default = rift_core::review::default_branch_of(&root);
        let all = rift_core::review::branches(&root).unwrap_or_default();
        self.win.review.branch_list = Some((default, all));
        self.open_palette(PaletteMode::ReviewBase);
    }

    /// The base picker's rows.
    pub(super) fn review_base_items(&self) -> Vec<Item> {
        let row = |title: String, id: &str, section, base, mono| Item {
            title,
            id: id.into(),
            shortcut: Vec::new(),
            section,
            action: PaletteAction::ReviewBase(base),
            mono_title: mono,
        };
        let (default, all) = self.win.review.branch_list.clone().unwrap_or_default();
        let mut items = vec![row(
            "Uncommitted".into(),
            "staged and unstaged changes",
            Section::Base,
            Base::Uncommitted,
            false,
        )];
        if let Some(d) = &default {
            items.push(row(
                format!("Branch: {d}"),
                "committed and uncommitted since the merge base",
                Section::Base,
                Base::DefaultBranch,
                false,
            ));
        }
        for b in all {
            items.push(row(b.clone(), "", Section::Branches, Base::Branch(b), true));
        }
        items
    }

    /// Compare against `base` in the panel's repository (kept for the
    /// session), and reload.
    pub(super) fn choose_review_base(&mut self, base: Base) {
        self.win.review.branch_list = None;
        let Some(root) = self.win.review.root.clone() else {
            return;
        };
        if base == Base::Uncommitted {
            self.review_bases.remove(&root);
        } else {
            self.review_bases.insert(root, base.clone());
        }
        let r = &mut self.win.review;
        r.base = base;
        r.expanded.clear();
        r.revealed.clear();
        r.scroll = 0.0;
        r.notice = None;
        // never a moment of the other base's list under the new header
        r.load = Load::Loading;
        self.request_review();
    }

    /// Commit the staged changes with the panel's message, off the UI
    /// thread.
    pub(super) fn review_commit(&mut self) {
        if self.win.review.busy.is_some() || self.win.review.commit_reason().is_some() {
            return;
        }
        let Some(root) = self.win.review.model().map(|m| m.root.clone()) else {
            return;
        };
        let message = self.win.review.commit_msg.clone();
        self.run_review_git(GitBusy::Commit, move || {
            rift_core::git_write::commit(&root, &message)
        });
    }

    /// Push the current branch, off the UI thread.
    pub(super) fn review_push(&mut self) {
        if self.win.review.busy.is_some() {
            return;
        }
        let Some(root) = self.win.review.model().map(|m| m.root.clone()) else {
            return;
        };
        self.run_review_git(GitBusy::Push, move || rift_core::git_write::push(&root));
    }

    fn run_review_git(
        &mut self,
        kind: GitBusy,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        let proxy = self.proxy.clone();
        let asker = self.win_id();
        let spawned = std::thread::Builder::new()
            .name("review-git".into())
            .spawn(move || {
                let _ = proxy.send_event(UserEvent::ReviewGit(asker, kind, work()));
            });
        match spawned {
            Ok(_) => self.win.review.busy = Some(kind),
            Err(e) => self.toast_error(format!("Could not start git: {e}")),
        }
    }

    /// A commit or push finished: say how, and reload the review.
    pub(super) fn review_git_done(&mut self, kind: GitBusy, result: Result<String, String>) {
        self.win.review.busy = None;
        match result {
            Ok(summary) => {
                if kind == GitBusy::Commit {
                    self.win.review.commit_msg.clear();
                }
                self.toast_done(summary);
            }
            Err(e) => {
                let what = match kind {
                    GitBusy::Commit => "Commit failed",
                    GitBusy::Push => "Push failed",
                };
                let mut e = e;
                if e.len() > MAX_TOAST_ERROR {
                    let cut = (0..=MAX_TOAST_ERROR)
                        .rev()
                        .find(|&i| e.is_char_boundary(i))
                        .unwrap_or(0);
                    e.truncate(cut);
                    e.push('…');
                }
                self.toast_error(format!("{what}: {e}"));
            }
        }
        self.win.explorer.refresh();
        self.request_review();
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Keys while the commit message has focus: typing, Backspace, Enter
    /// (a new line), ⌘⏎ commits, Esc leaves the field. Other chords pass
    /// on (⌘V pastes into the field through `paste_text`).
    pub(super) fn handle_commit_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let mods = self.win.modifiers;
        let r = &mut self.win.review;
        match &event.logical_key {
            Key::Named(NamedKey::Enter) if mods.super_key() || mods.control_key() => {
                if !event.repeat {
                    self.review_commit();
                }
                return true;
            }
            _ if mods.super_key() || mods.control_key() => return false,
            Key::Named(NamedKey::Escape) => r.commit_focus = false,
            Key::Named(NamedKey::Enter) => r.commit_msg.push('\n'),
            Key::Named(NamedKey::Backspace) => {
                crate::state::pop_grapheme(&mut r.commit_msg);
            }
            Key::Named(NamedKey::Tab) => r.commit_msg.push_str("    "),
            _ => {
                let typed: String = event
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if typed.is_empty() {
                    // arrows and the like: the field keeps them
                    return matches!(event.logical_key, Key::Named(_));
                }
                r.commit_msg.push_str(&typed);
            }
        }
        true
    }

    /// "Send to agent…" on a review file or hunk.
    pub(super) fn send_review_to_agent(
        &mut self,
        staged: bool,
        path: &str,
        hunk: Option<rift_core::review::HunkId>,
    ) {
        let (Some(parts), Some(root)) = (
            self.win.review.send_parts(staged, path, hunk),
            self.win.review.model().map(|m| m.root.clone()),
        ) else {
            self.toast("Nothing to send: the change has no lines".into());
            return;
        };
        self.win.send_source = Some(SendSource {
            path: root.join(path),
            parts,
            from_pane: self.active_pane_id(),
        });
        self.pick_send_target();
    }

    /// The viewer's selected lines, else the top line on screen (none on
    /// the rendered Markdown page without a selection).
    pub(super) fn viewer_send_source(&self) -> Option<SendSource> {
        let v = &self.win.viewer;
        let path = v.path.clone()?;
        let (start, end) = v.send_range()?;
        Some(SendSource {
            path,
            parts: vec![SendPart {
                start: start as u32 + 1,
                end: end as u32 + 1,
                body: v.lines[start..=end].join("\n"),
                truncated: false,
            }],
            // the viewer is no pane: no row is marked as the source
            from_pane: None,
        })
    }

    /// Open the target picker for the pending code.
    pub(super) fn pick_send_target(&mut self) {
        if self.win.send_source.is_none() {
            self.toast("Nothing to send".into());
            return;
        }
        self.win.context_menu.close();
        self.open_palette(PaletteMode::SendTarget);
    }

    /// The target picker's rows: panes running a coding agent first, then
    /// every other pane, across windows; each names its window, tab and
    /// pane, with its directory. The pane the code came from says so.
    pub(super) fn send_target_items(&self) -> Vec<Item> {
        let from = self.win.send_source.as_ref().and_then(|s| s.from_pane);
        let (mut agents, mut others) = (Vec::new(), Vec::new());
        for (wi, w) in self.windows_in_order().into_iter().enumerate() {
            for (ti, tab) in w.tabs.iter().enumerate() {
                for (pi, p) in tab.panes.iter().enumerate() {
                    let agent = Self::pane_agent(p);
                    // split panes of one tab differ by their number only
                    let pane_no = if tab.panes.len() > 1 {
                        format!(" · pane {}", pi + 1)
                    } else {
                        String::new()
                    };
                    let name = match agent {
                        Some(a) => a.name().to_string(),
                        None if !p.program_title().is_empty() => p.program_title().to_string(),
                        None => tab.display_title(),
                    };
                    let this = if from == Some(p.session.id) {
                        " (this pane)"
                    } else {
                        ""
                    };
                    let cwd = p
                        .session
                        .cwd
                        .lock()
                        .unwrap()
                        .as_deref()
                        .map(editor::shorten_home)
                        .unwrap_or_default();
                    let item = Item {
                        title: format!(
                            "{name} — window {} · tab {}{pane_no}{this}",
                            wi + 1,
                            ti + 1
                        ),
                        id: cwd,
                        shortcut: Vec::new(),
                        section: if agent.is_some() {
                            Section::Agents
                        } else {
                            Section::Panes
                        },
                        action: PaletteAction::SendToPane(p.session.id),
                        mono_title: false,
                    };
                    if agent.is_some() {
                        agents.push(item);
                    } else {
                        others.push(item);
                    }
                }
            }
        }
        agents.extend(others);
        agents
    }

    /// Paste the pending code into `pane` (any window): bracketed when its
    /// program asked for bracketed paste, never followed by Enter. Without
    /// bracketed paste each line would run, so that asks first. A pane that
    /// closed meanwhile gets nothing.
    pub(super) fn send_to_pane(&mut self, pane: PaneId) {
        let Some(source) = self.win.send_source.take() else {
            return;
        };
        if !self.enter_pane_window(pane) {
            self.toast_error("That pane closed: nothing was sent".into());
            return;
        }
        if self.refuse_read_only(pane) {
            return;
        }
        let Some((_, tab)) = self.find_tab_of_pane(pane) else {
            self.toast_error("That pane closed: nothing was sent".into());
            return;
        };
        let Some(p) = tab.pane_mut(pane) else {
            return;
        };
        let cwd = p.session.local_cwd();
        let text = send_agent::format(&source, cwd.as_deref().map(std::path::Path::new));
        let what = source
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // a shell at Rift's command editor: the text goes into the editor
        // (as a paste would), never raw past it to the shell
        if Self::editor_visible(p) {
            let clean = input::sanitize_editor_paste(&text);
            if p.editor.has_extra() {
                p.editor.paste_at_carets(&clean);
            } else {
                p.editor.insert(&clean);
            }
            p.selected_block = None;
            p.view_offset = 0;
            self.focus_pane_here(pane);
            self.toast_done(format!("Sent {what} to the pane's command line"));
            return;
        }
        let bracketed = bracketed_paste_on(p);
        let label = Self::pane_agent(p)
            .map(|a| a.name().to_string())
            .unwrap_or_else(|| "the pane".into());
        if bracketed {
            p.view_offset = 0;
            p.session.write(&input::bracketed_paste(&text));
        }
        self.focus_pane_here(pane);
        if bracketed {
            self.toast_done(format!("Sent {what} to {label}"));
        } else {
            let commands = text.lines().map(str::to_string).collect();
            self.win.confirm = Some(ConfirmClose {
                opened: Instant::now(),
                focus_cancel: true,
                // the chosen pane only, never its synchronized siblings
                action: ConfirmAction::Paste {
                    pane,
                    text,
                    synced: false,
                },
                commands,
            });
        }
    }

    /// A right-click on a review file header, hunk or line: a menu with
    /// "Send to agent…". False when the pointer is on none.
    pub(super) fn open_review_send_menu(&mut self) -> bool {
        if !self.review_visible() {
            return false;
        }
        let (mx, my) = self.win.mouse;
        let Some(hit) = self
            .win
            .review
            .send_rows
            .iter()
            .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
            .map(|(_, h)| h.clone())
        else {
            return false;
        };
        let ReviewHit::Send(staged, path, hunk) = hit else {
            return false;
        };
        let (Some(parts), Some(root)) = (
            self.win.review.send_parts(staged, &path, hunk),
            self.win.review.model().map(|m| m.root.clone()),
        ) else {
            return false;
        };
        self.win.send_source = Some(SendSource {
            path: root.join(&path),
            parts,
            from_pane: self.active_pane_id(),
        });
        // an unstaged file's header also offers the row's ⋮ actions
        let editable = self.win.review.model().is_some_and(|m| m.base.is_none());
        let mut items = Vec::new();
        if !staged && hunk.is_none() && editable {
            items = Self::review_file_items();
            self.win.review_menu = Some(path);
            items.push(MenuItem::new("Send to agent…", MenuAction::SendToAgent).sep());
        } else {
            items.push(MenuItem::new("Send to agent…", MenuAction::SendToAgent));
        }
        self.win.context_menu = ContextMenuState {
            open: true,
            pane: 0,
            block: 0,
            pos: self.win.mouse,
            items,
            selected: None,
        };
        true
    }

    /// An unstaged review file's actions (its ⋮ and its right-click):
    /// open, stage, and discard, which asks first.
    pub(super) fn review_file_items() -> Vec<MenuItem> {
        vec![
            MenuItem::new("Open file", MenuAction::ReviewOpen),
            MenuItem::new("Stage file", MenuAction::ReviewStage),
            MenuItem::new("Revert changes…", MenuAction::ReviewDiscard).sep(),
        ]
    }
}
