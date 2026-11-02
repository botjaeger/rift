use rift_core::git::GitInfo;
use rift_pty::{PaneId, Session};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

use crate::pane::{NodePath, PaneNode};
use crate::vim::{VimMode, VimState};

/// A sample alert from Settings › Notifications › Try it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestAlert {
    pub event: rift_core::notifications::Event,
    /// A desktop notification (as while Rift is in the background), else
    /// an in-window toast.
    pub desktop: bool,
}

/// What a mouse click landed on (rebuilt every frame).
#[derive(Debug, Clone, PartialEq)]
pub enum ClickTarget {
    Block(PaneId, rift_core::BlockId),
    BlockChevron(PaneId, rift_core::BlockId),
    BlockCopy(PaneId, rift_core::BlockId),
    /// A header's ⋮ button: the block's context menu.
    BlockMenu(PaneId, rift_core::BlockId),
    /// A header's duration: hover shows the full time; a click selects
    /// the block like the rest of the header.
    BlockDuration(PaneId, rift_core::BlockId),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    TrafficLight(u8),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    WindowDrag,
    TabSelect(usize),
    TabClose(usize),
    /// A tab group's header (strip pill, rail row): fold / open; a
    /// double-click renames, a right-click opens its menu.
    GroupHeader(u64),
    /// The vertical rail's search field, its panes-view toggle, and a pane
    /// row of that view.
    RailSearch,
    RailPanesToggle,
    RailPane(PaneId),
    TabNew,
    /// The ⌄ beside +: the profile menu.
    TabProfiles,
    /// Top-bar sidebar toggles.
    ToggleExplorer,
    ToggleReview,
    /// The code review button while the Settings tab is shown: dimmed, a
    /// tooltip only (Settings has no repository).
    ReviewUnavailable,
    /// Top-bar inbox: the notifications popover.
    ToggleNotifications,
    /// Toolbar buttons (`toolbar_left` / `toolbar_right`): the command
    /// palette and Settings.
    TogglePalette,
    ToggleSettings,
    /// Top-bar "Update Rift" chip: the update dialog.
    UpdateChip,
    /// Top-bar "Remote · …" chip (a phone is connected): Settings › Remote
    /// access.
    RemoteChip,
    /// The owned command editor of a pane (click places the cursor).
    Editor(PaneId),
    /// A clickable prompt chip (Warp): the cwd, branch, PHP, Python and
    /// Docker chips open their menus, the diff chip the code review panel,
    /// the PR chip the pull request in the browser.
    Chip(PaneId, rift_core::PromptChip),
    /// The chip after the ghost text naming its accept key: a menu of the
    /// keys (`views::editor::AcceptKey`).
    GhostKey(PaneId),
    /// The ✕ after the ghost text (`autosuggestion_ignore_button`): never
    /// suggest that command line again.
    GhostIgnore(PaneId),
    PaneFocus(PaneId),
    /// A split pane's title bar: a press focuses the pane and arms
    /// drag-to-move (Warp pane header).
    PaneHeader(PaneId),
    /// A palette mode chip (`views::palette::MODES` index).
    PaletteMode(u8),
    /// The history pane's scope chips, row `i` (of the results) and ×.
    HistoryScope(crate::views::history::Scope),
    HistoryRow(usize),
    HistoryClose,
    /// The completion panel's row `i`.
    CompletionRow(usize),
    /// A row of the welcome list on a fresh session (`app::welcome_entries`).
    Welcome(u8),
    /// The title bar's split (new pane to the right), ⋮ (pane menu) and ×
    /// (close pane).
    PaneSplit(PaneId),
    PaneMenu(PaneId),
    PaneClose(PaneId),
    /// Divider `index` of the split at the path.
    HandleDrag(NodePath, usize),
    JumpToBlock(PaneId, rift_core::BlockId),
    /// The minimized sticky command header's tab: show the header again.
    StickyRestore(PaneId),
    /// The jump-to-bottom button over a block running past the pane.
    JumpToBlockEnd(PaneId, rift_core::BlockId),
    /// The pane's overlay scrollbar (`views::scrollbar`).
    Scrollbar(PaneId),
    /// The subshell offer's Enable, Don't ask again and ✕.
    Subshell(PaneId, crate::views::subshell::Part),
    /// The Restart bar of a pane whose program exited: Restart, Close.
    Exited(PaneId, crate::views::exited::Part),
    // settings overlay controls
    SettingsNoop,
    /// Settings › Remote access › Recent connections while it holds more
    /// rows than it shows: the wheel over it scrolls the list.
    RemoteHistoryList,
    SettingsNav(u8),
    SettingsClose,
    SettingsToggle(&'static str),
    SettingsStep(&'static str, i8),
    SettingsSegment(&'static str, u8),
    /// The prompt chips preview: opens "Edit prompt".
    SettingsEditPrompt,
    /// The toolbar preview: opens "Edit toolbar".
    SettingsEditToolbar,
    /// A select: opens its option list.
    SettingsCycle(&'static str),
    /// Option `i` of the open select's list.
    SettingsOption(usize),
    /// ↺ beside a setting changed from its default.
    SettingsReset(&'static str),
    /// The sidebar's "Open config.toml".
    SettingsOpenConfig,
    /// Settings › About: copy the version.
    SettingsCopyVersion,
    /// Settings › About: "Update…" (the update dialog).
    SettingsUpdate,
    /// Settings › General and About: "Check now" (one update check).
    SettingsCheckUpdates,
    /// Settings › General › What's New: open the release notes.
    SettingsWhatsNew,
    /// Settings › Notifications ▷ beside a sound select: play what it plays.
    SettingsPlaySound(&'static str),
    /// Settings › Notifications › Try it: a sample alert.
    SettingsTestAlert(TestAlert),
    /// Settings › Notifications › Terminal bell: ring it as `bell` sets it.
    SettingsRingBell,
    /// Settings › Notifications › Desktop notifications: Allow… (ask macOS)
    /// or Open System Settings.
    SettingsNotifyPermission,
    SettingsSlider(&'static str),
    SettingsTheme(u8),
    /// Settings › Themes: show all themes (0), the dark (1) or the light
    /// ones (2).
    SettingsThemeFilter(u8),
    /// Settings › Themes › "Open themes folder" (created when missing).
    SettingsOpenThemes,
    /// Keybindings page row: record a new shortcut for the action.
    SettingsKeybind(String),
    /// Keybindings page ↺: drop the override (back to the default).
    SettingsKeybindReset(String),
    /// Settings › Remote access: "Pair a phone" / "Pair again".
    RemotePair,
    /// Settings › Remote access: pair the next phone view-only.
    RemoteViewOnly,
    /// Settings › Remote access: "Cancel" under a shown pairing code (the
    /// code stops working).
    RemotePairCancel,
    /// Settings › Remote access: "Recent connections" opens or closes.
    RemoteHistory,
    /// Settings › Remote access: copy the pairing link.
    RemoteCopyLink,
    /// Settings › Remote access: install cloudflared (Homebrew in a new
    /// tab, else Cloudflare's download page).
    RemoteInstallTunnel,
    /// Settings › Remote access: revoke the paired device with this id.
    RemoteRevoke(String),
    /// Settings › Remote access: the relay URL field (edit `remote_relay`).
    RemoteRelay,
    /// Settings › Remote access › Connection: "Try again" (retry the relay
    /// now).
    RemoteRelayRetry,
    /// Settings › Remote access: "Connection settings" opens or closes the
    /// listen address, the relay address and the Cloudflare tunnel.
    RemoteRelaySettings,
    /// Settings › Remote access: set the device with this id to Operate
    /// (`false`, confirmed first) or View only (`true`).
    RemotePermission(String, bool),
    /// A Settings text field (`shell`, `global_hotkey`, a profile's
    /// `profile.name`, …): a click edits it, ⏎ or a click away saves it.
    SettingsText(&'static str),
    /// Settings › Profiles: profile `i`'s row (config.toml order). A click
    /// opens or closes its edit form; a drag moves it.
    ProfileRow(usize),
    /// Settings › Profiles: a button acting on profile `i`.
    Profile(usize, ProfileOp),
    /// Settings › Profiles: "Add" (a new profile running the login shell).
    ProfileAdd,
    /// Structured Settings editors, using the same forms as Profiles.
    Collection(CollectionKind, usize, CollectionOp),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Ssh,
    Actions,
    Secrets,
    Colors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionOp {
    Add,
    Edit,
    Save,
    Cancel,
    Delete,
    ConfirmDelete,
    Up,
    Down,
    Connect,
    AddStep,
    SelectStep(usize),
    RemoveStep,
    StepUp,
    StepDown,
    StepType(u8),
    RecordKey,
    Color(u8),
}

/// What a Settings › Profiles button does to its profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileOp {
    /// Open or close its edit form.
    Edit,
    SetDefault,
    Duplicate,
    /// Hide it (or show it again): kept in the file, offered nowhere.
    ToggleHidden,
    /// Asked first (`ConfirmAction::DeleteProfile`).
    Delete,
}

pub struct Hit {
    pub rect: [f32; 4],
    pub target: ClickTarget,
}

impl Hit {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, rw, rh] = self.rect;
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }
}

/// Line editor for the owned input zone (Warp model — the app edits the
/// command line locally; the shell sees it only on Enter).
#[derive(Default)]
pub struct EditorState {
    pub buffer: String,
    /// Byte offset, always on a char boundary.
    pub cursor: usize,
    /// Commands submitted here, oldest first. Shared: a completion request
    /// takes it without copying (a push while one holds it copies once).
    pub history: std::sync::Arc<Vec<String>>,
    hist_idx: Option<usize>,
    saved: String,
    /// (buffer, cursor) snapshots for ⌘Z / ⇧⌘Z; cleared on submit.
    undo_stack: Vec<(String, usize)>,
    redo_stack: Vec<(String, usize)>,
    /// Snapshots dropped from the front of `undo_stack` at [`UNDO_CAP`]:
    /// keeps the depths vim marks with ([`Self::undo_len`]) valid.
    undo_evicted: usize,
    /// Modal editing state; `None` = vim mode disabled (see `vim.rs`).
    pub vim: Option<VimState>,
    /// Selection anchor (byte offset); the selection spans anchor..cursor.
    pub anchor: Option<usize>,
    /// More carets (multi-cursor, Warp): each a cursor and its selection
    /// anchor, like `cursor` and `anchor`, which stay the primary one.
    /// Edits and motions act at every caret ([`Self::at_each_caret`]); Esc,
    /// a plain click, undo, history and submit go back to one.
    pub extra: Vec<(usize, Option<usize>)>,
    /// Meta+.: how many commands back the inserted last word came from,
    /// and where it sits in the buffer (the next Meta+. replaces it).
    pub last_arg: Option<(usize, std::ops::Range<usize>)>,
    /// Characters that end a word for word moves and deletes besides
    /// blanks (`word_delimiters`).
    pub word_delimiters: String,
}

/// The span an edit changed, from `before` to `after`: (start, end in
/// `before`, end in `after`), byte offsets on character boundaries.
fn changed_span(before: &str, after: &str) -> (usize, usize, usize) {
    let mut start = before
        .bytes()
        .zip(after.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(start) || !after.is_char_boundary(start) {
        start -= 1;
    }
    let max_suffix = before.len().min(after.len()) - start;
    let mut suffix = before
        .bytes()
        .rev()
        .zip(after.bytes().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !before.is_char_boundary(before.len() - suffix)
        || !after.is_char_boundary(after.len() - suffix)
    {
        suffix -= 1;
    }
    (start, before.len() - suffix, after.len() - suffix)
}

const UNDO_CAP: usize = 100;

impl EditorState {
    /// True when the pane's editor is in vim mode.
    pub fn vim_enabled(&self) -> bool {
        self.vim.is_some()
    }

    /// Turn vim mode on/off (config `vim_mode`). Enabling starts in Insert;
    /// toggling is a no-op when the state already matches, so a config
    /// hot-reload never drops the mode the user is currently in.
    pub fn set_vim_enabled(&mut self, on: bool) {
        match (on, self.vim.is_some()) {
            (true, false) => self.vim = Some(VimState::default()),
            (false, true) => self.vim = None,
            _ => {}
        }
    }

    /// Current vim mode, or `None` when vim mode is disabled.
    pub fn vim_mode(&self) -> Option<VimMode> {
        self.vim.as_ref().map(|v| match v.mode {
            // the selection was cleared under visual mode (a click, a
            // paste): the next key leaves it, so report Normal already
            VimMode::Visual | VimMode::VisualLine if self.anchor.is_none() => VimMode::Normal,
            m => m,
        })
    }

    /// The one caret stands where the ghost text continues the line: at its
    /// end, or in vim's Normal mode on its last character (the block
    /// cursor rests there). The ghost text and its chips show then, and the
    /// accept keys take it.
    pub fn ghost_at_end(&self) -> bool {
        let last_char = self.buffer.char_indices().last().map_or(0, |(i, _)| i);
        self.cursor == self.buffer.len()
            || (self.vim_mode() == Some(VimMode::Normal) && self.cursor == last_char)
    }

    /// vim: the undo-stack depth, marking where a change begins.
    pub(crate) fn undo_len(&self) -> usize {
        self.undo_evicted + self.undo_stack.len()
    }

    /// vim: fold every snapshot pushed since `mark` into the first, so one
    /// undo reverts the whole change (an Insert session, a `.` repeat), and
    /// puts the cursor back where the change began (`cursor`), not where its
    /// first mutation happened to sit.
    pub(crate) fn squash_undo(&mut self, mark: usize, cursor: usize) {
        // the mark counts snapshots evicted at the cap since; when the
        // change's own first snapshot went too, the oldest one left stands in
        let (at, kept) = match mark.checked_sub(self.undo_evicted) {
            Some(at) => (at, true),
            None => (0, false),
        };
        self.undo_stack.truncate(at + 1);
        if kept
            && self.undo_stack.len() == at + 1
            && let Some((buf, cur)) = self.undo_stack.last_mut()
            && buf.is_char_boundary(cursor.min(buf.len()))
        {
            *cur = cursor.min(buf.len());
        }
    }

    /// More than one caret.
    pub fn has_extra(&self) -> bool {
        !self.extra.is_empty()
    }

    /// Back to the primary caret alone.
    pub fn collapse_carets(&mut self) {
        self.extra.clear();
    }

    /// Every caret's selection (non-empty ones), primary first.
    pub fn selections(&self) -> Vec<std::ops::Range<usize>> {
        let mut all: Vec<_> = self.selection().into_iter().collect();
        for &(c, a) in &self.extra {
            if let Some(a) = a.map(|a| a.min(self.buffer.len()))
                && a != c
            {
                all.push(a.min(c)..a.max(c));
            }
        }
        all
    }

    /// Run `op` at every caret, as if each were the only one: the last in
    /// the buffer first, so an edit never moves the carets still to come,
    /// and those done move by the edit's length change. One undo step for
    /// all of them; carets that end up on each other merge.
    pub fn at_each_caret(&mut self, mut op: impl FnMut(&mut EditorState)) {
        let mark = self.undo_len();
        let start_cursor = self.cursor;
        // (cursor, anchor, primary), from the end of the buffer back
        let mut todo: Vec<(usize, Option<usize>, bool)> =
            std::iter::once((self.cursor, self.anchor, true))
                .chain(self.extra.iter().map(|&(c, a)| (c, a, false)))
                .collect();
        todo.sort_by_key(|&(c, a, _)| std::cmp::Reverse(a.map_or(c, |a| a.min(c))));
        let mut done: Vec<(usize, Option<usize>, bool)> = Vec::with_capacity(todo.len());
        let mut k = 0;
        while k < todo.len() {
            let (c, a, primary) = todo[k];
            k += 1;
            self.cursor = c.min(self.buffer.len());
            self.anchor = a.map(|a| a.min(self.buffer.len()));
            let before = self.buffer.clone();
            op(self);
            if self.buffer == before {
                done.push((self.cursor, self.anchor, primary));
                continue;
            }
            // carets past this edit move with it; one inside what it replaced
            // (a word deleted across it) lands at its end, and one still to
            // come that it swallowed merges there instead of acting again
            let (start, old_end, new_end) = changed_span(&before, &self.buffer);
            let map = |v: usize| {
                if v >= old_end {
                    v - old_end + new_end
                } else if v > start {
                    new_end
                } else {
                    v
                }
            };
            for d in &mut done {
                d.0 = map(d.0);
                d.1 = d.1.map(map);
            }
            done.push((self.cursor, self.anchor, primary));
            for t in &mut todo[k..] {
                let lo = t.1.map_or(t.0, |a| a.min(t.0));
                if lo > start && lo < old_end {
                    done.push((new_end, None, t.2));
                    t.2 = false;
                    t.0 = usize::MAX;
                }
            }
            todo.retain(|t| t.0 != usize::MAX);
        }
        self.set_carets(done);
        self.squash_undo(mark, start_cursor);
    }

    /// Store carets (one of them the primary), sorted, without duplicates
    /// or overlapping selections.
    fn set_carets(&mut self, mut all: Vec<(usize, Option<usize>, bool)>) {
        // A caret or selection endpoint never splits a grapheme cluster.
        let snap = |v: usize| grapheme_floor(&self.buffer, v);
        for c in &mut all {
            c.0 = snap(c.0);
            c.1 = c.1.map(snap).filter(|a| *a != c.0);
        }
        all.sort_by_key(|&(c, a, _)| (a.map_or(c, |a| a.min(c)), c));
        let mut kept: Vec<(usize, Option<usize>, bool)> = Vec::with_capacity(all.len());
        for c in all {
            let span = |x: &(usize, Option<usize>, bool)| {
                let a = x.1.unwrap_or(x.0);
                (a.min(x.0), a.max(x.0))
            };
            if let Some(last) = kept.last_mut() {
                let (s0, e0) = span(last);
                let (s1, e1) = span(&c);
                if s1 < e0 || (s1, e1) == (s0, e0) {
                    // merged: the primary survives
                    last.2 |= c.2;
                    continue;
                }
            }
            kept.push(c);
        }
        let primary = kept.iter().position(|c| c.2).unwrap_or(0);
        let (c, a, _) = kept.remove(primary);
        self.cursor = c;
        self.anchor = a;
        self.extra = kept.into_iter().map(|(c, a, _)| (c, a)).collect();
    }

    /// Ctrl-⇧-↑ / ↓: a caret on the line above the topmost caret (or below
    /// the bottommost), at the same column or the line's end.
    pub fn add_caret_vertical(&mut self, down: bool) {
        let all: Vec<usize> = std::iter::once(self.cursor)
            .chain(self.extra.iter().map(|c| c.0))
            .collect();
        let from = if down {
            *all.iter().max().unwrap_or(&self.cursor)
        } else {
            *all.iter().min().unwrap_or(&self.cursor)
        };
        let Some(at) = self.vertical_from(from, down) else {
            return;
        };
        let mut all: Vec<_> = std::iter::once((self.cursor, self.anchor, true))
            .chain(self.extra.iter().map(|&(c, a)| (c, a, false)))
            .collect();
        all.push((at, None, false));
        self.set_carets(all);
    }

    /// The offset one line above (or below) `from`, at the same column or
    /// that line's end; `None` on the first (last) line.
    fn vertical_from(&self, from: usize, down: bool) -> Option<usize> {
        let line_start = self.buffer[..from].rfind('\n').map_or(0, |i| i + 1);
        let col = self.buffer[line_start..from].graphemes(true).count();
        let target_start = if down {
            from + self.buffer[from..].find('\n')? + 1
        } else {
            if line_start == 0 {
                return None;
            }
            self.buffer[..line_start - 1]
                .rfind('\n')
                .map_or(0, |i| i + 1)
        };
        let target_end = self.buffer[target_start..]
            .find('\n')
            .map_or(self.buffer.len(), |i| target_start + i);
        Some(
            self.buffer[target_start..target_end]
                .grapheme_indices(true)
                .nth(col)
                .map_or(target_end, |(i, _)| target_start + i),
        )
    }

    /// ↑ / ↓ inside a multi-line command: the cursor one line up (down);
    /// `false` on the first (last) line, where the keys walk history.
    pub fn line_vertical(&mut self, down: bool) -> bool {
        match self.vertical_from(self.cursor, down) {
            Some(at) => {
                self.anchor = None;
                self.extra.clear();
                self.cursor = at;
                true
            }
            None => false,
        }
    }

    /// Ctrl-G: select the word at the cursor, or with a selection, add a
    /// caret selecting its next occurrence (after the last caret, wrapping).
    pub fn add_next_occurrence(&mut self) {
        let Some(sel) = self.selection() else {
            let word = |g: &str| {
                g.chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-')
            };
            let start = self.buffer[..self.cursor]
                .grapheme_indices(true)
                .rev()
                .take_while(|&(_, c)| word(c))
                .last()
                .map_or(self.cursor, |(i, _)| i);
            let end = self.buffer[self.cursor..]
                .grapheme_indices(true)
                .find(|&(_, c)| !word(c))
                .map_or(self.buffer.len(), |(i, _)| self.cursor + i);
            if start < end {
                self.anchor = Some(start);
                self.cursor = end;
            }
            return;
        };
        let needle = self.buffer[sel].to_string();
        let taken: Vec<(usize, usize)> = self
            .selections()
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect();
        let last_end = taken.iter().map(|t| t.1).max().unwrap_or(0);
        // the first free one after the last caret, wrapping to the start
        let free = |s: &usize| {
            is_grapheme_boundary(&self.buffer, *s)
                && is_grapheme_boundary(&self.buffer, *s + needle.len())
                && !taken.contains(&(*s, *s + needle.len()))
        };
        let found = self
            .buffer
            .match_indices(&needle)
            .map(|(i, _)| i)
            .filter(|i| *i >= last_end)
            .find(free)
            .or_else(|| {
                self.buffer
                    .match_indices(&needle)
                    .map(|(i, _)| i)
                    .find(free)
            });
        if let Some(s) = found {
            let mut all: Vec<_> = std::iter::once((self.cursor, self.anchor, true))
                .chain(self.extra.iter().map(|&(c, a)| (c, a, false)))
                .collect();
            all.push((s + needle.len(), Some(s), false));
            self.set_carets(all);
        }
    }

    /// ⌘-click (⌥-click on Linux): one more caret at `offset`.
    /// One on a caret takes that caret away (while another stays).
    pub fn add_caret_at(&mut self, offset: usize) {
        let offset = grapheme_floor(&self.buffer, offset);
        let mut all: Vec<_> = std::iter::once((self.cursor, self.anchor, true))
            .chain(self.extra.iter().map(|&(c, a)| (c, a, false)))
            .collect();
        if all.len() > 1
            && let Some(i) = all.iter().position(|c| c.0 == offset)
        {
            let gone = all.remove(i);
            if gone.2 {
                all[0].2 = true;
            }
        } else {
            all.push((offset, None, false));
        }
        self.set_carets(all);
    }

    /// Paste at every caret: one line each when the text has as many lines
    /// as there are carets, else all of it at each.
    pub fn paste_at_carets(&mut self, text: &str) {
        let lines: Vec<&str> = text.lines().collect();
        let n = 1 + self.extra.len();
        if lines.len() == n {
            // carets in buffer order get the lines in order: hand them out
            // from the end, as `at_each_caret` visits them
            let mut from_end = lines.into_iter().rev();
            self.at_each_caret(|e| e.insert(from_end.next().unwrap_or_default()));
        } else {
            self.at_each_caret(|e| e.insert(text));
        }
    }

    /// Snapshot the current buffer before a mutation (skips no-op duplicates).
    fn push_undo(&mut self) {
        if self.undo_stack.last().map(|(b, _)| b) == Some(&self.buffer) {
            return;
        }
        if self.undo_stack.len() >= UNDO_CAP {
            self.undo_stack.remove(0);
            self.undo_evicted += 1;
        }
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.redo_stack.clear();
    }

    /// Show `line` in place of the buffer without an undo snapshot: the
    /// history menu's previews, and its way back to what was typed.
    pub fn preview_line(&mut self, line: &str, cursor: usize) {
        self.extra.clear();
        self.anchor = None;
        self.buffer.clear();
        self.buffer.push_str(line);
        self.cursor = cursor.min(self.buffer.len());
        self.hist_idx = None;
    }

    /// The previewed line stays: one undo step leads back to `typed`.
    pub fn keep_preview(&mut self, typed: &str, cursor: usize) {
        if self.buffer == typed {
            return;
        }
        let line = std::mem::replace(&mut self.buffer, typed.to_string());
        let at = std::mem::replace(&mut self.cursor, cursor.min(typed.len()));
        self.push_undo();
        self.buffer = line;
        self.cursor = at;
    }

    pub fn undo(&mut self) {
        let Some((buf, cur)) = self.undo_stack.pop() else {
            return;
        };
        self.extra.clear();
        self.redo_stack.push((self.buffer.clone(), self.cursor));
        self.anchor = None;
        self.buffer = buf;
        self.cursor = cur.min(self.buffer.len());
        self.hist_idx = None;
    }

    pub fn redo(&mut self) {
        let Some((buf, cur)) = self.redo_stack.pop() else {
            return;
        };
        self.extra.clear();
        // bypass push_undo: it would clear redo_stack
        if self.undo_stack.len() >= UNDO_CAP {
            self.undo_stack.remove(0);
            self.undo_evicted += 1;
        }
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.anchor = None;
        self.buffer = buf;
        self.cursor = cur.min(self.buffer.len());
        self.hist_idx = None;
    }

    /// Selected byte range (non-empty), if any.
    pub fn selection(&self) -> Option<std::ops::Range<usize>> {
        // vim's visual modes select inclusively (whole lines in `V`)
        if let Some(v) = &self.vim
            && matches!(v.mode, VimMode::Visual | VimMode::VisualLine)
        {
            return self.vim_visual_range(v.mode);
        }
        let a = self.anchor?.min(self.buffer.len());
        (a != self.cursor).then(|| a.min(self.cursor)..a.max(self.cursor))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.buffer[r])
    }

    pub fn select_all(&mut self) {
        self.extra.clear();
        self.anchor = Some(0);
        self.cursor = self.buffer.len();
    }

    /// Replace the selection with `s`; false when nothing is selected.
    fn replace_selection(&mut self, s: &str) -> bool {
        match self.selection() {
            Some(r) => {
                self.replace_range(r, s);
                self.hist_idx = None;
                true
            }
            None => {
                self.anchor = None;
                false
            }
        }
    }

    /// Cut the selection (⌘X): returns the removed text.
    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?.to_string();
        self.replace_selection("");
        Some(text)
    }

    /// Shift+motion: extend the selection with `motion`.
    pub fn extend(&mut self, motion: fn(&mut EditorState)) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        motion(self);
        self.anchor = Some(anchor);
    }

    /// Mouse: place the cursor (optionally extending the selection).
    pub fn click_to(&mut self, offset: usize, extend: bool) {
        let offset = grapheme_floor(&self.buffer, offset);
        self.extra.clear();
        if extend {
            let anchor = self.anchor.unwrap_or(self.cursor);
            self.cursor = offset;
            self.anchor = Some(anchor);
        } else {
            self.cursor = offset;
            self.anchor = None;
        }
    }

    /// Single mutation point: every buffer edit goes through here (undo
    /// snapshot, selection reset, cursor after the replacement).
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, s: &str) {
        self.push_undo();
        self.anchor = None;
        self.buffer.replace_range(range.clone(), s);
        self.cursor = range.start + s.len();
    }

    pub fn insert(&mut self, s: &str) {
        if !self.replace_selection(s) {
            self.replace_range(self.cursor..self.cursor, s);
        }
        self.hist_idx = None;
    }

    /// IDE-style quote/bracket insertion. Closing an existing pair steps over
    /// it, while quotes inside identifiers remain literal (for e.g. `don't`).
    pub fn insert_with_pairs(&mut self, s: &str) {
        let mut chars = s.chars();
        let Some(ch) = chars.next() else {
            return;
        };
        if chars.next().is_some() {
            self.insert(s);
            return;
        }

        if matches!(ch, ')' | ']' | '}' | '\'' | '"' | '`')
            && self.selection().is_none()
            && self.buffer[self.cursor..].starts_with(ch)
        {
            self.cursor = self.next_boundary();
            self.hist_idx = None;
            return;
        }

        // inside a quoted string the text is literal: a bracket or the other
        // quote pairs nothing, and the closing quote ends the string (paired,
        // `'[a'` used to leave `]'` behind)
        if self.selection().is_none() && quote_open_at(&self.buffer[..self.cursor]) {
            self.insert(s);
            return;
        }
        let close = match ch {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '\'' => '\'',
            '"' => '"',
            '`' => '`',
            _ => {
                self.insert(s);
                return;
            }
        };
        // an opener typed over a selection wraps it (IDE-style)
        if let Some(r) = self.selection() {
            let wrapped = format!("{ch}{}{close}", &self.buffer[r.clone()]);
            let start = r.start;
            let inner_end = r.end + ch.len_utf8();
            self.replace_range(r, &wrapped);
            self.anchor = Some(start + ch.len_utf8());
            self.cursor = inner_end;
            self.hist_idx = None;
            return;
        }
        let is_quote = matches!(ch, '\'' | '"' | '`');
        let touches_identifier = self.buffer[..self.cursor]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
            || self.buffer[self.cursor..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if is_quote && touches_identifier {
            self.insert(s);
            return;
        }

        let start = self.cursor;
        let pair = format!("{ch}{close}");
        self.replace_range(start..start, &pair);
        self.cursor = start + ch.len_utf8();
        self.hist_idx = None;
    }

    /// Start of the grapheme cluster before the cursor (extended clusters:
    /// a combining sequence, ZWJ emoji or flag moves and deletes as one).
    fn prev_boundary(&self) -> usize {
        grapheme_prev(&self.buffer, self.cursor)
    }

    /// End of the grapheme cluster after the cursor.
    fn next_boundary(&self) -> usize {
        grapheme_next(&self.buffer, self.cursor)
    }

    pub fn backspace(&mut self) {
        if self.replace_selection("") {
            return;
        }
        if self.cursor > 0 {
            let p = self.prev_boundary();
            self.replace_range(p..self.cursor, "");
        }
    }

    /// Remove both halves when the cursor is inside an auto-inserted pair.
    pub fn backspace_with_pairs(&mut self) {
        if self.selection().is_some() {
            self.backspace();
            return;
        }
        if self.cursor > 0 && self.cursor < self.buffer.len() {
            let prev = self.prev_boundary();
            let next = self.next_boundary();
            // whole clusters: `(` carrying a combining mark is not a bare opener
            let left = &self.buffer[prev..self.cursor];
            let right = &self.buffer[self.cursor..next];
            if matches!(
                (left, right),
                ("(", ")") | ("[", "]") | ("{", "}") | ("'", "'") | ("\"", "\"") | ("`", "`")
            ) {
                self.replace_range(prev..next, "");
                self.hist_idx = None;
                return;
            }
        }
        self.backspace();
    }

    pub fn delete(&mut self) {
        if self.replace_selection("") {
            return;
        }
        let n = self.next_boundary();
        if n > self.cursor {
            self.replace_range(self.cursor..n, "");
        }
    }

    pub fn delete_word_back(&mut self) {
        if self.replace_selection("") {
            return;
        }
        let cut = self.word_left_target();
        self.replace_range(cut..self.cursor, "");
    }

    /// A blank or a `word_delimiters` character: where words end.
    fn is_word_end(&self, c: char) -> bool {
        c.is_whitespace() || self.word_delimiters.contains(c)
    }

    /// Start of the word left of the cursor (words end at blanks and
    /// `word_delimiters`).
    fn word_left_target(&self) -> usize {
        let trimmed = self.buffer[..self.cursor].trim_end_matches(|c| self.is_word_end(c));
        // step past the whole whitespace char: U+3000/U+00A0 are multi-byte
        let i = trimmed
            .char_indices()
            .rfind(|&(_, c)| self.is_word_end(c))
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        // a space carrying a combining mark is one cluster: never stop inside it
        grapheme_floor(&self.buffer, i)
    }

    /// End of the word right of the cursor (words end at blanks and
    /// `word_delimiters`).
    fn word_right_target(&self) -> usize {
        let rest = &self.buffer[self.cursor..];
        let skipped = rest.len() - rest.trim_start_matches(|c| self.is_word_end(c)).len();
        let word = &rest[skipped..];
        let end = word.find(|c| self.is_word_end(c)).unwrap_or(word.len());
        grapheme_ceil(&self.buffer, self.cursor + skipped + end)
    }

    // raw motions (selection-agnostic); `extend` wraps them for Shift
    pub fn mv_word_left(&mut self) {
        self.cursor = self.word_left_target();
    }

    pub fn mv_word_right(&mut self) {
        self.cursor = self.word_right_target();
    }

    pub fn mv_left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn mv_right(&mut self) {
        self.cursor = self.next_boundary();
    }

    /// Start of the cursor's line (⌘← / Home / ⌃A): the buffer start on a
    /// one-line command.
    pub fn mv_home(&mut self) {
        self.cursor = self.buffer[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
    }

    /// End of the cursor's line (⌘→ / End / ⌃E).
    pub fn mv_end(&mut self) {
        self.cursor = self.buffer[self.cursor..]
            .find('\n')
            .map_or(self.buffer.len(), |i| self.cursor + i);
    }

    /// Start of the whole command (⌘↑ in a multi-line command).
    pub fn mv_doc_start(&mut self) {
        self.cursor = 0;
    }

    /// End of the whole command (⌘↓ in a multi-line command).
    pub fn mv_doc_end(&mut self) {
        self.cursor = self.buffer.len();
    }

    /// The cursor is on a line below the first: ⌘↑ moves within the
    /// command rather than selecting the block above.
    pub fn below_first_line(&self) -> bool {
        self.buffer[..self.cursor].contains('\n')
    }

    /// The cursor is on a line above the last: ⌘↓ moves within the command.
    pub fn above_last_line(&self) -> bool {
        self.buffer[self.cursor..].contains('\n')
    }

    pub fn word_left(&mut self) {
        self.anchor = None;
        self.mv_word_left();
    }

    pub fn word_right(&mut self) {
        self.anchor = None;
        self.mv_word_right();
    }

    pub fn delete_word_forward(&mut self) {
        if self.replace_selection("") {
            return;
        }
        let end = self.word_right_target();
        if end > self.cursor {
            self.replace_range(self.cursor..end, "");
        }
    }

    /// ⌃K: delete to the end of the cursor's line; at a line's end, the
    /// newline after it (joins the next line, like readline and macOS).
    pub fn kill_to_end(&mut self) {
        self.anchor = None;
        let start = self.cursor;
        self.mv_end();
        let mut end = self.cursor;
        self.cursor = start;
        if end == start && self.buffer[start..].starts_with('\n') {
            end += 1;
        }
        if end > start {
            self.replace_range(start..end, "");
        }
    }

    /// ⌘⌫: delete to the start of the cursor's line; at a line's start,
    /// the newline before it.
    pub fn kill_to_start(&mut self) {
        self.anchor = None;
        let end = self.cursor;
        self.mv_home();
        let mut start = self.cursor;
        self.cursor = end;
        if start == end && self.buffer[..end].ends_with('\n') {
            start -= 1;
        }
        if end > start {
            self.replace_range(start..end, "");
        }
    }

    /// ← collapses a selection to its start, else moves one char.
    pub fn left(&mut self) {
        match self.selection() {
            Some(r) => self.cursor = r.start,
            None => self.mv_left(),
        }
        self.anchor = None;
    }

    /// → collapses a selection to its end, else moves one char.
    pub fn right(&mut self) {
        match self.selection() {
            Some(r) => self.cursor = r.end,
            None => self.mv_right(),
        }
        self.anchor = None;
    }

    pub fn home(&mut self) {
        self.anchor = None;
        self.mv_home();
    }

    pub fn end(&mut self) {
        self.anchor = None;
        self.mv_end();
    }

    pub fn clear(&mut self) {
        self.anchor = None;
        self.extra.clear();
        self.push_undo();
        self.buffer.clear();
        self.cursor = 0;
        self.hist_idx = None;
        // fresh line → back to typing (Warp behavior)
        if let Some(v) = &mut self.vim {
            v.reset_insert();
        }
    }

    pub fn submit(&mut self) -> String {
        let cmd = std::mem::take(&mut self.buffer);
        self.extra.clear();
        self.cursor = 0;
        self.anchor = None;
        self.hist_idx = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        // users expect to type immediately after running a command
        if let Some(v) = &mut self.vim {
            v.reset_insert();
        }
        if !cmd.trim().is_empty() && self.history.last() != Some(&cmd) {
            std::sync::Arc::make_mut(&mut self.history).push(cmd.clone());
        }
        cmd
    }

    pub fn history_up(&mut self) {
        self.anchor = None;
        self.extra.clear();
        if self.history.is_empty() {
            return;
        }
        self.push_undo();
        let idx = match self.hist_idx {
            None => {
                self.saved = self.buffer.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.hist_idx = Some(idx);
        self.buffer = self.history[idx].clone();
        self.cursor = self.buffer.len();
    }

    pub fn history_down(&mut self) {
        self.anchor = None;
        self.extra.clear();
        if self.hist_idx.is_some() {
            self.push_undo();
        }
        match self.hist_idx {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.hist_idx = Some(i + 1);
                self.buffer = self.history[i + 1].clone();
                self.cursor = self.buffer.len();
            }
            Some(_) => {
                self.hist_idx = None;
                self.buffer = self.saved.clone();
                self.cursor = self.buffer.len();
            }
        }
    }
}

/// The shell's size for the window's own grid `cols` × `rows` and a phone's
/// `fit` (columns, rows, when it was last sent): the smaller of the two in
/// each direction while its lease of [`rift_remote::FIT_LEASE`] lasts.
pub fn fitted_grid(
    fit: Option<(u16, u16, std::time::Instant)>,
    cols: usize,
    rows: usize,
) -> (usize, usize) {
    match fit {
        Some((c, r, at)) if at.elapsed() < rift_remote::FIT_LEASE => {
            (cols.min(c as usize), rows.min(r as usize))
        }
        _ => (cols, rows),
    }
}

/// One live shell + its view state.
pub struct PaneState {
    pub session: Session,
    pub view_offset: usize,
    /// Paired phones' own scrolled view of the pane (their `scroll`, their
    /// styled `screen`), apart from `view_offset`: the desktop's view never
    /// moves for a phone.
    pub remote_view: crate::views::terminal::RemoteView,
    pub last_display_len: usize,
    /// The last frame's grid cursor shape when it drew one, and how many
    /// cells it drew as geometry (box drawing, blocks, Powerline; dump).
    pub drawn_cursor: Option<rift_core::CursorStyle>,
    pub art_cells: usize,
    pub editor: EditorState,
    pub git: Option<GitInfo>,
    pub last_git_cwd: Option<String>,
    /// Canonical directory for color rules, resolved only when the cwd changes.
    pub color_cwd: std::cell::RefCell<Option<(String, String)>>,
    /// The last block when git was last asked: the next prompt after a
    /// command (a new block) asks again.
    pub last_git_block: rift_core::BlockId,
    /// The checked-out branch's pull request (GitHub PR chip).
    pub pr: Option<rift_core::github::PrInfo>,
    /// The Docker Compose project around the cwd (Docker chip).
    pub docker: Option<rift_core::docker::ComposeInfo>,
    /// Exit code of the last finished command (recolors the ❯).
    pub last_exit: Option<i32>,
    /// Frame cache for mouse → grid mapping (see terminal::RowKind).
    pub rows: Vec<crate::views::terminal::RowInfo>,
    pub last_grid: Option<crate::views::terminal::GridLayout>,
    /// The space the grid gets, in points (whole cells fit in it); window
    /// resizes in characters size the window around it.
    pub text_area: (f32, f32),
    /// The window's inner size (points) when `text_area` was laid out: the
    /// chrome around the grid is their difference, whatever the window does
    /// before the next frame.
    pub layout_window: (f32, f32),
    pub alt_screen: bool,
    /// Mirrors `term.selection.is_some()` to avoid locking on every key.
    pub has_selection: bool,
    /// Visible URL spans from the last frame: (rect, url).
    pub url_hits: Vec<([f32; 4], String)>,
    /// The sticky command header shown at the top last frame: (rect, block).
    pub sticky_hit: Option<([f32; 4], rift_core::BlockId)>,
    /// This pane's sticky command header is minimized to a small tab at
    /// the top right (`toggle_sticky_header`; a click on it restores).
    pub sticky_minimized: bool,
    /// Keyboard-selected block (⌘↑/⌘↓), target of ⇧⌘C / ⌘I / ⌃M / ⌘B.
    pub selected_block: Option<rift_core::BlockId>,
    /// Multi-block selection (⌘-click, ⇧-click, ⇧↑ ⇧↓, ⌘A): every selected
    /// block, `selected_block` among them; it counts only while it holds
    /// `selected_block` (see [`PaneState::selected_blocks`]).
    pub block_selection: Vec<rift_core::BlockId>,
    /// Per-block output filters (Warp block filter,
    /// `crate::views::block_filter`).
    pub filter: crate::views::block_filter::BlockFilters,
    /// Local fix for the last failed command (shown in the empty editor).
    pub correction: Option<String>,
    /// The offer to load Rift's hooks in the subshell its command started.
    pub subshell: Option<crate::app::SubshellOffer>,
    /// Newest finished block already checked for a correction.
    pub correction_checked: Option<rift_core::BlockId>,
    /// File id of this pane's saved block transcript (`save_transcripts`).
    pub transcript_id: String,
    /// When the app first saw the shell's output (boot placeholder grace).
    pub first_output: Option<std::time::Instant>,
    /// Running block whose password prompt already raised a notification.
    pub password_block: Option<rift_core::BlockId>,
    /// This pane's own window title (OSC 0/2): its title bar in a split.
    pub title: String,
    /// The size a paired phone holds this pane to (columns, rows) and when
    /// it last said so: a lease of [`rift_remote::FIT_LEASE`].
    pub fit: Option<(u16, u16, std::time::Instant)>,
    /// The window has focus: someone is using it, so it keeps its own size
    /// and the fit waits until they leave.
    pub fit_paused: bool,
    /// The window's own grid at the last layout (columns, rows, cell size
    /// in px), before any `fit`.
    pub base_grid: Option<(usize, usize, (u16, u16))>,
    /// Last layout's grid with the command editor closed, before any `fit`.
    pub program_grid: Option<(usize, usize, (u16, u16))>,
    /// What the pane was opened with (profile name, own shell and args):
    /// kept by duplicate, save-layout and restore.
    pub launch: rift_core::profiles::PaneLaunch,
    /// The pane's own colour scheme (its launch's `theme`, else its
    /// profile's): the theme's name and the loaded theme its terminal
    /// content renders in. `None`: the window theme.
    pub theme: Option<(String, Box<rift_core::Theme>)>,
    /// The overlay scrollbar's state (`scrollbar`, `views::scrollbar`).
    pub scrollbar: crate::views::scrollbar::PaneScrollbar,
    /// Masked secrets on screen and the ones revealed (`mask_secrets`).
    pub secrets: crate::secret_mask::PaneSecrets,
    /// The program the pane started (its integration, completions and
    /// labels follow its basename); empty: not known (the global shell).
    pub program: String,
    /// The coding agent this pane runs, or ran until its finish is seen.
    pub agent_run: Option<crate::agent_status::AgentRun>,
    /// Its status at the last frame (tab strip, rail, `dump`).
    pub agent_status: Option<crate::agent_status::AgentStatus>,
    /// Output arrived while its tab was not viewed (`TabState::mark_activity`).
    pub activity: bool,
    /// Rich input over the running program (`crate::composer`).
    pub composer: crate::composer::Composer,
    /// Its program exited and the pane stays (`close_on_exit`): the exit
    /// code, -1 when none is known. The Restart bar shows.
    pub exited: Option<i32>,
    /// Its shell session in Rift's command history (next-command ghost text).
    pub history: HistorySession,
}

/// A pane's shell session in Rift's command history: what the next command
/// it finishes is recorded under, and the last one it finished.
#[derive(Debug, Clone)]
pub struct HistorySession {
    /// New for every shell the pane starts (a restarted or restored pane
    /// gets a new one), so commands follow each other only within one shell.
    pub id: String,
    /// The place of the next command recorded.
    pub seq: u64,
    /// The last command finished in this run from the editor: its command
    /// line, directory and exit code. A restored block never counts.
    pub last: Option<(String, Option<String>, Option<i32>)>,
}

impl Default for HistorySession {
    fn default() -> Self {
        HistorySession {
            id: rift_core::transcript::new_id(),
            seq: 0,
            last: None,
        }
    }
}

impl PaneState {
    /// The program's title as a name: a coding agent's status mark (`✳`,
    /// `◑`, a braille spinner frame) is the tab's status, not part of it.
    /// Empty when the program set no title.
    pub fn program_title(&self) -> &str {
        rift_core::notifications::agent_session_title(&self.title)
    }

    /// The selected blocks, oldest first: the multi-block selection while it
    /// holds `selected_block`, else `selected_block` alone.
    pub fn selected_blocks(&self) -> Vec<rift_core::BlockId> {
        let Some(sel) = self.selected_block else {
            return Vec::new();
        };
        if self.block_selection.contains(&sel) {
            let mut all = self.block_selection.clone();
            all.sort_unstable();
            all.dedup();
            all
        } else {
            vec![sel]
        }
    }

    /// Drop a multi-block selection that no longer holds `selected_block`
    /// (the selection was cleared or moved by a single-block path).
    pub fn settle_block_selection(&mut self) {
        if self
            .selected_block
            .is_none_or(|s| !self.block_selection.contains(&s))
        {
            self.block_selection.clear();
        }
    }

    pub fn new(session: Session) -> Self {
        PaneState {
            session,
            view_offset: 0,
            remote_view: Default::default(),
            last_display_len: 0,
            drawn_cursor: None,
            art_cells: 0,
            editor: EditorState::default(),
            git: None,
            last_git_cwd: None,
            color_cwd: Default::default(),
            last_git_block: 0,
            pr: None,
            docker: None,
            last_exit: None,
            rows: Vec::new(),
            last_grid: None,
            text_area: (0.0, 0.0),
            layout_window: (0.0, 0.0),
            alt_screen: false,
            has_selection: false,
            url_hits: Vec::new(),
            sticky_hit: None,
            sticky_minimized: false,
            selected_block: None,
            block_selection: Vec::new(),
            filter: Default::default(),
            correction: None,
            subshell: None,
            correction_checked: None,
            transcript_id: rift_core::transcript::new_id(),
            first_output: None,
            password_block: None,
            title: String::new(),
            fit: None,
            fit_paused: false,
            base_grid: None,
            program_grid: None,
            launch: Default::default(),
            theme: None,
            scrollbar: Default::default(),
            secrets: Default::default(),
            program: String::new(),
            agent_run: None,
            agent_status: None,
            activity: false,
            composer: Default::default(),
            exited: None,
            history: Default::default(),
        }
    }

    /// Follow this pane's coding agent one frame (`agent_status::step`):
    /// `open_request` = its request waits on you, `viewed` = on screen in
    /// the focused window.
    pub fn sync_agent(&mut self, open_request: bool, viewed: bool) {
        use crate::agent_status::{BlockView, Observed, step};
        use rift_core::notifications::{Agent, agent_idle};
        let view = |b: &rift_core::Block| BlockView {
            id: b.id,
            running: matches!(b.state, rift_core::BlockState::Running { .. }),
            exit: match b.state {
                rift_core::BlockState::Done { exit, .. } => exit,
                _ => None,
            },
            agent: Agent::from_command(&b.command),
        };
        let (last, tracked) = {
            let blocks = self.session.blocks.lock().unwrap();
            let last = blocks.last().map(view);
            let tracked = self
                .agent_run
                .and_then(|r| r.block)
                .and_then(|id| blocks.iter().rev().find(|b| b.id == id).map(view));
            (last, tracked)
        };
        // the agent `step` follows: a new agent command, else the run's
        // (Codex's idle title is its bare project name, `agent_idle`)
        let agent = last
            .filter(|b| b.running)
            .and_then(|b| b.agent)
            .or(self.agent_run.map(|r| r.agent));
        let (run, status) = step(
            self.agent_run,
            Observed {
                last,
                tracked,
                open_request,
                idle_title: agent_idle(agent, &self.title) && !self.signals_work(),
                viewed,
            },
        );
        self.agent_run = run;
        self.agent_status = status;
    }

    /// The shell's size: the window's own grid, or the phone's where that is
    /// smaller. The one place the two meet (layout calls it every frame).
    pub fn fitted(&self, cols: usize, rows: usize) -> (usize, usize) {
        if self.fit_paused {
            return (cols, rows);
        }
        fitted_grid(self.fit, cols, rows)
    }

    /// The size a capture for the phone is taken at: while the window in use
    /// holds off the phone's fit and that fit would be smaller than the pane.
    pub fn capture_size(&self) -> Option<rift_pty::GridSize> {
        if !self.fit_paused {
            return None;
        }
        let (own_cols, own_rows, _) = self.base_grid?;
        let (cols, rows) = fitted_grid(self.fit, own_cols, own_rows);
        ((cols, rows) != (own_cols, own_rows)).then_some(rift_pty::GridSize { cols, rows })
    }

    /// Size the shell now from the last layout's grid and the phone's fit:
    /// for a pane that is not drawn (another tab), which layout does not
    /// reach. Returns whether it is known how (a pane never laid out waits
    /// for its first layout).
    pub fn apply_fit(&mut self) -> bool {
        let Some((cols, rows, cell)) = self.base_grid else {
            return false;
        };
        let (cols, rows) = self.fitted(cols, rows);
        self.session.resize(rift_pty::GridSize { cols, rows }, cell);
        true
    }

    /// Publish the program's size before its input can start a process.
    pub fn prepare_program_grid(&mut self) {
        if let Some((cols, rows, cell)) = self.program_grid {
            let (cols, rows) = self.fitted(cols, rows);
            self.session.resize(rift_pty::GridSize { cols, rows }, cell);
        }
    }

    /// A command runs in this pane and is working (its tab and title bar
    /// spin). A coding agent that sits at its own prompt waiting for you
    /// still runs as a command, but is not working: no spinner (Warp).
    /// No standard says a program is busy, so a coding agent or a
    /// full-screen program (vim, htop, OpenCode) works only while it says
    /// so ([`Self::signals_work`]).
    pub fn running(&self) -> bool {
        if self.exited.is_some() {
            return false;
        }
        use rift_core::notifications::{Agent, agent_title_idle};
        use rift_pty::alacritty_terminal::term::TermMode;
        let agent = {
            let blocks = self.session.blocks.lock().unwrap();
            if !blocks.any_running() {
                // an agent whose command block is gone (reattached after a
                // restart, the replay too short to hold its start): its
                // working title and a foreground job still say it runs
                return rift_core::notifications::agent_title_working(&self.title)
                    && self.session.has_foreground_job();
            }
            // the typed command names the agent (no process lookup: this
            // runs every frame)
            blocks
                .last()
                .filter(|b| matches!(b.state, rift_core::BlockState::Running { .. }))
                .and_then(|b| Agent::from_command(&b.command))
        };
        // the blocks lock is released: the reader takes the terminal first
        let full_screen = self
            .session
            .term
            .lock()
            .mode()
            .contains(TermMode::ALT_SCREEN);
        if agent.is_some() || full_screen {
            return self.signals_work();
        }
        !agent_title_idle(&self.title)
    }

    /// The program says it works: a coding agent's working title
    /// (`agent_title_working`) or an OSC 9;4 progress report that runs
    /// (normal or indeterminate; not paused or failed).
    fn signals_work(&self) -> bool {
        use rift_core::ProgressState;
        rift_core::notifications::agent_title_working(&self.title)
            || self.session.progress.lock().unwrap().is_some_and(|r| {
                matches!(
                    r.progress.state,
                    ProgressState::Normal | ProgressState::Indeterminate
                )
            })
    }

    /// Any command runs in this pane, working or not (the tab's Elapsed
    /// clock counts an agent at its prompt until it exits).
    pub fn command_running(&self) -> bool {
        // its program ended: nothing runs, whatever its last block says
        self.exited.is_none()
            && (self.session.blocks.lock().unwrap().any_running() || self.running())
    }

    /// Clear this pane's terminal selection (cheap no-op when none).
    pub fn clear_selection(&mut self) {
        if self.has_selection {
            self.session.term.lock().selection = None;
            self.has_selection = false;
        }
    }
}

/// One tab: a tree of panes.
pub struct TabState {
    pub root: PaneNode,
    pub active_pane: PaneId,
    pub panes: Vec<PaneState>,
    pub title: String,
    /// Background tab produced output since last viewed.
    pub activity: bool,
    /// ⇧⌘⏎ zoom: this pane temporarily gets the whole tab rect.
    pub zoomed: Option<PaneId>,
    /// User-set name (rename); wins over the OSC title and the cwd.
    pub custom_title: Option<String>,
    /// The tab's own colour (a named one or any hex); a
    /// `[[tab_color_rules]]` colour shows only while this is none.
    pub color: Option<rift_core::tab_color::TabColor>,
    /// ⌥⌘I: editor submissions and raw keys go to every pane of the tab.
    pub sync_inputs: bool,
    /// When the tab's first command was submitted (the Elapsed chip); wall
    /// clock so it survives a restart.
    pub started: Option<std::time::SystemTime>,
    /// Time a command ran in the tab (`elapsed_counts = "running"`); always
    /// tracked, shown only in that mode.
    pub clock: rift_core::tab_clock::TabClock,
    /// When the tab was last the active one (a counter; 0: never), for
    /// ⌃Tab's most-recently-used order.
    pub last_used: u64,
    /// Pinned: kept first in the strip and the rail, compact (`tab_groups`).
    pub pinned: bool,
    /// The tab group it belongs to (`TabGroup::id`).
    pub group: Option<u64>,
    /// Picked with ⌘-click / ⇧-click for "Group tabs".
    pub picked: bool,
}

/// Tab color names, in menu order (Warp's palette); rendered from theme ANSI
/// colors so they follow the theme.
pub const TAB_COLORS: [&str; 6] = rift_core::tab_color::NAMES;

impl TabState {
    pub fn new(pane: PaneState) -> Self {
        let id = pane.session.id;
        TabState {
            root: PaneNode::Leaf(id),
            active_pane: id,
            panes: vec![pane],
            title: String::new(),
            activity: false,
            zoomed: None,
            custom_title: None,
            color: None,
            sync_inputs: false,
            started: None,
            clock: Default::default(),
            last_used: 0,
            pinned: false,
            group: None,
            picked: false,
        }
    }

    /// Start the Elapsed clock at the tab's first non-empty command.
    pub fn start_clock(&mut self, cmd: &str) {
        if self.started.is_none() && !cmd.trim().is_empty() {
            self.started = Some(std::time::SystemTime::now());
        }
    }

    /// Follow the panes: the tab counts while any of them runs a command (an
    /// agent at its own prompt included: it runs until it exits).
    pub fn sync_clock(&mut self, now: std::time::Instant) {
        let running: Vec<bool> = self.panes.iter().map(PaneState::command_running).collect();
        self.clock.sync(running, now);
    }

    /// The Elapsed chip: time since the tab's first command, or the time a
    /// command ran in it (None before there is any).
    pub fn elapsed(
        &self,
        mode: rift_core::ElapsedCounts,
        now: std::time::Instant,
    ) -> Option<crate::views::prompt_chips::Elapsed> {
        use crate::views::prompt_chips::Elapsed;
        match mode {
            rift_core::ElapsedCounts::Always => {
                let time = self.started?.elapsed().unwrap_or_default();
                Some(Elapsed {
                    time,
                    counting: true,
                })
            }
            rift_core::ElapsedCounts::Running => {
                let time = self.clock.total(now);
                (self.started.is_some() || !time.is_zero()).then_some(Elapsed {
                    time,
                    counting: self.clock.counting(),
                })
            }
        }
    }

    /// Reset button of the Elapsed card: both counts restart from now (so a
    /// later switch of `elapsed_counts` shows no stale time). The chip stays,
    /// at 0m, so its card can be used again; a tab that shows no chip yet
    /// stays without one.
    pub fn reset_clock(&mut self, mode: rift_core::ElapsedCounts, now: std::time::Instant) {
        let shown = self.elapsed(mode, now).is_some();
        self.clock.reset(now);
        if shown {
            self.started = Some(std::time::SystemTime::now());
        }
    }

    pub fn pane(&self, id: PaneId) -> Option<&PaneState> {
        self.panes.iter().find(|p| p.session.id == id)
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut PaneState> {
        self.panes.iter_mut().find(|p| p.session.id == id)
    }

    pub fn active(&self) -> Option<&PaneState> {
        self.pane(self.active_pane)
    }

    pub fn active_mut(&mut self) -> Option<&mut PaneState> {
        self.pane_mut(self.active_pane)
    }

    pub fn is_split(&self) -> bool {
        self.panes.len() > 1
    }

    /// Split chrome (gutters, compact editor, pane borders) applies only
    /// when split AND not zoomed — a zoomed pane renders like a single pane.
    /// The viewer pane counts as a leaf here: it lays out like any split.
    pub fn effective_split(&self) -> bool {
        self.root.is_split() && !(self.is_split() && self.zoomed.is_some())
    }

    /// The tab's name: the one it was given (rename), else its focused
    /// pane's folder (`~` for home), else the program's title until a
    /// directory is known. The strip and the rail add the parent when two
    /// tabs share a folder name (`tab_status::titles`).
    pub fn display_title(&self) -> String {
        if let Some(t) = self.custom_name() {
            return t.to_string();
        }
        match self.focused_cwd() {
            Some(cwd) => crate::tab_status::folder(&cwd).to_string(),
            None if !rift_core::notifications::agent_session_title(&self.title).is_empty() => {
                rift_core::notifications::agent_session_title(&self.title).to_string()
            }
            None => "shell".into(),
        }
    }

    /// The name the tab was given (rename), if any.
    pub fn custom_name(&self) -> Option<&str> {
        self.custom_title.as_deref().filter(|t| !t.is_empty())
    }

    /// The focused pane's directory, `~`-shortened.
    pub fn focused_cwd(&self) -> Option<String> {
        self.active()
            .and_then(|p| p.session.cwd.lock().unwrap().clone())
            .map(|p| crate::views::editor::shorten_home(&p))
    }

    /// Output since last viewed: the tab's and each pane's flag (the strip,
    /// the rail and the status ranking read them).
    pub fn mark_activity(&mut self, pane: PaneId) {
        self.activity = true;
        if let Some(p) = self.pane_mut(pane) {
            p.activity = true;
        }
    }

    /// The tab was viewed: its output is seen.
    pub fn clear_activity(&mut self) {
        self.activity = false;
        for p in &mut self.panes {
            p.activity = false;
        }
    }
}

/// Whether `before` (a command line up to the cursor) ends inside a quoted
/// string, the way the shell reads it: `'…'` literal, `"…"` with `\`
/// escapes, and a `\` outside quotes escaping the next char.
fn quote_open_at(before: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut chars = before.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (Some('"'), '\\') | (None, '\\') => {
                chars.next();
            }
            (Some('"'), '"') => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, _) => {}
        }
    }
    quote.is_some()
}

/// Start of the extended grapheme cluster ending at or before byte `i`.
pub(crate) fn grapheme_prev(s: &str, i: usize) -> usize {
    // GraphemeCursor panics on an offset inside a char
    let i = s.floor_char_boundary(i);
    let mut c = GraphemeCursor::new(i, s.len(), true);
    // the chunk is the whole string, so no extra context is ever requested
    c.prev_boundary(s, 0).ok().flatten().unwrap_or(0)
}

/// Backspace in a one-line field: drop the last whole grapheme cluster (an
/// emoji with its modifiers, a letter with its combining marks). False when
/// `s` was empty.
pub fn pop_grapheme(s: &mut String) -> bool {
    if s.is_empty() {
        return false;
    }
    s.truncate(grapheme_prev(s, s.len()));
    true
}

/// End of the extended grapheme cluster starting at or after byte `i`.
fn grapheme_next(s: &str, i: usize) -> usize {
    let i = s.ceil_char_boundary(i);
    let mut c = GraphemeCursor::new(i, s.len(), true);
    c.next_boundary(s, 0).ok().flatten().unwrap_or(s.len())
}

fn is_grapheme_boundary(s: &str, i: usize) -> bool {
    s.is_char_boundary(i)
        && GraphemeCursor::new(i, s.len(), true)
            .is_boundary(s, 0)
            .unwrap_or(true)
}

/// `i` if it is a cluster boundary, else the start of the cluster around it.
fn grapheme_floor(s: &str, i: usize) -> usize {
    let i = s.floor_char_boundary(i);
    if is_grapheme_boundary(s, i) {
        i
    } else {
        grapheme_prev(s, i)
    }
}

/// `i` if it is a cluster boundary, else the end of the cluster around it.
fn grapheme_ceil(s: &str, i: usize) -> usize {
    let i = s.ceil_char_boundary(i);
    if is_grapheme_boundary(s, i) {
        i
    } else {
        grapheme_next(s, i)
    }
}

#[cfg(test)]
mod tests {
    use super::{EditorState, quote_open_at};

    #[test]
    fn word_motion_and_kills() {
        let mut e = EditorState::default();
        e.insert("git  commit -m msg");
        // word_left from end → start of "msg"
        e.word_left();
        assert_eq!(e.cursor, 15);
        e.word_left();
        assert_eq!(e.cursor, 12); // "-m"
        e.word_left();
        assert_eq!(e.cursor, 5); // "commit" (double space skipped)
        e.word_left();
        assert_eq!(e.cursor, 0);
        e.word_left();
        assert_eq!(e.cursor, 0); // clamped
        // word_right walks forward
        e.word_right();
        assert_eq!(e.cursor, 3); // end of "git"
        e.word_right();
        assert_eq!(e.cursor, 11); // end of "commit"
        // kill_to_end from mid-buffer
        e.kill_to_end();
        assert_eq!(e.buffer, "git  commit");
        // delete_word_forward at end of buffer is a no-op
        e.end();
        e.delete_word_forward();
        assert_eq!(e.buffer, "git  commit");
        // kill_to_start
        e.cursor = 5;
        e.kill_to_start();
        assert_eq!(e.buffer, "commit");
        assert_eq!(e.cursor, 0);
        // delete_word_forward eats the word ahead
        e.delete_word_forward();
        assert_eq!(e.buffer, "");
    }

    /// ⌘← / ⌘→ act on the cursor's line of a multi-line command; ⌘↑ / ⌘↓
    /// reach its start and end only while there are lines that way.
    /// ⌘⌫ and ⌃K act on the cursor's line of a multi-line command; at the
    /// line's edge they take the newline (readline / macOS).
    #[test]
    fn line_kills_stay_on_the_cursor_line() {
        let mut e = EditorState::default();
        e.insert("echo one\necho two\necho three");
        // cursor after "echo t" on line 2
        e.cursor = "echo one\necho t".len();
        e.kill_to_start();
        assert_eq!(e.buffer, "echo one\nwo\necho three");
        assert_eq!(e.cursor, "echo one\n".len());
        // at column 0: the newline before
        e.kill_to_start();
        assert_eq!(e.buffer, "echo onewo\necho three");
        assert_eq!(e.cursor, "echo one".len());
        // ⌃K to the line end, not the command end
        e.kill_to_end();
        assert_eq!(e.buffer, "echo one\necho three");
        // at the line end: the newline after
        e.kill_to_end();
        assert_eq!(e.buffer, "echo oneecho three");
        // last line, at its end: nothing
        e.end();
        e.kill_to_end();
        assert_eq!(e.buffer, "echo oneecho three");
    }

    /// vim folds a change's snapshots by the depth it marked; at the undo
    /// cap the front evicts, and one undo must still revert the change.
    #[test]
    fn squash_undo_holds_at_the_undo_cap() {
        let mut e = EditorState::default();
        for _ in 0..super::UNDO_CAP + 5 {
            e.insert("a");
        }
        let before = e.buffer.clone();
        let mark = e.undo_len();
        let cursor = e.cursor;
        for _ in 0..3 {
            e.insert("b");
        }
        e.squash_undo(mark, cursor);
        e.undo();
        assert_eq!(e.buffer, before);
        assert_eq!(e.cursor, cursor);
    }

    #[test]
    fn every_caret_types_and_deletes_as_one_undo_step() {
        let mut e = EditorState {
            buffer: "a1 a2 a3".into(),
            cursor: 2,
            ..Default::default()
        };
        e.add_caret_at(5);
        e.add_caret_at(8);
        e.at_each_caret(|e| e.insert("x"));
        assert_eq!(e.buffer, "a1x a2x a3x");
        assert_eq!(
            (e.cursor, e.extra.clone()),
            (3, vec![(7, None), (11, None)])
        );
        e.at_each_caret(|e| e.backspace());
        e.at_each_caret(|e| e.backspace());
        assert_eq!(e.buffer, "a a a");
        e.at_each_caret(|e| e.mv_left());
        e.at_each_caret(|e| e.insert("b"));
        assert_eq!(e.buffer, "ba ba ba");
        // one undo step per multi-caret edit, back to one caret
        e.undo();
        assert_eq!(e.buffer, "a a a");
        assert!(!e.has_extra());
    }

    #[test]
    fn the_ghost_text_follows_the_end_or_vims_normal_block_on_the_last_character() {
        let mut e = EditorState {
            buffer: "echo é".into(),
            cursor: 7,
            ..Default::default()
        };
        assert!(e.ghost_at_end());
        // mid-line: no
        e.cursor = 5;
        assert!(!e.ghost_at_end());
        // vim Insert on the last character: no; Normal rests there: yes
        e.set_vim_enabled(true);
        assert!(!e.ghost_at_end());
        e.vim.as_mut().unwrap().mode = crate::vim::VimMode::Normal;
        assert!(e.ghost_at_end());
        e.cursor = 4;
        assert!(!e.ghost_at_end());
    }

    #[test]
    fn a_delete_across_another_caret_keeps_it_on_the_text() {
        // ⌥⌫ at the end of "éé" deletes the word the other caret is inside
        let mut e = EditorState {
            buffer: "ls ééé x".into(),
            cursor: 9,
            ..Default::default()
        };
        e.add_caret_at(5);
        e.at_each_caret(|e| e.delete_word_back());
        assert_eq!(e.buffer, "ls  x");
        assert!(e.buffer.is_char_boundary(e.cursor));
        assert!(e.extra.iter().all(|c| e.buffer.is_char_boundary(c.0)));
        e.at_each_caret(|e| e.insert("é"));
        assert!(e.buffer.starts_with("ls é"));
        // ⌘-click on a caret takes it away
        let mut f = EditorState {
            buffer: "abc".into(),
            cursor: 0,
            ..Default::default()
        };
        f.add_caret_at(2);
        f.add_caret_at(2);
        assert!(!f.has_extra());
        // the next occurrence wraps past a taken one
        let mut g = EditorState {
            buffer: "x a a a".into(),
            cursor: 5,
            anchor: Some(4),
            ..Default::default()
        };
        g.add_next_occurrence();
        g.add_next_occurrence();
        assert_eq!(g.selections().len(), 3);
    }

    #[test]
    fn next_occurrence_renames_a_word_everywhere() {
        let mut e = EditorState {
            buffer: "cp foo.txt foo.bak && rm foo.txt".into(),
            cursor: 4,
            ..Default::default()
        };
        e.add_next_occurrence();
        assert_eq!(e.selected_text(), Some("foo"));
        e.add_next_occurrence();
        e.add_next_occurrence();
        assert_eq!(e.selections().len(), 3);
        // a fourth press finds no new one
        e.add_next_occurrence();
        assert_eq!(e.selections().len(), 3);
        e.at_each_caret(|e| e.insert("bar"));
        assert_eq!(e.buffer, "cp bar.txt bar.bak && rm bar.txt");
        assert_eq!(e.extra.len(), 2);
    }

    #[test]
    fn next_occurrence_keeps_combining_marks_with_the_word() {
        let mut e = EditorState {
            buffer: "cafe\u{301} cafe\u{301}".into(),
            cursor: 0,
            ..Default::default()
        };
        e.add_next_occurrence();
        assert_eq!(e.selected_text(), Some("cafe\u{301}"));
        e.add_next_occurrence();
        assert_eq!(e.selections().len(), 2);
        e.at_each_caret(|e| e.insert("tea"));
        assert_eq!(e.buffer, "tea tea");
    }

    #[test]
    fn pointer_carets_and_selections_never_split_a_grapheme() {
        for cluster in ["a\u{301}", "👩‍💻", "🇵🇭"] {
            let inside = cluster.char_indices().nth(1).unwrap().0;
            let mut e = EditorState {
                buffer: format!("{cluster}z"),
                cursor: cluster.len() + 1,
                ..Default::default()
            };
            e.click_to(inside, false);
            assert_eq!(e.cursor, 0, "click in {cluster}");
            e.click_to(cluster.len(), true);
            assert_eq!(e.selected_text(), Some(cluster));
            e.click_to(e.buffer.len(), false);
            e.add_caret_at(inside);
            assert_eq!(e.extra, vec![(0, None)], "extra caret in {cluster}");
        }
    }

    #[test]
    fn next_occurrence_skips_partial_grapheme_matches() {
        let mut e = EditorState {
            buffer: "a a\u{301} a".into(),
            cursor: 0,
            ..Default::default()
        };
        e.add_next_occurrence();
        e.add_next_occurrence();
        assert_eq!(e.selections(), vec![0..1, 6..7]);
        e.add_next_occurrence();
        assert_eq!(e.selections(), vec![0..1, 6..7]);
        e.at_each_caret(|e| e.insert("X"));
        assert_eq!(e.buffer, "X a\u{301} X");
    }

    #[test]
    fn carets_above_and_below_keep_the_column_and_merge() {
        let mut e = EditorState {
            buffer: "echo one\nls\necho three".into(),
            cursor: 5,
            ..Default::default()
        };
        e.add_caret_vertical(true);
        e.add_caret_vertical(true);
        // the short middle line takes its end; the next caret goes from the
        // outermost one, at its column
        assert_eq!(e.extra, vec![(11, None), (14, None)]);
        e.add_caret_vertical(true);
        assert_eq!(e.extra.len(), 2, "no line below the last");
        e.at_each_caret(|e| e.insert(">"));
        assert_eq!(e.buffer, "echo >one\nls>\nec>ho three");
        // moving every caret to its line start, then two more lefts: the
        // carets that meet merge
        e.at_each_caret(|e| e.mv_doc_start());
        assert!(!e.has_extra());
    }

    #[test]
    fn paste_gives_each_caret_its_line() {
        let mut e = EditorState {
            buffer: "a b c".into(),
            cursor: 1,
            ..Default::default()
        };
        e.add_caret_at(3);
        e.add_caret_at(5);
        e.paste_at_carets("1\n2\n3");
        assert_eq!(e.buffer, "a1 b2 c3");
        e.paste_at_carets("-");
        assert_eq!(e.buffer, "a1- b2- c3-");
        e.collapse_carets();
        assert!(!e.has_extra());
    }

    #[test]
    fn up_and_down_move_between_lines_until_the_ends() {
        let mut e = EditorState {
            buffer: "echo one\nab\necho three".into(),
            cursor: 15, // "ech|o three"
            ..Default::default()
        };
        assert!(e.line_vertical(false));
        assert_eq!(e.cursor, 11, "the shorter line's end");
        assert!(e.line_vertical(false));
        assert_eq!(e.cursor, 2, "back at column 2 of the first line");
        // the first line: the key walks history instead
        assert!(!e.line_vertical(false));
        assert_eq!(e.cursor, 2);
        assert!(e.line_vertical(true));
        assert!(e.line_vertical(true));
        assert_eq!(e.cursor, 14);
        assert!(!e.line_vertical(true));
    }

    #[test]
    fn vertical_moves_and_carets_keep_grapheme_clusters_whole() {
        for cluster in ["a\u{301}", "👩‍💻", "🇵🇭"] {
            let mut e = EditorState {
                buffer: format!("xy\n{cluster}z"),
                cursor: 1,
                ..Default::default()
            };
            let after_cluster = 3 + cluster.len();
            assert!(e.line_vertical(true));
            assert_eq!(e.cursor, after_cluster, "after {cluster}");
            assert!(e.line_vertical(false));
            assert_eq!(e.cursor, 1, "back at the first column");
            e.add_caret_vertical(true);
            assert_eq!(e.extra, vec![(after_cluster, None)]);
            e.at_each_caret(|e| e.backspace());
            assert_eq!(e.buffer, "y\nz", "delete whole {cluster}");
        }
    }

    #[test]
    fn line_and_command_bounds() {
        let mut e = EditorState {
            buffer: "echo one\necho two\necho three".into(),
            cursor: 12,
            ..Default::default()
        };
        e.home();
        assert_eq!(e.cursor, 9);
        e.end();
        assert_eq!(e.cursor, 17);
        assert!(e.below_first_line() && e.above_last_line());
        e.mv_doc_start();
        assert_eq!(e.cursor, 0);
        assert!(!e.below_first_line() && e.above_last_line());
        e.end();
        assert_eq!(e.cursor, 8);
        e.mv_doc_end();
        assert_eq!(e.cursor, 28);
        assert!(e.below_first_line() && !e.above_last_line());
        e.home();
        assert_eq!(e.cursor, 18);
        // one line: the whole command, and no lines either way
        let mut one = EditorState {
            buffer: "ls -la".into(),
            cursor: 3,
            ..Default::default()
        };
        one.home();
        assert_eq!(one.cursor, 0);
        one.end();
        assert_eq!(one.cursor, 6);
        assert!(!one.below_first_line() && !one.above_last_line());
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut e = EditorState::default();
        e.insert("echo ");
        e.insert("hello");
        assert_eq!(e.buffer, "echo hello");
        e.undo();
        assert_eq!(e.buffer, "echo ");
        e.undo();
        assert_eq!(e.buffer, "");
        e.undo(); // empty stack: no-op
        assert_eq!(e.buffer, "");
        e.redo();
        assert_eq!(e.buffer, "echo ");
        e.redo();
        assert_eq!(e.buffer, "echo hello");
        e.redo(); // empty stack: no-op
        assert_eq!(e.buffer, "echo hello");
        // undo then a fresh edit clears the redo stack
        e.undo();
        e.insert("X");
        assert_eq!(e.buffer, "echo X");
        e.redo();
        assert_eq!(e.buffer, "echo X");
        // submit clears history
        e.submit();
        e.undo();
        assert_eq!(e.buffer, "");
    }

    #[test]
    fn autocomplete_pairs_insert_skip_and_delete() {
        let mut e = EditorState::default();
        e.insert_with_pairs("(");
        assert_eq!(e.buffer, "()");
        assert_eq!(e.cursor, 1);

        e.insert_with_pairs("x");
        e.insert_with_pairs(")");
        assert_eq!(e.buffer, "(x)");
        assert_eq!(e.cursor, 3);

        e.home();
        e.insert_with_pairs("[");
        assert_eq!(e.buffer, "[](x)");
        assert_eq!(e.cursor, 1);
        e.backspace_with_pairs();
        assert_eq!(e.buffer, "(x)");
        assert_eq!(e.cursor, 0);
    }

    #[test]
    fn autocomplete_quotes_stay_literal_inside_words() {
        let mut e = EditorState::default();
        e.insert("don");
        e.insert_with_pairs("'");
        e.insert("t");
        assert_eq!(e.buffer, "don't");

        e.clear();
        e.insert_with_pairs("\"");
        assert_eq!(e.buffer, "\"\"");
        assert_eq!(e.cursor, 1);
    }

    #[test]
    fn nothing_pairs_inside_a_quoted_string() {
        let mut e = EditorState::default();
        for c in "printf '\\033[?5l' \"(x\" [y]".chars() {
            e.insert_with_pairs(&c.to_string());
        }
        assert_eq!(e.buffer, "printf '\\033[?5l' \"(x\" [y]");
        assert!(
            quote_open_at("echo \"a\\\"b"),
            "an escaped quote keeps it open"
        );
        assert!(
            !quote_open_at("echo \\'a"),
            "an escaped quote opens nothing"
        );
    }

    #[test]
    fn backspace_removes_whole_combining_sequence() {
        let mut e = EditorState::default();
        e.insert("e\u{301}");
        e.backspace();
        assert_eq!(e.buffer, "");
        assert_eq!(e.cursor, 0);
        e.undo();
        assert_eq!(e.buffer, "e\u{301}");
    }

    #[test]
    fn mv_left_crosses_zwj_family_in_one_step() {
        let mut e = EditorState::default();
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        e.insert(family);
        e.mv_left();
        assert_eq!(e.cursor, 0);
        e.mv_right();
        assert_eq!(e.cursor, family.len());
        // selection extension uses the same boundaries
        e.extend(EditorState::mv_left);
        assert_eq!(e.selected_text(), Some(family));
    }

    #[test]
    fn backspace_keeps_flag_and_drops_toned_emoji() {
        let mut e = EditorState::default();
        e.insert("\u{1F1EF}\u{1F1F5}\u{1F44D}\u{1F3FD}");
        e.backspace();
        assert_eq!(e.buffer, "\u{1F1EF}\u{1F1F5}");
        e.backspace();
        assert_eq!(e.buffer, "");
        e.undo();
        assert_eq!(e.buffer, "\u{1F1EF}\u{1F1F5}");
        e.undo();
        assert_eq!(e.buffer, "\u{1F1EF}\u{1F1F5}\u{1F44D}\u{1F3FD}");
    }

    #[test]
    fn field_backspace_drops_whole_graphemes() {
        use super::pop_grapheme;
        let mut s = "a\u{65}\u{301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{1F44D}\u{1F3FD}"
            .to_string();
        assert!(pop_grapheme(&mut s));
        assert_eq!(
            s,
            "a\u{65}\u{301}\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"
        );
        assert!(pop_grapheme(&mut s));
        assert_eq!(s, "a\u{65}\u{301}");
        assert!(pop_grapheme(&mut s));
        assert_eq!(s, "a");
        assert!(pop_grapheme(&mut s));
        assert!(!pop_grapheme(&mut s));
    }

    #[test]
    fn delete_forward_removes_whole_combining_sequence() {
        let mut e = EditorState::default();
        e.insert("a\u{65}\u{301}\u{323}b");
        e.home();
        e.mv_right();
        e.delete();
        assert_eq!(e.buffer, "ab");
        assert_eq!(e.cursor, 1);
        e.undo();
        assert_eq!(e.buffer, "a\u{65}\u{301}\u{323}b");
    }

    #[test]
    fn pair_backspace_ignores_opener_with_combining_mark() {
        let mut e = EditorState::default();
        e.insert("(\u{301})");
        e.mv_left();
        e.backspace_with_pairs();
        // only the "(́" cluster goes; the closer is not its pair
        assert_eq!(e.buffer, ")");
        e.undo();
        assert_eq!(e.buffer, "(\u{301})");
    }

    #[test]
    fn word_motions_never_stop_inside_a_cluster() {
        let mut e = EditorState::default();
        // " \u{301}" is one cluster (space + combining acute)
        e.insert("ab \u{301}c");
        e.word_left();
        assert_eq!(e.cursor, 2);
        e.home();
        e.mv_right();
        e.mv_right();
        e.mv_right();
        assert_eq!(e.cursor, 5);
    }

    #[test]
    fn word_delimiters_end_words_for_moves_and_deletes() {
        let mut e = EditorState::default();
        e.insert("cd src/app/main.rs");
        // blanks only by default: one word
        e.word_left();
        assert_eq!(e.cursor, 3);
        let mut e = EditorState {
            word_delimiters: "/.".into(),
            ..Default::default()
        };
        e.insert("cd src/app/main.rs");
        e.word_left();
        assert_eq!(e.cursor, 16); // "rs"
        e.word_left();
        assert_eq!(e.cursor, 11); // "main" (the "." skipped)
        e.word_left();
        assert_eq!(e.cursor, 7); // "app"
        e.word_right();
        assert_eq!(e.cursor, 10); // end of "app"
        e.word_right();
        assert_eq!(e.cursor, 15); // end of "main"
        e.end();
        e.delete_word_back();
        assert_eq!(e.buffer, "cd src/app/main.");
        e.cursor = 3;
        e.delete_word_forward();
        assert_eq!(e.buffer, "cd /app/main.");
    }

    #[test]
    fn word_ops_step_over_multibyte_whitespace() {
        for (text, ws) in [
            ("a\u{3000}b", "\u{3000}"),
            ("a\u{a0}b", "\u{a0}"),
            ("a\u{2003}b", "\u{2003}"),
            ("a b", " "),
            ("a\tb", "\t"),
            ("a \u{3000}\u{a0}b", " \u{3000}\u{a0}"),
        ] {
            let b_at = 1 + ws.len();
            let mut e = EditorState::default();
            e.insert(text);
            e.word_left();
            assert_eq!(e.cursor, b_at, "word_left {text:?}");

            e.end();
            e.extend(EditorState::mv_word_left);
            assert_eq!(e.selection(), Some(b_at..text.len()), "select {text:?}");

            e.end();
            e.delete_word_back();
            assert_eq!(e.buffer, format!("a{ws}"), "delete {text:?}");
            e.undo();
            assert_eq!(e.buffer, text, "undo {text:?}");

            e.home();
            e.word_right();
            assert_eq!(e.cursor, 1, "word_right {text:?}");
        }
    }

    #[test]
    fn word_back_keeps_combining_marks_whole() {
        let mut e = EditorState::default();
        // "e\u{301}" is one cluster after the U+3000 boundary
        e.insert("x\u{3000}e\u{301}y");
        e.delete_word_back();
        assert_eq!(e.buffer, "x\u{3000}");
        e.undo();
        e.insert(" \u{1F468}\u{200D}\u{1F469}");
        e.backspace();
        assert_eq!(e.buffer, "x\u{3000}e\u{301}y ");
    }

    #[test]
    fn grapheme_helpers_snap_interior_offsets() {
        let s = "a\u{3000}b";
        for i in 0..=s.len() {
            let f = super::grapheme_floor(s, i);
            let c = super::grapheme_ceil(s, i);
            assert!(s.is_char_boundary(f) && s.is_char_boundary(c), "{i}");
            assert!(f <= i && c >= i, "{i}");
        }
        assert_eq!(super::grapheme_floor(s, 2), 1);
        assert_eq!(super::grapheme_ceil(s, 2), 4);
    }

    #[test]
    fn a_phones_fit_shrinks_the_grid_until_its_lease_ends() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        // no fit, a fit in both directions, a fit larger than the window
        assert_eq!(super::fitted_grid(None, 120, 40), (120, 40));
        assert_eq!(super::fitted_grid(Some((64, 30, now)), 120, 40), (64, 30));
        assert_eq!(super::fitted_grid(Some((200, 30, now)), 120, 40), (120, 30));
        // a lease that was not refreshed for 30 s is gone (negative control:
        // the same fit, fresh, still holds)
        let old = now - rift_remote::FIT_LEASE - Duration::from_secs(1);
        assert_eq!(super::fitted_grid(Some((64, 30, old)), 120, 40), (120, 40));
        assert_eq!(super::fitted_grid(Some((64, 30, now)), 120, 40), (64, 30));
    }
}
