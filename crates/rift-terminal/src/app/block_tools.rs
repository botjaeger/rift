//! Block tools: the multi-block selection (⌘-click toggles a block, ⇧-click
//! selects a range, ⇧↑ / ⇧↓ extend it, ⌘A with a block selected selects
//! every block, Esc clears) and its actions; Copy as HTML / Markdown; Save
//! output / Export buffer (a path typed in the palette); comparing two
//! finished blocks (`views::compare`). Output is untrusted: it is copied,
//! written or diffed as text and never run.

use super::*;
use crate::block_export;
use crate::views::compare;
use crate::views::context_menu::{MenuAction, MenuItem};
use rift_core::BlockId;

/// Why a block has no previous run to compare with, or the run and how
/// long ago it finished.
type PreviousRun = Result<(BlockId, String), &'static str>;

impl App {
    pub(super) fn pane_ref(&self, pane_id: PaneId) -> Option<&PaneState> {
        self.win.tabs.get(self.win.active)?.pane(pane_id)
    }

    pub(super) fn pane_mut_ref(&mut self, pane_id: PaneId) -> Option<&mut PaneState> {
        self.win.tabs.get_mut(self.win.active)?.pane_mut(pane_id)
    }

    /// The blocks an action on `block` covers: the multi-block selection
    /// when `block` is in it, else `block` alone.
    fn block_targets(&self, pane_id: PaneId, block: BlockId) -> Vec<BlockId> {
        let sel = self
            .pane_ref(pane_id)
            .map(|p| p.selected_blocks())
            .unwrap_or_default();
        if sel.len() > 1 && sel.contains(&block) {
            sel
        } else {
            vec![block]
        }
    }

    /// The block menu of a multi-block selection (`block` in it).
    pub(super) fn multi_block_menu(
        &self,
        pane_id: PaneId,
        block: BlockId,
    ) -> Option<Vec<MenuItem>> {
        let targets = self.block_targets(pane_id, block);
        if targets.len() < 2 {
            return None;
        }
        let pane = self.pane_ref(pane_id)?;
        let blocks = pane.session.blocks.lock().unwrap();
        let picked: Vec<&rift_core::Block> =
            blocks.iter().filter(|b| targets.contains(&b.id)).collect();
        let n = picked.len();
        let with_output: Vec<&&rift_core::Block> = picked
            .iter()
            .filter(|b| b.output_range().is_some())
            .collect();
        let all_bookmarked = picked.iter().all(|b| b.bookmarked);
        let all_collapsed = !with_output.is_empty() && with_output.iter().all(|b| b.collapsed);
        let compare_ok = n == 2
            && picked
                .iter()
                .all(|b| matches!(b.state, rift_core::BlockState::Done { .. }));
        Some(vec![
            MenuItem::new(format!("Copy {n} commands"), MenuAction::CopyCommand),
            MenuItem::new(format!("Copy {n} outputs"), MenuAction::CopyOutput)
                .enabled(!with_output.is_empty()),
            MenuItem::new("Copy commands and outputs", MenuAction::CopyBoth),
            MenuItem::new(
                format!("Insert {n} commands into the input"),
                MenuAction::ReinputCommands,
            )
            .enabled(self.win.editor_active),
            MenuItem::new("Compare outputs", MenuAction::CompareSelected)
                .enabled(compare_ok)
                .sep(),
            MenuItem::new(
                if all_bookmarked {
                    "Remove bookmarks"
                } else {
                    "Bookmark all"
                },
                MenuAction::ToggleBookmark,
            )
            .sep(),
            MenuItem::new(
                if all_collapsed {
                    "Expand all"
                } else {
                    "Collapse all"
                },
                MenuAction::ToggleCollapse,
            )
            .enabled(!with_output.is_empty()),
        ])
    }

