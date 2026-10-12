use rift_core::git::GitInfo;
use rift_pty::{PaneId, Session};
use unicode_segmentation::GraphemeCursor;

use crate::pane::{NodePath, PaneNode};
use crate::vim::{VimMode, VimState};

/// What a mouse click landed on (rebuilt every frame).
#[derive(Debug, Clone, PartialEq)]
pub enum ClickTarget {
    Block(PaneId, rift_core::BlockId),
    BlockChevron(PaneId, rift_core::BlockId),
    BlockCopy(PaneId, rift_core::BlockId),
    /// A header's duration: hover shows the full time; a click selects
    /// the block like the rest of the header.
    BlockDuration(PaneId, rift_core::BlockId),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    TrafficLight(u8),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    WindowDrag,
    TabSelect(usize),
    TabClose(usize),
    TabNew,
    /// Top-bar sidebar toggles.
    ToggleExplorer,
    ToggleReview,
    /// Top-bar inbox: the notifications popover.
    ToggleNotifications,
    /// Top-bar "Update Rift" chip: the update dialog.
    UpdateChip,
    /// The owned command editor of a pane (click places the cursor).
    Editor(PaneId),
    /// A clickable prompt chip (Warp): the cwd, branch, PHP, Python and
    /// Docker chips open their menus, the diff chip the code review panel,
    /// the PR chip the pull request in the browser.
    Chip(PaneId, rift_core::PromptChip),
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
    /// The completion panel's source tab `i` (0 = all) and row `i`.
    CompletionSource(u8),
    CompletionRow(usize),
    /// A row of the welcome list on a fresh session (`app::welcome_entries`).
    Welcome(u8),
    /// The title bar's split (new pane to the right), ⋮ (pane menu) and ×
    /// (close pane).
    PaneSplit(PaneId),
    PaneMenu(PaneId),
    PaneClose(PaneId),
    HandleDrag(NodePath),
    JumpToBlock(PaneId, rift_core::BlockId),
    // settings overlay controls
    SettingsNoop,
    SettingsNav(u8),
    SettingsClose,
    SettingsToggle(&'static str),
    SettingsStep(&'static str, i8),
    SettingsSegment(&'static str, u8),
    /// The prompt chips preview: opens "Edit prompt".
    SettingsEditPrompt,
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
    SettingsSlider(&'static str),
    SettingsTheme(u8),
    /// Keybindings page row: record a new shortcut for the action.
    SettingsKeybind(String),
    /// Keybindings page ↺: drop the override (back to the default).
    SettingsKeybindReset(String),
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
    pub history: Vec<String>,
    hist_idx: Option<usize>,
    saved: String,
    /// (buffer, cursor) snapshots for ⌘Z / ⇧⌘Z; cleared on submit.
    undo_stack: Vec<(String, usize)>,
    redo_stack: Vec<(String, usize)>,
    /// Modal editing state; `None` = vim mode disabled (see `vim.rs`).
    pub vim: Option<VimState>,
    /// Selection anchor (byte offset); the selection spans anchor..cursor.
    pub anchor: Option<usize>,
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
        self.vim.as_ref().map(|v| v.mode)
    }

    /// Snapshot the current buffer before a mutation (skips no-op duplicates).
    fn push_undo(&mut self) {
        if self.undo_stack.last().map(|(b, _)| b) == Some(&self.buffer) {
            return;
        }
        if self.undo_stack.len() >= UNDO_CAP {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) {
        let Some((buf, cur)) = self.undo_stack.pop() else {
            return;
        };
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
        // bypass push_undo: it would clear redo_stack
        if self.undo_stack.len() >= UNDO_CAP {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.anchor = None;
        self.buffer = buf;
        self.cursor = cur.min(self.buffer.len());
        self.hist_idx = None;
    }

    /// Selected byte range (non-empty), if any.
    pub fn selection(&self) -> Option<std::ops::Range<usize>> {
        let a = self.anchor?.min(self.buffer.len());
        (a != self.cursor).then(|| a.min(self.cursor)..a.max(self.cursor))
    }

    pub fn selected_text(&self) -> Option<&str> {
        self.selection().map(|r| &self.buffer[r])
    }

    pub fn select_all(&mut self) {
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
        let offset = offset.min(self.buffer.len());
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

    /// Whitespace-delimited word start left of the cursor.
    fn word_left_target(&self) -> usize {
        let trimmed = self.buffer[..self.cursor].trim_end();
        // step past the whole whitespace char: U+3000/U+00A0 are multi-byte
        let i = trimmed
            .char_indices()
            .rfind(|&(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        // a space carrying a combining mark is one cluster: never stop inside it
        grapheme_floor(&self.buffer, i)
    }

    /// Whitespace-delimited word end right of the cursor.
    fn word_right_target(&self) -> usize {
        let rest = &self.buffer[self.cursor..];
        let skipped = rest.len() - rest.trim_start().len();
        let word = &rest[skipped..];
        let end = word.find(char::is_whitespace).unwrap_or(word.len());
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

    pub fn mv_home(&mut self) {
        self.cursor = 0;
    }

    pub fn mv_end(&mut self) {
        self.cursor = self.buffer.len();
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

    pub fn kill_to_end(&mut self) {
        self.anchor = None;
        if self.cursor < self.buffer.len() {
            self.replace_range(self.cursor..self.buffer.len(), "");
        }
    }

    pub fn kill_to_start(&mut self) {
        self.anchor = None;
        if self.cursor > 0 {
            self.replace_range(0..self.cursor, "");
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
            self.history.push(cmd.clone());
        }
        cmd
    }

    pub fn history_up(&mut self) {
        self.anchor = None;
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

/// One live shell + its view state.
pub struct PaneState {
    pub session: Session,
    pub view_offset: usize,
    pub last_display_len: usize,
    pub editor: EditorState,
    pub git: Option<GitInfo>,
    pub last_git_cwd: Option<String>,
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
    /// Keyboard-selected block (⌘↑/⌘↓), target of ⇧⌘C / ⌘I / ⌃M / ⌘B.
    pub selected_block: Option<rift_core::BlockId>,
    /// Per-block output filter (Warp block filter): only output lines of
    /// the block containing the query (case-insensitive) are shown.
    pub filter: Option<(rift_core::BlockId, String)>,
    /// Local fix for the last failed command (shown in the empty editor).
    pub correction: Option<String>,
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
}

impl PaneState {
    pub fn new(session: Session) -> Self {
        PaneState {
            session,
            view_offset: 0,
            last_display_len: 0,
            editor: EditorState::default(),
            git: None,
            last_git_cwd: None,
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
            selected_block: None,
            filter: None,
            correction: None,
            correction_checked: None,
            transcript_id: rift_core::transcript::new_id(),
            first_output: None,
            password_block: None,
            title: String::new(),
        }
    }

    /// A command runs in this pane and is working (its tab and title bar
    /// spin). A coding agent that sits at its own prompt waiting for you
    /// still runs as a command, but is not working: no spinner (Warp).
    pub fn running(&self) -> bool {
        use rift_core::notifications::{Agent, agent_idle};
        let agent = {
            let blocks = self.session.blocks.lock().unwrap();
            if !blocks.any_running() {
                return false;
            }
            // the typed command names the agent (no process lookup: this
            // runs every frame)
            blocks
                .last()
                .filter(|b| matches!(b.state, rift_core::BlockState::Running { .. }))
                .and_then(|b| Agent::from_command(&b.command))
        };
        !agent_idle(agent, &self.title)
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
    /// Tab color (index into `TAB_COLORS`).
    pub color: Option<usize>,
    /// ⌥⌘I: editor submissions and raw keys go to every pane of the tab.
    pub sync_inputs: bool,
}

/// Tab color names, in menu order (Warp's palette); rendered from theme ANSI
/// colors so they follow the theme.
pub const TAB_COLORS: [&str; 6] = ["red", "green", "yellow", "blue", "magenta", "cyan"];

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

    pub fn display_title(&self) -> String {
        if let Some(t) = self.custom_title.as_ref().filter(|t| !t.is_empty()) {
            return t.clone();
        }
        if !self.title.is_empty() {
            return self.title.clone();
        }
        self.active()
            .and_then(|p| p.session.cwd.lock().unwrap().clone())
            .map(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or(p)
            })
            .unwrap_or_else(|| "shell".into())
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
fn grapheme_prev(s: &str, i: usize) -> usize {
    // GraphemeCursor panics on an offset inside a char
    let i = s.floor_char_boundary(i);
    let mut c = GraphemeCursor::new(i, s.len(), true);
    // the chunk is the whole string, so no extra context is ever requested
    c.prev_boundary(s, 0).ok().flatten().unwrap_or(0)
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
}
