use shift_core::git::GitInfo;
use shift_pty::{PaneId, Session};

use crate::pane::{NodePath, PaneNode};
use crate::vim::{VimMode, VimState};

/// What a mouse click landed on (rebuilt every frame).
#[derive(Debug, Clone, PartialEq)]
pub enum ClickTarget {
    Block(PaneId, shift_core::BlockId),
    BlockChevron(PaneId, shift_core::BlockId),
    BlockCopy(PaneId, shift_core::BlockId),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    TrafficLight(u8),
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    WindowDrag,
    TabSelect(usize),
    TabClose(usize),
    TabNew,
    /// The owned command editor of a pane (click places the cursor).
    Editor(PaneId),
    PaneFocus(PaneId),
    HandleDrag(NodePath),
    JumpToBlock(PaneId, shift_core::BlockId),
    // settings overlay controls
    SettingsNoop,
    SettingsNav(u8),
    SettingsClose,
    SettingsToggle(&'static str),
    SettingsStep(&'static str, i8),
    SettingsSegment(&'static str, u8),
    SettingsCycle(&'static str),
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
    /// Workflow placeholder spans (byte ranges), tab-cycled.
    pub placeholders: Vec<(usize, usize)>,
    pub active_placeholder: Option<usize>,
    /// Active placeholder untouched — first keystroke replaces it whole.
    placeholder_fresh: bool,
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
        self.exit_placeholder_mode();
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
        self.exit_placeholder_mode();
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

    /// Single mutation point: every buffer edit goes through here so
    /// placeholder spans stay consistent (byte offsets never go stale).
    pub fn replace_range(&mut self, range: std::ops::Range<usize>, s: &str) {
        self.push_undo();
        self.anchor = None;
        let removed = range.end - range.start;
        let delta = s.len() as i64 - removed as i64;
        self.buffer.replace_range(range.clone(), s);
        for span in &mut self.placeholders {
            if span.0 >= range.end {
                // entirely after: shift
                span.0 = (span.0 as i64 + delta) as usize;
                span.1 = (span.1 as i64 + delta) as usize;
            } else if span.1 <= range.start {
                // entirely before: unchanged
            } else {
                // overlaps the edit: span becomes the replacement
                span.0 = range.start;
                span.1 = range.start + s.len();
            }
        }
        self.cursor = range.start + s.len();
    }

    /// Load a workflow command with its placeholder spans; selects the first.
    pub fn set_with_placeholders(&mut self, text: String, spans: Vec<(usize, usize)>) {
        self.anchor = None;
        self.buffer = text;
        self.placeholders = spans;
        self.hist_idx = None;
        if self.placeholders.is_empty() {
            self.active_placeholder = None;
            self.cursor = self.buffer.len();
        } else {
            self.active_placeholder = Some(0);
            self.placeholder_fresh = true;
            self.cursor = self.placeholders[0].1;
        }
    }

    pub fn in_placeholder_mode(&self) -> bool {
        self.active_placeholder.is_some()
    }

    /// Tab: advance to the next placeholder; exits the mode after the last.
    pub fn cycle_placeholder(&mut self) -> bool {
        let Some(i) = self.active_placeholder else {
            return false;
        };
        if i + 1 < self.placeholders.len() {
            self.active_placeholder = Some(i + 1);
            self.placeholder_fresh = true;
            self.cursor = self.placeholders[i + 1].1;
        } else {
            self.exit_placeholder_mode();
        }
        true
    }

    pub fn exit_placeholder_mode(&mut self) {
        self.active_placeholder = None;
        self.placeholders.clear();
        self.placeholder_fresh = false;
    }

    pub fn insert(&mut self, s: &str) {
        if let Some(i) = self.active_placeholder
            && self.placeholder_fresh
        {
            let span = self.placeholders[i];
            self.placeholder_fresh = false;
            self.replace_range(span.0..span.1, s);
        } else if !self.replace_selection(s) {
            self.replace_range(self.cursor..self.cursor, s);
        }
        self.hist_idx = None;
    }

    /// IDE-style quote/bracket insertion. Closing an existing pair steps over
    /// it, while quotes inside identifiers remain literal (for e.g. `don't`).
    pub fn insert_with_pairs(&mut self, s: &str) {
        if self.active_placeholder.is_some() && self.placeholder_fresh {
            self.insert(s);
            return;
        }
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

    fn prev_boundary(&self) -> usize {
        self.buffer[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_boundary(&self) -> usize {
        self.buffer[self.cursor..]
            .chars()
            .next()
            .map(|c| self.cursor + c.len_utf8())
            .unwrap_or(self.cursor)
    }

    pub fn backspace(&mut self) {
        if let Some(i) = self.active_placeholder
            && self.placeholder_fresh
        {
            let span = self.placeholders[i];
            self.placeholder_fresh = false;
            self.replace_range(span.0..span.1, "");
            return;
        }
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
        if (self.active_placeholder.is_some() && self.placeholder_fresh)
            || self.selection().is_some()
        {
            self.backspace();
            return;
        }
        if self.cursor > 0 && self.cursor < self.buffer.len() {
            let prev = self.prev_boundary();
            let next = self.next_boundary();
            let left = self.buffer[prev..self.cursor].chars().next();
            let right = self.buffer[self.cursor..next].chars().next();
            if matches!(
                (left, right),
                (Some('('), Some(')'))
                    | (Some('['), Some(']'))
                    | (Some('{'), Some('}'))
                    | (Some('\''), Some('\''))
                    | (Some('"'), Some('"'))
                    | (Some('`'), Some('`'))
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
        trimmed
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// Whitespace-delimited word end right of the cursor.
    fn word_right_target(&self) -> usize {
        let rest = &self.buffer[self.cursor..];
        let skipped = rest.len() - rest.trim_start().len();
        let word = &rest[skipped..];
        let end = word.find(char::is_whitespace).unwrap_or(word.len());
        self.cursor + skipped + end
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
        self.exit_placeholder_mode();
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
        self.exit_placeholder_mode();
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
        self.exit_placeholder_mode();
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
        self.exit_placeholder_mode();
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
    /// Exit code of the last finished command (recolors the ❯).
    pub last_exit: Option<i32>,
    /// Frame cache for mouse → grid mapping (see terminal::RowKind).
    pub rows: Vec<crate::views::terminal::RowInfo>,
    pub last_grid: Option<crate::views::terminal::GridLayout>,
    pub alt_screen: bool,
    /// Mirrors `term.selection.is_some()` to avoid locking on every key.
    pub has_selection: bool,
    /// Visible URL spans from the last frame: (rect, url).
    pub url_hits: Vec<([f32; 4], String)>,
    /// Keyboard-selected block (⌘↑/⌘↓), target of ⇧⌘C / ⌘I / ⌃M / ⌘B.
    pub selected_block: Option<shift_core::BlockId>,
    /// Per-block output filter (Warp block filter): only output lines of
    /// the block containing the query (case-insensitive) are shown.
    pub filter: Option<(shift_core::BlockId, String)>,
    /// Local fix for the last failed command (shown in the empty editor).
    pub correction: Option<String>,
    /// Newest finished block already checked for a correction.
    pub correction_checked: Option<shift_core::BlockId>,
    /// File id of this pane's saved block transcript (`save_transcripts`).
    pub transcript_id: String,
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
            last_exit: None,
            rows: Vec::new(),
            last_grid: None,
            alt_screen: false,
            has_selection: false,
            url_hits: Vec::new(),
            selected_block: None,
            filter: None,
            correction: None,
            correction_checked: None,
            transcript_id: shift_core::transcript::new_id(),
        }
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
    pub fn effective_split(&self) -> bool {
        self.is_split() && self.zoomed.is_none()
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

#[cfg(test)]
mod tests {
    use super::EditorState;

    #[test]
    fn placeholder_replace_and_cycle() {
        let mut e = EditorState::default();
        e.set_with_placeholders("ssh host -p port".into(), vec![(4, 8), (12, 16)]);
        assert!(e.in_placeholder_mode());
        assert_eq!(e.cursor, 8);
        // first keystroke replaces the whole fresh span
        e.insert("prod.example.com");
        assert_eq!(e.buffer, "ssh prod.example.com -p port");
        // second span shifted correctly
        assert_eq!(e.placeholders[1], (24, 28));
        assert_eq!(&e.buffer[24..28], "port");
        // tab → next placeholder, type replaces it
        assert!(e.cycle_placeholder());
        e.insert("2222");
        assert_eq!(e.buffer, "ssh prod.example.com -p 2222");
        // tab past last exits mode
        assert!(e.cycle_placeholder());
        assert!(!e.in_placeholder_mode());
    }

    #[test]
    fn placeholder_backspace_clears_fresh_span() {
        let mut e = EditorState::default();
        e.set_with_placeholders("kill -9 pid".into(), vec![(8, 11)]);
        e.backspace();
        assert_eq!(e.buffer, "kill -9 ");
        assert_eq!(e.cursor, 8);
        // subsequent typing inserts normally at the (now empty) span
        e.insert("42");
        assert_eq!(e.buffer, "kill -9 42");
    }

    #[test]
    fn replace_range_adjusts_spans() {
        let mut e = EditorState::default();
        e.set_with_placeholders("a BB c DD".into(), vec![(2, 4), (7, 9)]);
        // edit before both spans
        e.replace_range(0..1, "xyz");
        assert_eq!(e.buffer, "xyz BB c DD");
        assert_eq!(e.placeholders, vec![(4, 6), (9, 11)]);
        assert_eq!(&e.buffer[4..6], "BB");
        assert_eq!(&e.buffer[9..11], "DD");
    }

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
    fn undo_exits_placeholder_mode() {
        let mut e = EditorState::default();
        e.set_with_placeholders("ssh host".into(), vec![(4, 8)]);
        e.insert("prod");
        assert_eq!(e.buffer, "ssh prod");
        e.undo();
        assert_eq!(e.buffer, "ssh host");
        assert!(!e.in_placeholder_mode());
        assert!(e.placeholders.is_empty());
    }

    #[test]
    fn submit_exits_placeholder_mode() {
        let mut e = EditorState::default();
        e.set_with_placeholders("echo x".into(), vec![(5, 6)]);
        let cmd = e.submit();
        assert_eq!(cmd, "echo x");
        assert!(!e.in_placeholder_mode());
        assert!(e.placeholders.is_empty());
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
}