    /// Rows the single-block menu ends with.
    pub(super) fn block_menu_extras(
        &self,
        pane_id: PaneId,
        block: BlockId,
        has_output: bool,
        has_filter: bool,
    ) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem::new("Copy as HTML", MenuAction::CopyHtml).sep(),
            MenuItem::new("Copy as Markdown", MenuAction::CopyMarkdown),
            MenuItem::new("Save output…", MenuAction::SaveOutput).enabled(has_output),
            match self.previous_run(pane_id, block) {
                Ok((_, age)) => MenuItem::new(
                    format!("Compare with previous run ({age})"),
                    MenuAction::ComparePrevious,
                ),
                Err(why) => MenuItem::new(
                    format!("Compare with previous run ({why})"),
                    MenuAction::ComparePrevious,
                )
                .enabled(false),
            },
        ];
        if has_filter {
            items.push(MenuItem::new(
                "Clear output filter",
                MenuAction::ClearFilter,
            ));
        }
        items
    }

    /// The baseline "Compare with previous run" uses: the newest earlier
    /// finished block of this pane with the same exact command in the same
    /// directory on the same host (a local directory compared resolved, so
    /// `/tmp` and `/private/tmp` match; a remote block needs its host name,
    /// recorded by the remote shell's OSC 7).
    fn previous_run(&self, pane_id: PaneId, block: BlockId) -> PreviousRun {
        let pane = self.pane_ref(pane_id).ok_or("no pane")?;
        let blocks = pane.session.blocks.lock().unwrap();
        let this = blocks.iter().find(|b| b.id == block).ok_or("no block")?;
        if !matches!(this.state, rift_core::BlockState::Done { .. }) {
            return Err("not finished");
        }
        if this.cwd_remote && this.host.is_none() {
            return Err("remote host unknown");
        }
        if this.command.is_empty() || this.cwd.is_none() {
            return Err("none found");
        }
        let want = place(this);
        let prev = blocks
            .iter()
            .rev()
            .filter(|b| b.id < block)
            .find(|b| {
                matches!(b.state, rift_core::BlockState::Done { .. })
                    && !b.background
                    && b.command == this.command
                    && place(b) == want
            })
            .ok_or("none found")?;
        let age = match prev.finished_at {
            Some(t) => crate::views::history::age(t, std::time::SystemTime::now()).to_lowercase(),
            None if prev.restored => "restored".into(),
            None => "earlier".into(),
        };
        Ok((prev.id, age))
    }

    /// Block-menu actions this module owns; false for the rest.
    pub(super) fn run_block_tool(
        &mut self,
        action: MenuAction,
        pane_id: PaneId,
        block: BlockId,
    ) -> bool {
        let targets = self.block_targets(pane_id, block);
        match action {
            MenuAction::CopyCommand
            | MenuAction::CopyOutput
            | MenuAction::CopyBoth
            | MenuAction::ToggleBookmark
            | MenuAction::ToggleCollapse
                if targets.len() > 1 =>
            {
                self.run_on_selection(action, pane_id, &targets)
            }
            MenuAction::ReinputCommands => self.reinput_commands(pane_id, &targets),
            MenuAction::CompareSelected if targets.len() == 2 => {
                self.open_compare(pane_id, targets[0], targets[1], None);
            }
            MenuAction::CompareSelected => {}
            MenuAction::ComparePrevious => match self.previous_run(pane_id, block) {
                Ok((prev, age)) => self.open_compare(
                    pane_id,
                    prev,
                    block,
                    Some(format!(
                        "against the previous run of the same command ({age})"
                    )),
                ),
                Err(why) => self.toast(format!("Compare with previous run: {why}")),
            },
            MenuAction::CopyHtml => self.copy_block_html(pane_id, block),
            MenuAction::CopyMarkdown => self.copy_block_markdown(pane_id, block),
            MenuAction::SaveOutput => {
                let Some(pane) = self.pane_ref(pane_id) else {
                    return true;
                };
                let text = terminal::block_output_text(&pane.session, block).unwrap_or_default();
                let cmd = self.block_command(pane_id, block).unwrap_or_default();
                let name = file_stem(&cmd);
                self.open_save(
                    pane_id,
                    "the output".into(),
                    text,
                    format!("{name}-output.txt"),
                );
            }
            MenuAction::ExportBuffer => {
                let Some(pane) = self.pane_ref(pane_id) else {
                    return true;
                };
                let text = terminal::buffer_text(&pane.session);
                self.open_save(pane_id, "the buffer".into(), text, "rift-buffer.txt".into());
            }
            MenuAction::ClearFilter => {
                if let Some(pane) = self.pane_mut_ref(pane_id) {
                    let editing = pane.filter.editing == Some(block);
                    pane.filter.remove(block);
                    if editing {
                        self.win.filter_editing = false;
                        self.win.search.close();
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// Copy, bookmark or collapse every selected block.
    fn run_on_selection(&mut self, action: MenuAction, pane_id: PaneId, targets: &[BlockId]) {
        let Some(pane) = self.pane_ref(pane_id) else {
            return;
        };
        let snapshot: Vec<rift_core::Block> = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| targets.contains(&b.id))
            .cloned()
            .collect();
        let output =
            |id: BlockId| terminal::block_output_text(&pane.session, id).unwrap_or_default();
        // copies are masked while `mask_secrets` is on (as shown)
        let masker = rift_core::secrets::active();
        let m = masker.as_deref();
        let text = match action {
            MenuAction::CopyCommand => Some(block_export::join_commands(
                &snapshot
                    .iter()
                    .map(|b| b.command.clone())
                    .collect::<Vec<_>>(),
                m,
            )),
            MenuAction::CopyOutput => Some(block_export::join_blocks(
                &snapshot
                    .iter()
                    .filter(|b| b.output_range().is_some())
                    .map(|b| output(b.id))
                    .collect::<Vec<_>>(),
                m,
            )),
            MenuAction::CopyBoth => Some(block_export::join_blocks(
                &snapshot
                    .iter()
                    .map(|b| format!("{}\n{}", b.command, output(b.id)))
                    .collect::<Vec<_>>(),
                m,
            )),
            _ => None,
        };
        if let Some(text) = text {
            self.copy_to_clipboard(text);
            return;
        }
        let mut store = pane.session.blocks.lock().unwrap();
        match action {
            // all on, unless all are on already
            MenuAction::ToggleBookmark => {
                let on = !snapshot.iter().all(|b| b.bookmarked);
                for b in snapshot.iter().filter(|b| b.bookmarked != on) {
                    store.toggle_bookmark(b.id);
                }
            }
            MenuAction::ToggleCollapse => {
                let outputs: Vec<&rift_core::Block> = snapshot
                    .iter()
                    .filter(|b| b.output_range().is_some())
                    .collect();
                let on = !outputs.iter().all(|b| b.collapsed);
                for b in outputs.iter().filter(|b| b.collapsed != on) {
                    store.toggle_collapsed(b.id);
                }
            }
            _ => {}
        }
    }

    /// The selected blocks' commands into the input, one per line (not run).
    fn reinput_commands(&mut self, pane_id: PaneId, targets: &[BlockId]) {
        if !self.win.editor_active || self.refuse_read_only(pane_id) {
            return;
        }
        let Some(pane) = self.pane_mut_ref(pane_id) else {
            return;
        };
        let cmds: Vec<String> = pane
            .session
            .blocks
            .lock()
            .unwrap()
            .iter()
            .filter(|b| targets.contains(&b.id) && !b.command.is_empty())
            .map(|b| b.command.clone())
            .collect();
        pane.editor.clear();
        // into the input to run: the commands as they are
        pane.editor
            .insert(&block_export::join_commands(&cmds, None));
        pane.selected_block = None;
        pane.block_selection.clear();
        self.update_completions();
    }

    fn copy_block_html(&mut self, pane_id: PaneId, block: BlockId) {
        let theme = self.theme.clone();
        let Some(pane) = self.pane_ref(pane_id) else {
            return;
        };
        let pane_theme = terminal::pane_theme(&theme, &pane.session);
        let theme = pane_theme.as_ref().unwrap_or(&theme);
        let command = self.block_command(pane_id, block).unwrap_or_default();
        let (lines, trimmed) =
            terminal::block_output_runs(&pane.session, theme, block).unwrap_or_default();
        let output = terminal::block_output_text(&pane.session, block).unwrap_or_default();
        let hex = |c: rift_core::Color| {
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
        };
        // copies are masked while `mask_secrets` is on (as shown)
        let masker = rift_core::secrets::active();
        let m = masker.as_deref();
        let html = block_export::html(
            &command,
            &lines,
            trimmed,
            (&hex(theme.fg), &hex(theme.bg)),
            m,
        );
        let plain = block_export::plain(&command, &output, m);
        self.copy_rich_to_clipboard(plain, Some(html));
    }

    fn copy_block_markdown(&mut self, pane_id: PaneId, block: BlockId) {
        let Some(pane) = self.pane_ref(pane_id) else {
            return;
        };
        let snap = terminal::block_output_snapshot(&pane.session, block, usize::MAX, usize::MAX)
            .unwrap_or_default();
        let command = self.block_command(pane_id, block).unwrap_or_default();
        let md = block_export::markdown(
            &command,
            &snap.text,
            snap.trimmed_rows,
            snap.truncated,
            rift_core::secrets::active().as_deref(),
        );
        self.copy_to_clipboard(md);
    }

    /// Ask for the path `text` is written to (the palette's text input,
    /// prefilled with `name` in the pane's directory).
    fn open_save(&mut self, pane_id: PaneId, what: String, text: String, name: String) {
        let dir = self
            .pane_ref(pane_id)
            .and_then(|p| p.session.local_cwd())
            .or_else(|| rift_core::sys::home_var().ok())
            .unwrap_or_default();
        let path = std::path::Path::new(&dir).join(name);
        // written masked while `mask_secrets` is on (as shown)
        let text = block_export::masked(text, rift_core::secrets::active().as_deref());
        self.win.save_target = Some((what, text));
        self.win.new_file_dir = None;
        self.open_palette(crate::views::palette::PaletteMode::SavePath);
        self.win.palette.query = crate::views::editor::shorten_home(&path.to_string_lossy());
        self.win.palette.query_selected = true;
        self.refresh_palette();
    }

    /// Enter in the save prompt: write the pending text to the typed path
    /// (`~` = home; a relative path is from the active pane's directory).
    pub(super) fn save_to_typed_path(&mut self) {
        let Some((what, text)) = self.win.save_target.take() else {
            return;
        };
        // the explorer's "New file…": an empty file, never over one
        if let Some(dir) = self.win.new_file_dir.take() {
            return self.create_typed_file(dir);
        }
        let typed = self.win.palette.query.trim().to_string();
        if typed.is_empty() {
            self.restore_save_prompt(what, text);
            self.toast_error("Save: no path given".into());
            return;
        }
        let mut path = crate::views::palette::expand_home(&typed);
        if path.is_relative()
            && let Some(cwd) = self
                .win
                .tabs
                .get(self.win.active)
                .and_then(|t| t.active())
                .and_then(|p| p.session.local_cwd())
        {
            path = std::path::Path::new(&cwd).join(path);
        }
        if path.is_dir() {
            self.restore_save_prompt(what, text);
            self.toast_error(format!("Save: {} is a directory", path.display()));
            return;
        }
        if path.metadata().is_ok_and(|m| !m.is_file()) {
            self.restore_save_prompt(what, text);
            self.toast_error(format!("Save: {} is not a regular file", path.display()));
            return;
        }
        // a file there already: replace it only when asked
        if path.exists() {
            let shown = crate::views::editor::shorten_home(&path.to_string_lossy());
            self.win.confirm = Some(ConfirmClose {
                opened: Instant::now(),
                focus_cancel: false,
                action: ConfirmAction::Overwrite { path, what, text },
                commands: vec![shown],
            });
            return;
        }
        self.write_saved(&path, &what, &text, false);
    }

    /// Keep the captured output and path after an error or overwrite cancel.
    pub(super) fn restore_save_prompt(&mut self, what: String, text: String) {
        self.win.save_target = Some((what, text));
        self.win.palette.mode = crate::views::palette::PaletteMode::SavePath;
        self.win.palette.open = true;
        self.win.palette.query_selected = true;
        self.refresh_palette();
    }

    /// Write a Save output / Export buffer text to `path`.
    pub(super) fn write_saved(
        &mut self,
        path: &std::path::Path,
        what: &str,
        text: &str,
        replace: bool,
    ) {
        match write_regular_file(path, text.as_bytes(), replace) {
            Ok(()) => self.toast(format!(
                "Saved {what} to {}",
                crate::views::editor::shorten_home(&path.to_string_lossy())
            )),
            Err(e) => {
                self.restore_save_prompt(what.to_string(), text.to_string());
                self.toast_error(format!("Could not save to {}: {e}", path.display()));
            }
        }
    }

    /// One compared block: metadata under the blocks lock, then its output
    /// snapshot (bounded) under the term lock.
    fn compare_side(&self, pane_id: PaneId, block: BlockId) -> Option<compare::Side> {
        let pane = self.pane_ref(pane_id)?;
        // shown masked while `mask_secrets` is on: rows, header, a11y status
        let masker = rift_core::secrets::active();
        let m = masker.as_deref();
        let mut side = {
            let blocks = pane.session.blocks.lock().unwrap();
            let b = blocks.iter().find(|b| b.id == block)?;
            let (exit, duration) = match b.state {
                rift_core::BlockState::Done { exit, duration } => (exit, Some(duration)),
                _ => (None, None),
            };
            compare::Side {
                block,
                command: block_export::masked(b.command.clone(), m),
                exit,
                duration,
                cwd: b.cwd.clone(),
                remote: b.cwd_remote,
                restored: b.restored,
                finished: b.finished_at,
                no_output: b.output_range().is_none(),
                ..Default::default()
            }
        };
        side.output = terminal::block_output_snapshot(
            &pane.session,
            block,
            compare::MAX_BYTES,
            compare::MAX_LINES,
        )
        .unwrap_or_default();
        side.output.text = block_export::masked(std::mem::take(&mut side.output.text), m);
        Some(side)
    }

    /// Open the comparison of `a` (before) and `b` (after); the diff runs on
    /// a worker and comes back as [`UserEvent::CompareReady`].
    fn open_compare(&mut self, pane_id: PaneId, a: BlockId, b: BlockId, baseline: Option<String>) {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let started = std::time::Instant::now();
        let (Some(sa), Some(sb)) = (self.compare_side(pane_id, a), self.compare_side(pane_id, b))
        else {
            self.toast("Compare: a block is gone".into());
            return;
        };
        log::info!(
            "compare: snapshot of blocks {a} and {b} ({} + {} bytes) took {:?}",
            sa.output.text.len(),
            sb.output.text.len(),
            started.elapsed()
        );
        let proxy = self.proxy.clone();
        self.win
            .compare
            .start(sa, sb, baseline, move |generation, result| {
                let _ = proxy.send_event(UserEvent::CompareReady(generation, Box::new(result)));
            });
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
    }

    /// Find opening with a block selected searches that block only.
    pub(super) fn scope_find_to_selection(&mut self) {
        let sel = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| Some((p.session.id, p.selected_block?)));
        self.win.search.scope = sel.map(|(pane, id)| {
            let cmd = self.block_command(pane, id).unwrap_or_default();
            (id, cmd)
        });
    }

    /// Widen find to the pane once its block is no longer selected.
    pub(super) fn settle_find_scope(&mut self) {
        let Some((id, _)) = self.win.search.scope else {
            return;
        };
        let selected = self
            .win
            .tabs
            .get(self.win.active)
            .and_then(|t| t.active())
            .and_then(|p| p.selected_block);
        if !self.win.search.open || selected != Some(id) {
            self.win.search.scope = None;
            if self.win.search.open {
                self.refresh_search_live();
            }
        }
    }

    /// A click on the filter bar's chip `i`: Aa, .*, !, then ±N (one more
    /// context line, ⇧ one fewer; past the most it wraps to none).
    pub(super) fn filter_chip(&mut self, i: usize) {
        let shift = self.win.modifiers.shift_key();
        let Some(f) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
            .and_then(|p| p.filter.editing_mut())
        else {
            return;
        };
        match i {
            0 => f.case = !f.case,
            1 => f.regex = !f.regex,
            2 => f.invert = !f.invert,
            3 if shift => f.step_context(false),
            3 if f.context >= crate::views::block_filter::MAX_CONTEXT => f.context = 0,
            3 => f.step_context(true),
            _ => return,
        }
        f.recompile();
    }

    /// Keys while the comparison is open: Esc closes, arrows and pages scroll.
    pub(super) fn handle_compare_key(&mut self, event: &input::KeyPress) {
        use winit::keyboard::{Key, NamedKey};
        let page = self.win.compare.page();
        let c = &mut self.win.compare;
        match event.logical_key {
            Key::Named(NamedKey::Escape | NamedKey::Enter) => c.close(),
            Key::Named(NamedKey::ArrowDown) => c.scroll_by(1),
            Key::Named(NamedKey::ArrowUp) => c.scroll_by(-1),
            Key::Named(NamedKey::PageDown | NamedKey::Space) => c.scroll_by(page),
            Key::Named(NamedKey::PageUp) => c.scroll_by(-page),
            Key::Named(NamedKey::Home) => c.scroll_by(isize::MIN / 2),
            Key::Named(NamedKey::End) => c.scroll_by(isize::MAX / 2),
            _ => {}
        }
    }

    /// ⌘-click (toggle) or ⇧-click (range from the selected block) on
    /// `block`; false without those modifiers.
    pub(super) fn click_block_with_mods(&mut self, pane_id: PaneId, block: BlockId) -> bool {
        let mods = self.win.modifiers;
        let (cmd, shift) = (mods.super_key(), mods.shift_key());
        if cmd == shift || mods.control_key() || mods.alt_key() {
            return false;
        }
        let Some(pane) = self.pane_mut_ref(pane_id) else {
            return false;
        };
        pane.settle_block_selection();
        if cmd {
            let mut set = pane.selected_blocks();
            if let Some(i) = set.iter().position(|b| *b == block) {
                set.remove(i);
                pane.selected_block = set.last().copied();
            } else {
                set.push(block);
                pane.selected_block = Some(block);
            }
            pane.block_selection = if set.len() > 1 { set } else { Vec::new() };
        } else {
            let anchor = pane.selected_block.unwrap_or(block);
            let eligible = selectable_blocks(pane);
            let (lo, hi) = (anchor.min(block), anchor.max(block));
            let range: Vec<BlockId> = eligible
                .into_iter()
                .filter(|b| *b >= lo && *b <= hi)
                .collect();
            pane.selected_block = Some(anchor);
            pane.block_selection = if range.len() > 1 { range } else { Vec::new() };
        }
        true
    }

    /// ⇧↑ / ⇧↓ with a block selected: grow or shrink the selection from it;
    /// ⌘A with a block selected: every block. False for other keys.
    pub(super) fn block_selection_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if event.released {
            return false;
        }
        let mods = self.win.modifiers;
        let Some(pane) = self
            .win
            .tabs
            .get_mut(self.win.active)
            .and_then(|t| t.active_mut())
        else {
            return false;
        };
        let Some(anchor) = pane.selected_block else {
            return false;
        };
        pane.settle_block_selection();
        let eligible = selectable_blocks(pane);
        let select_all = mods.super_key()
            && !mods.shift_key()
            && !mods.alt_key()
            && !mods.control_key()
            && matches!(&event.logical_key, Key::Character(s) if s.eq_ignore_ascii_case("a"));
        if select_all {
            pane.block_selection = eligible;
            if !pane.block_selection.contains(&anchor) {
                pane.block_selection.push(anchor);
            }
            return true;
        }
        let up = match event.logical_key {
            Key::Named(NamedKey::ArrowUp) => true,
            Key::Named(NamedKey::ArrowDown) => false,
            _ => return false,
        };
        if !mods.shift_key() || mods.super_key() || mods.alt_key() || mods.control_key() {
            return false;
        }
        let Some(ai) = eligible.iter().position(|b| *b == anchor) else {
            return false;
        };
        // the end that moves: the one away from the anchor
        let sel = pane.selected_blocks();
        let lo = sel.first().copied().unwrap_or(anchor);
        let hi = sel.last().copied().unwrap_or(anchor);
        let cursor = if lo < anchor { lo } else { hi };
        let ci = eligible.iter().position(|b| *b == cursor).unwrap_or(ai);
        let ni = if up {
            ci.saturating_sub(1)
        } else {
            (ci + 1).min(eligible.len() - 1)
        };
        let (from, to) = (ai.min(ni), ai.max(ni));
        let range = eligible[from..=to].to_vec();
        pane.block_selection = if range.len() > 1 { range } else { Vec::new() };
        let pane_id = pane.session.id;
        let moved = eligible[ni];
        self.jump_to_block(pane_id, moved);
        true
    }
}

/// Open without truncating first: a path can change while Replace is open.
/// NONBLOCK prevents a FIFO substituted for a file from hanging the UI.
fn write_regular_file(path: &std::path::Path, text: &[u8], replace: bool) -> std::io::Result<()> {
    use rift_core::sys::UnixMode;
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).nonblocking_open();
    if replace {
        options.create(true);
    } else {
        options.create_new(true);
    }
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("destination is not a regular file"));
    }
    file.set_len(0)?;
    file.write_all(text)
}

/// Blocks a selection can hold (those shown with a header), oldest first.
fn selectable_blocks(pane: &PaneState) -> Vec<BlockId> {
    pane.session
        .blocks
        .lock()
        .unwrap()
        .iter()
        .filter(|b| b.suppressed_range().is_some())
        .map(|b| b.id)
        .collect()
}

/// Where a block ran, for "Compare with previous run": the remote host and
/// its path as recorded, or the local directory resolved (symlinks, so a
/// relaunch that reports `/private/tmp` for `/tmp` still matches; as
/// recorded when it no longer exists).
fn place(b: &rift_core::Block) -> (Option<String>, Option<String>) {
    place_of(b.cwd_remote, b.host.as_deref(), b.cwd.as_deref())
}

fn place_of(
    remote: bool,
    host: Option<&str>,
    cwd: Option<&str>,
) -> (Option<String>, Option<String>) {
    if remote {
        return (host.map(str::to_string), cwd.map(str::to_string));
    }
    let cwd = cwd.map(|c| {
        rift_core::sys::canonicalize(c)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| c.to_string())
    });
    (None, cwd)
}

/// A file name from a command: its first word's letters, digits, `-`, `_`
/// and `.` ("block" when nothing is left).
fn file_stem(command: &str) -> String {
    let word = command.split_whitespace().next().unwrap_or("");
    let word = word.rsplit('/').next().unwrap_or(word);
    let stem: String = word
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(40)
        .collect();
    let stem = stem.trim_start_matches('.');
    if stem.is_empty() {
        "block".into()
    } else {
        stem.to_string()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::place_of;
    use super::{file_stem, write_regular_file};

    #[test]
    fn save_never_replaces_a_file_without_confirmation() {
        let path = std::env::temp_dir().join(format!("rift-save-race-{}", std::process::id()));
        std::fs::write(&path, b"keep").unwrap();
        assert!(write_regular_file(&path, b"new", false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        write_regular_file(&path, b"new", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_file(&path).unwrap();
        write_regular_file(&path, b"created", false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"created");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn save_rechecks_special_files_and_accepts_regular_symlinks() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("rift-save-special-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("target");
        let link = dir.join("link");
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"old text").unwrap();
        std::os::unix::fs::symlink(&path, &link).unwrap();
        write_regular_file(&link, b"new", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        std::fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // A reader keeps a broken implementation from hanging the test.
        let _reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        assert!(write_regular_file(&link, b"must not enter the pipe", true).is_err());
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    /// "Compare with previous run" matches a directory through a symlink
    /// (`/tmp` and `/private/tmp` after a relaunch) and a remote one only
    /// on the same host.
    #[test]
    #[cfg(unix)]
    fn previous_run_places() {
        let dir = std::env::temp_dir().join(format!("rift-place-{}", std::process::id()));
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.join("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (r, l) = (real.to_string_lossy(), link.to_string_lossy());
        assert_eq!(
            place_of(false, None, Some(&r)),
            place_of(false, None, Some(&l))
        );
        // gone: as recorded
        assert_eq!(
            place_of(false, None, Some("/no/such/rift-dir"))
                .1
                .as_deref(),
            Some("/no/such/rift-dir")
        );
        // remote: the host counts, the path is never read here
        assert_eq!(
            place_of(true, Some("a"), Some("/srv")),
            (Some("a".into()), Some("/srv".into()))
        );
        assert_ne!(
            place_of(true, Some("a"), Some("/srv")),
            place_of(true, Some("b"), Some("/srv"))
        );
        assert_ne!(
            place_of(true, Some("a"), Some(&r)),
            place_of(false, None, Some(&r))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_stems_are_safe_names() {
        assert_eq!(file_stem("cargo test -p x"), "cargo");
        assert_eq!(file_stem("/usr/bin/ls -la"), "ls");
        assert_eq!(file_stem("../../etc/passwd"), "passwd");
        assert_eq!(file_stem("$(rm -rf ~)"), "rm");
        assert_eq!(file_stem("..."), "block");
        assert_eq!(file_stem(""), "block");
    }
}
