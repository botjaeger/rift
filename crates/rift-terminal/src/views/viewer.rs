//! Read-only file viewer: opens from the explorer or the review panel as a
//! pane of the active tab's split. Line numbers, lightweight highlighting
//! (`rift_core::code_highlight`), only the lines on screen are shaped.
//! Markdown files open rendered (`markdown_view`), with a Rendered | Raw
//! switch in the header.
//! Esc or ✕ closes it; the terminal underneath keeps running. ⌘F opens a
//! find bar scoped to the open file (case-insensitive plain text).
//!
//! config.toml beside Settings is the one file it edits ([`ConfigEdit`]):
//! a caret, selection and undo, Save / Dismiss in the header while the text
//! differs from the file, lint marks 150 ms after the last keystroke, and a
//! bar when the file changes on disk under unsaved edits.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::state::EditorState;
use crate::views::icons::{self, Icon};
use rift_core::code_highlight::{self, Kind, Lang, State};
use rift_core::config_lint::{Diagnostic, Severity};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

/// Files past this are not loaded.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Same height as a split pane's title bar (the viewer is a pane).
pub const HEADER_H: f32 = 28.0;
const FONT: f32 = 13.0;
const LINE: f32 = 20.0;
/// Height of the find strip under the header while find is open.
const FIND_H: f32 = 40.0;
/// Height of the conflict strip under the header and of the lint status
/// bar under the text (the pane title bar's height).
const BAR_H: f32 = HEADER_H;
/// Lint runs this long after the last keystroke.
pub const LINT_DELAY: Duration = Duration::from_millis(150);
/// How long "Saved" shows in the header after a save.
pub const SAVED_SHOWN: Duration = Duration::from_millis(1500);
/// Extra gutter width while editing: the lint dots' column, left of the
/// line numbers.
const LINT_GUTTER: f32 = 10.0;
/// Spaces Tab inserts.
const TAB: &str = "  ";

/// The config.toml pane's editing state. Its buffer is the command line's
/// `EditorState`: the same undo stack, selection, grapheme-aware moves and
/// `replace_range` as the one mutation point (its history, vim mode and
/// extra carets stay unused here).
#[derive(Default)]
pub struct ConfigEdit {
    pub buffer: EditorState,
    /// The file's text as last read or written.
    pub disk: String,
    /// The last lint of `buffer` (line order).
    pub diagnostics: Vec<Diagnostic>,
    /// The file changed on disk while the buffer had unsaved edits.
    pub conflict: bool,
    /// When the pending lint runs (debounce after a keystroke).
    pub lint_due: Option<Instant>,
    /// When the last save landed ("Saved" in the header).
    pub saved_at: Option<Instant>,
    /// The generated-profiles list (`lint_with`): Rift's own shell stubs.
    pub seen: Option<BTreeSet<String>>,
    /// A press in the text is selecting: a drag extends to the pointer.
    pub dragging: bool,
    /// The column ↑ / ↓ keep across shorter lines (chars).
    goal: Option<usize>,
    /// The diagnostic the status bar jumped to last.
    next_diag: usize,
}

impl std::fmt::Debug for ConfigEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigEdit")
            .field("dirty", &self.dirty())
            .field("conflict", &self.conflict)
            .field("cursor", &self.buffer.cursor)
            .field("diagnostics", &self.diagnostics.len())
            .finish()
    }
}

impl ConfigEdit {
    pub fn new(text: String, seen: Option<BTreeSet<String>>) -> Self {
        let mut buffer = EditorState::default();
        buffer.buffer = text.clone();
        let mut e = ConfigEdit {
            buffer,
            disk: text,
            seen,
            ..Default::default()
        };
        e.lint();
        e
    }

    /// The text differs from the file.
    pub fn dirty(&self) -> bool {
        self.buffer.buffer != self.disk
    }

    pub fn errors(&self) -> usize {
        self.count(Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        self.count(Severity::Warning)
    }

    fn count(&self, s: Severity) -> usize {
        self.diagnostics.iter().filter(|d| d.severity == s).count()
    }

    /// Save is offered (dirty) and allowed (no error in the last lint; a
    /// save lints a pending edit first).
    pub fn can_save(&self) -> bool {
        self.dirty() && self.errors() == 0
    }

    /// Lint the buffer now.
    pub fn lint(&mut self) {
        self.lint_due = None;
        self.diagnostics =
            rift_core::config_lint::lint_with(&self.buffer.buffer, self.seen.as_ref());
    }

    /// "2 errors · 1 warning — line 4: …" (the first problem), or `None`.
    pub fn summary(&self) -> Option<String> {
        let first = self.diagnostics.first()?;
        let plural = |n: usize, w: &str| format!("{n} {w}{}", if n == 1 { "" } else { "s" });
        let mut counts = Vec::new();
        if self.errors() > 0 {
            counts.push(plural(self.errors(), "error"));
        }
        if self.warnings() > 0 {
            counts.push(plural(self.warnings(), "warning"));
        }
        Some(format!(
            "{} — line {}: {}",
            counts.join(" · "),
            first.line + 1,
            first.message
        ))
    }

    /// Byte offset of char column `col` on 0-based line `line` (clamped to
    /// the line's end; past the last line: the end of the text).
    pub fn offset_of(&self, line: usize, col: usize) -> usize {
        let text = &self.buffer.buffer;
        let Some(start) = line_start(text, line) else {
            return text.len();
        };
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        text[start..end]
            .char_indices()
            .nth(col)
            .map_or(end, |(i, _)| start + i)
    }

    /// The caret's (line, char column).
    pub fn caret(&self) -> (usize, usize) {
        line_col(&self.buffer.buffer, self.buffer.cursor)
    }

    /// The file on disk now reads `text`. A clean buffer follows it (the
    /// caret stays where it was, clamped); a dirty one is never replaced:
    /// unless the file now holds exactly the buffer (our own save), the
    /// change is a conflict to resolve. True when anything changed.
    pub fn disk_changed(&mut self, text: String) -> bool {
        if text == self.disk {
            return false;
        }
        if !self.dirty() {
            let cursor = self.buffer.cursor;
            self.set_text(&text);
            self.buffer.cursor = floor_boundary(&self.buffer.buffer, cursor);
            self.disk = text;
            self.lint();
        } else {
            self.conflict = text != self.buffer.buffer;
            self.disk = text;
        }
        true
    }

    /// Conflict › Reload, and Dismiss: the file's text replaces the
    /// buffer (one undo step).
    pub fn revert(&mut self) {
        let disk = self.disk.clone();
        self.set_text(&disk);
        self.buffer.cursor = floor_boundary(&self.buffer.buffer, self.buffer.cursor);
        self.conflict = false;
        self.lint();
    }

    /// The whole text replaced through the buffer's one mutation point.
    fn set_text(&mut self, text: &str) {
        if self.buffer.buffer != text {
            let cursor = self.buffer.cursor;
            let all = 0..self.buffer.buffer.len();
            self.buffer.replace_range(all, text);
            self.buffer.cursor = floor_boundary(text, cursor);
        }
    }

    /// The buffer was written to the file.
    pub fn saved(&mut self, now: Instant) {
        self.disk = self.buffer.buffer.clone();
        self.conflict = false;
        self.saved_at = Some(now);
    }

    /// Move the caret `delta` lines, keeping its column (`goal`).
    fn vertical(&mut self, delta: isize) {
        let (line, col) = self.caret();
        let goal = *self.goal.get_or_insert(col);
        let lines = self.buffer.buffer.split('\n').count() as isize;
        let to = line as isize + delta;
        self.buffer.cursor = if to < 0 {
            0
        } else if to >= lines {
            self.buffer.buffer.len()
        } else {
            self.offset_of(to as usize, goal)
        };
    }

    /// The next diagnostic after the last one jumped to (wrapping): the
    /// caret goes to its start.
    pub fn jump_next(&mut self) -> bool {
        if self.diagnostics.is_empty() {
            return false;
        }
        let i = self.next_diag % self.diagnostics.len();
        self.next_diag = i + 1;
        let d = &self.diagnostics[i];
        let at = self.offset_of(d.line, d.cols.start);
        self.buffer.click_to(at, false);
        self.goal = None;
        true
    }
}

/// Byte offset where 0-based `line` starts, if the text has it.
fn line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    text.match_indices('\n').nth(line - 1).map(|(i, _)| i + 1)
}

/// (line, char column) of byte offset `at`.
fn line_col(text: &str, at: usize) -> (usize, usize) {
    let at = at.min(text.len());
    let before = &text[..at];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, text[start..at].chars().count())
}

fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Display width in columns of `s` as drawn (a tab: 4).
fn display_cols(s: &str) -> usize {
    s.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// The modifiers an edit key reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct EditMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    /// ⌘ on macOS.
    pub cmd: bool,
}

impl EditMods {
    /// The command modifier: ⌘ (Super), and Ctrl too off macOS.
    fn primary(&self) -> bool {
        self.cmd || (self.ctrl && !cfg!(target_os = "macos"))
    }
}

/// What an edit key asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum EditOutcome {
    /// Not an edit key: app shortcuts may take it.
    Ignored,
    Handled,
    /// ⌘S.
    Save,
    /// ⌘C / ⌘X: this text to the clipboard.
    Copy(String),
    /// ⌘V.
    Paste,
    /// Esc without a selection: close the pane (asks while dirty).
    Close,
}
/// Color runs covering a line, with the byte ranges in `over` repainted
/// `color` (runs split at their edges). Both inputs are ascending.
fn recolor(
    runs: Vec<(std::ops::Range<usize>, Color)>,
    over: &[std::ops::Range<usize>],
    color: Color,
) -> Vec<(std::ops::Range<usize>, Color)> {
    let mut out = Vec::with_capacity(runs.len() + over.len() * 2);
    for (run, c) in runs {
        let mut at = run.start;
        for m in over
            .iter()
            .filter(|m| m.start < run.end && m.end > run.start)
        {
            let (s, e) = (m.start.max(run.start), m.end.min(run.end));
            if s > at {
                out.push((at..s, c));
            }
            out.push((s..e, color));
            at = e;
        }
        if at < run.end {
            out.push((at..run.end, c));
        }
    }
    out
}

/// In-file find: case-insensitive plain-text matches over `ViewerState::lines`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ViewerFind {
    pub query: String,
    /// `(line index, byte start, byte len)`, sorted by line then start.
    pub matches: Vec<(usize, usize, usize)>,
    /// Index into `matches` of the selected match.
    pub current: Option<usize>,
}

impl ViewerFind {
    /// Counter text: `n/total`, `No matches`, or a placeholder for an empty query.
    pub fn counter(&self) -> String {
        match self.current {
            Some(i) => format!("{}/{}", i + 1, self.matches.len()),
            None if self.query.is_empty() => "Find in file".to_string(),
            None => "No matches".to_string(),
        }
    }
}

fn eq_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Non-overlapping case-insensitive matches of `query` in `lines`. Matching is
/// char by char, so byte ranges always fall on char boundaries of the line.
pub fn find_matches(lines: &[String], query: &str) -> Vec<(usize, usize, usize)> {
    let q: Vec<char> = query.chars().collect();
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    for (li, line) in lines.iter().enumerate() {
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
        while i + q.len() <= chars.len() {
            if q.iter()
                .zip(&chars[i..])
                .all(|(qc, (_, lc))| eq_ci(*lc, *qc))
            {
                let start = chars[i].0;
                let end = chars.get(i + q.len()).map_or(line.len(), |c| c.0);
                out.push((li, start, end - start));
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct ViewerState {
    pub path: Option<PathBuf>,
    /// Shown instead of the text (binary, too large, unreadable).
    pub error: Option<String>,
    pub lines: Vec<String>,
    lang: Option<Lang>,
    /// Highlighter state at the start of each line.
    states: Vec<State>,
    pub scroll: f32,
    pub view_h: f32,
    generation: u64,
    /// Open find bar (⌘F); reset whenever a file opens, loads or closes.
    pub find: Option<ViewerFind>,
    /// Markdown blocks (empty for other files): the *Rendered* view.
    pub markdown: Vec<rift_core::markdown::Block>,
    /// Showing the rendered Markdown rather than the source (Warp's
    /// Rendered | Raw toggle); on when a Markdown file loads.
    pub rendered: bool,
    md_layout: super::markdown_view::MdLayout,
    /// Horizontal scroll of the source view (px).
    pub scroll_x: f32,
    /// Horizontal scroll of each rendered code block (by block index).
    pub code_scroll: std::collections::HashMap<usize, f32>,
    /// Widest source line (measured once per file).
    max_line_w: Option<f32>,
    /// Source view width including the gutter, and the last frame's
    /// viewport (for clamping between frames).
    content_w: f32,
    view_w: f32,
    body_top: f32,
    /// Source line (0-based) to scroll to once the file is loaded and laid
    /// out (`rift://action/open_file?…&line=`).
    pub reveal_line: Option<usize>,
    /// Selected source lines (0-based): (anchor, cursor). A click picks a
    /// line, ⇧-click and ⇧↑ ⇧↓ extend; "Send to agent…" sends them.
    pub sel: Option<(usize, usize)>,
    /// The file's exact text (valid UTF-8 only): what an edit starts from.
    raw: Option<String>,
    /// Editing config.toml (Settings › Open config.toml only).
    pub edit: Option<ConfigEdit>,
    /// Where the source text starts (window x, before `scroll_x`) and one
    /// monospace cell's width, last frame: clicks map to a caret with them.
    text_x: f32,
    char_w: f32,
    /// The line-number gutter's width, last frame.
    gutter: f32,
    /// config.toml that cannot be edited here (not UTF-8 text, too large,
    /// binary): why, shown in the status bar.
    pub read_only: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerHit {
    Close,
    /// The ⋮ button: the viewer menu (insert path, copy path, find).
    Menu,
    /// Rendered | Raw segments (Markdown files).
    Mode(bool),
    /// A rendered shell code block's Run button (index into `markdown`).
    RunCode(usize),
    /// config.toml: write the buffer (disabled while lint finds an error).
    Save,
    /// config.toml: drop the edits (the file's text again).
    Dismiss,
    /// The conflict bar: the file's new text replaces the buffer.
    Reload,
    /// The conflict bar: keep the buffer (a save then overwrites the file).
    KeepMine,
    /// The lint status bar: the caret to the next problem.
    Status,
    /// A gutter dot: the problems on this 0-based line (hover tooltip).
    Lint(usize),
}

impl ViewerState {
    pub fn is_open(&self) -> bool {
        self.path.is_some()
    }

    pub fn close(&mut self) {
        *self = ViewerState::default();
    }

    pub fn open(&mut self, path: &Path, deliver: impl FnOnce(ViewerState) + Send + 'static) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT.fetch_add(1, Ordering::Relaxed);
        *self = ViewerState {
            path: Some(path.to_path_buf()),
            error: Some("Loading…".into()),
            generation,
            ..ViewerState::default()
        };
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let mut loaded = ViewerState {
                path: Some(path.clone()),
                generation,
                ..Default::default()
            };
            loaded.load(&path);
            deliver(loaded);
        });
    }

    /// `loaded` is the file this viewer is waiting for (generations are
    /// unique across windows).
    pub fn awaits(&self, loaded: &ViewerState) -> bool {
        self.path.is_some() && self.generation == loaded.generation
    }

    pub fn accept(&mut self, loaded: ViewerState) -> bool {
        if !self.awaits(&loaded) {
            return false;
        }
        let reveal = self.reveal_line.take();
        *self = loaded;
        // a line of a Markdown file is a line of its source
        if reveal.is_some() {
            self.rendered = false;
        }
        self.reveal_line = reveal;
        true
    }

    /// Scroll a pending [`ViewerState::reveal_line`] into view (centred),
    /// once the file has lines and the viewport its height.
    fn apply_reveal(&mut self) {
        if self.lines.is_empty() || self.view_h <= 0.0 {
            return;
        }
        let Some(line) = self.reveal_line.take() else {
            return;
        };
        let line = line.min(self.lines.len() - 1);
        let top = 12.0 + line as f32 * LINE;
        self.scroll = top - ((self.view_h - LINE) / 2.0).max(0.0);
        self.scroll_by(0.0);
    }

    fn load(&mut self, path: &Path) {
        match std::fs::metadata(path) {
            Err(e) => self.error = Some(e.to_string()),
            Ok(m) if !m.is_file() => {
                self.error = Some("Only regular files can be previewed".into())
            }
            Ok(m) if m.len() > MAX_BYTES => {
                self.error = Some(format!(
                    "Too large to preview ({:.1} MB)",
                    m.len() as f64 / 1_048_576.0
                ))
            }
            Ok(_) => match read_preview(path) {
                Err(e) => self.error = Some(e.to_string()),
                Ok(bytes) if bytes.iter().take(8192).any(|&b| b == 0) => {
                    self.error = Some("Binary file".into())
                }
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    if let std::borrow::Cow::Borrowed(exact) = &text {
                        self.raw = Some(exact.to_string());
                    }
                    self.lines = text
                        .split('\n')
                        .map(|l| l.trim_end_matches('\r').replace('\t', "    "))
                        .collect();
                    if self.lines.last().is_some_and(String::is_empty) {
                        self.lines.pop();
                    }
                    let lang = code_highlight::lang_for(path);
                    let mut state = State::default();
                    self.states = self
                        .lines
                        .iter()
                        .map(|l| {
                            let at = state;
                            code_highlight::line(lang, &mut state, l);
                            at
                        })
                        .collect();
                    self.lang = Some(lang);
                    if rift_core::markdown::is_markdown(path) {
                        self.markdown = rift_core::markdown::parse(&self.lines);
                        self.rendered = true;
                    }
                }
            },
        }
    }

    pub fn content_h(&self) -> f32 {
        if self.showing_rendered() && self.md_layout.height > 0.0 {
            return self.md_layout.height;
        }
        self.lines.len() as f32 * LINE + 24.0
    }

    /// Scroll sideways by `px` with the pointer at window `y`: the source
    /// view as a whole, or (rendered) the code block under the pointer.
    pub fn scroll_x_by(&mut self, px: f32, y: f32) {
        if self.showing_rendered() {
            let at = y - self.body_top + self.scroll;
            if let Some((i, max)) = self.md_layout.code_at(at) {
                let s = self.code_scroll.entry(i).or_insert(0.0);
                *s = (*s + px).clamp(0.0, max);
            }
            return;
        }
        let max = (self.content_w - self.view_w).max(0.0);
        self.scroll_x = (self.scroll_x + px).clamp(0.0, max);
    }

    /// A Markdown file shown rendered.
    pub fn showing_rendered(&self) -> bool {
        self.rendered && !self.markdown.is_empty()
    }

    /// Switch between the rendered page and the source (top of the file).
    pub fn set_rendered(&mut self, on: bool) {
        if self.rendered != on {
            self.rendered = on;
            self.scroll = 0.0;
        }
    }

    pub fn scroll_by(&mut self, px: f32) {
        let max = (self.content_h() - self.view_h).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    /// The source line at window `y` (the source view only).
    pub fn line_at(&self, y: f32) -> Option<usize> {
        if self.showing_rendered() || self.error.is_some() || y < self.body_top {
            return None;
        }
        let i = ((y - self.body_top + self.scroll - 12.0) / LINE).floor();
        (i >= 0.0 && (i as usize) < self.lines.len()).then_some(i as usize)
    }

    /// The first line fully on screen.
    pub fn top_line(&self) -> usize {
        (((self.scroll - 12.0) / LINE).ceil().max(0.0) as usize)
            .min(self.lines.len().saturating_sub(1))
    }

    /// Select line `i`, or extend the selection to it.
    pub fn select(&mut self, i: usize, extend: bool) {
        let anchor = match self.sel {
            Some((a, _)) if extend => a,
            _ => i,
        };
        self.sel = Some((anchor, i));
    }

    /// ⇧↑ / ⇧↓: move the selection's moving end `delta` lines (starting at
    /// the top line on screen), scrolling it into view.
    pub fn extend_by(&mut self, delta: isize) {
        if self.lines.is_empty() || self.showing_rendered() {
            return;
        }
        let (anchor, cursor) = self.sel.unwrap_or((self.top_line(), self.top_line()));
        let to = (cursor as isize + delta).clamp(0, self.lines.len() as isize - 1) as usize;
        self.sel = Some((anchor, to));
        let top = 12.0 + to as f32 * LINE;
        if top < self.scroll {
            self.scroll = top - 12.0;
        } else if top + LINE > self.scroll + self.view_h {
            self.scroll = top + LINE - self.view_h + 12.0;
        }
        self.scroll_by(0.0);
    }

    /// The selected lines as an inclusive range, else the top line on
    /// screen; `None` without source lines, and on the rendered Markdown
    /// page without a selection (its scroll maps to no source line).
    pub fn send_range(&self) -> Option<(usize, usize)> {
        if self.lines.is_empty() || self.error.is_some() {
            return None;
        }
        match self.sel {
            Some((a, b)) => Some((a.min(b), a.max(b).min(self.lines.len() - 1))),
            None if self.showing_rendered() => None,
            None => Some((self.top_line(), self.top_line())),
        }
    }

    pub fn page(&self) -> f32 {
        (self.view_h - LINE * 2.0).max(LINE)
    }

    /// Open (or keep) the find bar; a no-op while no file is open.
    pub fn open_find(&mut self) {
        if self.is_open() && self.find.is_none() {
            // find works on the source lines
            self.set_rendered(false);
            self.find = Some(ViewerFind::default());
        }
    }

    /// Keys while find is open: text edits the query, Backspace pops,
    /// Enter / ⇧Enter step to the next / previous match (wrapping), Esc
    /// closes find (not the viewer). Returns true when consumed.
    pub fn handle_find_key(&mut self, event: &crate::input::KeyPress, shift: bool) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let Some(find) = self.find.as_mut() else {
            return false;
        };
        if event.released {
            return false;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.find = None;
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                self.step_match(!shift);
                return true;
            }
            Key::Named(NamedKey::Backspace) => {
                crate::state::pop_grapheme(&mut find.query);
                self.refresh_find();
                return true;
            }
            _ => {}
        }
        let typed: String = event
            .text
            .as_deref()
            .unwrap_or("")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if typed.is_empty() {
            return false;
        }
        find.query.push_str(&typed);
        self.refresh_find();
        true
    }

    /// Recompute matches for the query and select the first match at or
    /// after the top of the viewport (wrapping to the first).
    fn refresh_find(&mut self) {
        let top = ((self.scroll - 12.0) / LINE).floor().max(0.0) as usize;
        let Some(find) = self.find.as_mut() else {
            return;
        };
        find.matches = find_matches(&self.lines, &find.query);
        find.current = if find.matches.is_empty() {
            None
        } else {
            Some(find.matches.partition_point(|m| m.0 < top) % find.matches.len())
        };
        self.reveal_current();
    }

    /// Move to the next (`forward`) or previous match, wrapping.
    fn step_match(&mut self, forward: bool) {
        let Some(find) = self.find.as_mut() else {
            return;
        };
        let n = find.matches.len();
        if n == 0 {
            return;
        }
        find.current = Some(match find.current {
            None => 0,
            Some(i) if forward => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
        });
        self.reveal_current();
    }

    /// Scroll so the selected match's line is visible (centered when it
    /// was off screen).
    fn reveal_current(&mut self) {
        let Some(line) = self
            .find
            .as_ref()
            .and_then(|f| f.current.map(|i| f.matches[i].0))
        else {
            return;
        };
        let top = 12.0 + line as f32 * LINE;
        if top < self.scroll || top + LINE > self.scroll + self.view_h {
            self.scroll = top - ((self.view_h - LINE) / 2.0).max(0.0);
        }
        self.scroll_by(0.0);
    }
}

/// Editing (config.toml beside Settings).
impl ViewerState {
    /// Make the loaded file editable: its exact text, or an empty buffer
    /// when it does not exist yet (Save creates it). A binary, unreadable
    /// or non-UTF-8 file stays read-only.
    pub fn start_edit(&mut self, seen: Option<BTreeSet<String>>) {
        if self.edit.is_some() || self.path.is_none() {
            return;
        }
        if self.raw.is_none() && self.path.as_ref().is_some_and(|p| !p.exists()) {
            self.raw = Some(String::new());
            self.error = None;
        }
        let Some(text) = self.raw.take() else {
            // say why typing does nothing
            let why = match &self.error {
                Some(e) => e.trim_end_matches('.').to_string(),
                None => "it is not UTF-8 text".to_string(),
            };
            self.read_only = Some(format!(
                "Read-only: {why}. Fix config.toml in another editor"
            ));
            return;
        };
        self.rendered = false;
        self.sel = None;
        self.edit = Some(ConfigEdit::new(text, seen));
        self.sync_lines();
    }

    /// Height of the status bar under the text (lint summary or why it is
    /// read-only), 0 without one: toasts at the window's foot sit above it.
    pub fn status_bar_h(&self) -> f32 {
        let shown = self
            .edit
            .as_ref()
            .is_some_and(|e| !e.diagnostics.is_empty())
            || self.read_only.is_some();
        if self.path.is_some() && shown {
            BAR_H
        } else {
            0.0
        }
    }

    /// Unsaved edits.
    pub fn dirty(&self) -> bool {
        self.edit.as_ref().is_some_and(ConfigEdit::dirty)
    }

    /// The file on disk now reads `text` (Settings saved it, the watcher
    /// saw it change, an external editor wrote it): see
    /// [`ConfigEdit::disk_changed`]. The scroll position stays.
    pub fn disk_changed(&mut self, text: String) -> bool {
        let Some(edit) = self.edit.as_mut() else {
            return false;
        };
        let changed = edit.disk_changed(text);
        if changed {
            self.sync_lines();
        }
        changed
    }

    /// Dismiss / Reload: the file's text replaces the buffer.
    pub fn revert(&mut self) {
        if let Some(edit) = self.edit.as_mut() {
            edit.revert();
            self.sync_lines();
        }
    }

    /// Run a lint that is due; returns (repaint now, next deadline).
    pub fn tick(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let Some(edit) = self.edit.as_mut() else {
            return (false, None);
        };
        let mut repaint = false;
        if edit.lint_due.is_some_and(|due| now >= due) {
            edit.lint();
            repaint = true;
        }
        if let Some(at) = edit.saved_at
            && now >= at + SAVED_SHOWN
        {
            edit.saved_at = None;
            repaint = true;
        }
        let next = [edit.lint_due, edit.saved_at.map(|at| at + SAVED_SHOWN)]
            .into_iter()
            .flatten()
            .min();
        (repaint, next)
    }

    /// `lines` (display: a tab as 4 spaces) and their highlighter states
    /// from the buffer; the last line stays even when empty (the caret can
    /// stand on it).
    fn sync_lines(&mut self) {
        let Some(edit) = &self.edit else {
            return;
        };
        self.lines = edit
            .buffer
            .buffer
            .split('\n')
            .map(|l| l.trim_end_matches('\r').replace('\t', "    "))
            .collect();
        let lang = self.lang.unwrap_or(Lang::Plain);
        let mut state = State::default();
        self.states = self
            .lines
            .iter()
            .map(|l| {
                let at = state;
                code_highlight::line(lang, &mut state, l);
                at
            })
            .collect();
        self.max_line_w = None;
        if let Some(find) = self.find.as_mut() {
            find.matches = find_matches(&self.lines, &find.query);
            find.current = find
                .current
                .filter(|_| !find.matches.is_empty())
                .map(|i| i.min(find.matches.len() - 1));
        }
        self.scroll_by(0.0);
    }

    /// After an edit: the lines again, a lint scheduled, the caret shown.
    fn edited(&mut self, now: Instant) {
        if let Some(edit) = self.edit.as_mut() {
            edit.lint_due = Some(now + LINT_DELAY);
        }
        self.sync_lines();
        self.reveal_caret();
    }

    /// Scroll so the caret's line and column are on screen.
    pub fn reveal_caret(&mut self) {
        let Some(edit) = &self.edit else {
            return;
        };
        let (line, col) = edit.caret();
        let text = &edit.buffer.buffer;
        let start = line_start(text, line).unwrap_or(0);
        let prefix: String = text[start..].chars().take(col).collect();
        let cols = display_cols(&prefix);
        let top = 12.0 + line as f32 * LINE;
        if top < self.scroll {
            self.scroll = (top - 12.0).max(0.0);
        } else if top + LINE > self.scroll + self.view_h {
            self.scroll = top + LINE + 12.0 - self.view_h;
        }
        self.scroll_by(0.0);
        if self.char_w > 0.0 {
            let cx = cols as f32 * self.char_w;
            let room = (self.view_w - self.gutter - 24.0).max(self.char_w);
            if cx < self.scroll_x {
                self.scroll_x = (cx - 24.0).max(0.0);
            } else if cx > self.scroll_x + room {
                self.scroll_x = cx - room;
            }
        }
    }

    /// The buffer offset under window point (x, y), in the text area.
    pub fn edit_offset_at(&self, x: f32, y: f32) -> Option<usize> {
        let edit = self.edit.as_ref()?;
        if y < self.body_top || self.char_w <= 0.0 {
            return None;
        }
        let line = ((y - self.body_top + self.scroll - 12.0) / LINE)
            .floor()
            .max(0.0) as usize;
        let text = &edit.buffer.buffer;
        let Some(start) = line_start(text, line) else {
            return Some(text.len());
        };
        let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        let want = ((x - self.text_x + self.scroll_x) / self.char_w).max(0.0);
        // the nearest boundary between cells (a tab spans 4)
        let mut at_col = 0.0;
        for (i, c) in text[start..end].char_indices() {
            let w = if c == '\t' { 4.0 } else { 1.0 };
            if want < at_col + w / 2.0 {
                return Some(start + i);
            }
            at_col += w;
        }
        Some(end)
    }

    /// A press in the text: the caret there (⇧ extends), and a drag
    /// selects from it.
    pub fn edit_press(&mut self, x: f32, y: f32, extend: bool) -> bool {
        let Some(at) = self.edit_offset_at(x, y) else {
            return false;
        };
        if let Some(edit) = self.edit.as_mut() {
            edit.buffer.click_to(at, extend);
            edit.dragging = true;
            edit.goal = None;
        }
        true
    }

    /// The pointer moved with the button down: extend to it.
    pub fn edit_drag(&mut self, x: f32, y: f32) -> bool {
        if !self.edit.as_ref().is_some_and(|e| e.dragging) {
            return false;
        }
        let Some(at) = self.edit_offset_at(x, y) else {
            return false;
        };
        if let Some(edit) = self.edit.as_mut() {
            edit.buffer.click_to(at, true);
        }
        self.reveal_caret();
        true
    }

    /// Insert `text` at the caret (a paste; CRLF as LF).
    pub fn edit_insert(&mut self, text: &str, now: Instant) {
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        let text = text.replace("\r\n", "\n");
        edit.buffer.insert(&text);
        edit.goal = None;
        self.edited(now);
    }

    /// Where the first line's text starts on screen (x, y of its top) and a
    /// cell's width, last frame (`dump`: scripts click and drag by them).
    pub fn text_origin(&self) -> (f32, f32, f32) {
        (
            self.text_x - self.scroll_x,
            self.body_top + 12.0 - self.scroll,
            self.char_w,
        )
    }

    /// A gutter dot: the caret to the start of its 0-based line.
    pub fn caret_to_line(&mut self, line: usize) {
        if let Some(edit) = self.edit.as_mut() {
            let at = edit.offset_of(line, 0);
            edit.buffer.click_to(at, false);
            edit.goal = None;
            self.reveal_caret();
        }
    }

    /// Status bar: the caret to the next problem, scrolled into view.
    pub fn jump_next_problem(&mut self) {
        if self.edit.as_mut().is_some_and(ConfigEdit::jump_next) {
            self.reveal_caret();
        }
    }

    /// A key while the config pane edits (and its find bar is closed).
    pub fn edit_key(
        &mut self,
        event: &crate::input::KeyPress,
        m: EditMods,
        now: Instant,
    ) -> EditOutcome {
        use winit::keyboard::{Key, NamedKey};
        let page_lines = (self.page() / LINE).floor().max(1.0) as isize;
        let Some(edit) = self.edit.as_mut() else {
            return EditOutcome::Ignored;
        };
        if event.released {
            return EditOutcome::Ignored;
        }
        let b = &mut edit.buffer;
        // a motion: with ⇧ it extends the selection from where it stood
        let mv = |b: &mut EditorState, shift: bool, f: fn(&mut EditorState)| {
            if shift {
                b.extend(f);
            } else {
                b.anchor = None;
                f(b);
            }
        };
        let mut vertical = None;
        let before = b.buffer.clone();
        match &event.logical_key {
            Key::Character(c) if m.primary() && !m.alt => {
                match (c.to_lowercase().as_str(), m.shift) {
                    ("s", false) => return EditOutcome::Save,
                    ("a", false) => b.select_all(),
                    ("z", false) => b.undo(),
                    ("z", true) => b.redo(),
                    ("c", false) => {
                        return match b.selected_text() {
                            Some(t) => EditOutcome::Copy(t.to_string()),
                            None => EditOutcome::Handled,
                        };
                    }
                    ("x", false) => {
                        let Some(t) = b.cut_selection() else {
                            return EditOutcome::Handled;
                        };
                        self.edited(now);
                        return EditOutcome::Copy(t);
                    }
                    ("v", false) => return EditOutcome::Paste,
                    _ => return EditOutcome::Ignored,
                }
            }
            Key::Named(NamedKey::Escape) => {
                if b.selection().is_some() {
                    b.anchor = None;
                } else {
                    return EditOutcome::Close;
                }
            }
            Key::Named(NamedKey::ArrowLeft) if m.cmd => mv(b, m.shift, EditorState::mv_home),
            Key::Named(NamedKey::ArrowRight) if m.cmd => mv(b, m.shift, EditorState::mv_end),
            Key::Named(NamedKey::ArrowLeft) if m.alt => mv(b, m.shift, EditorState::mv_word_left),
            Key::Named(NamedKey::ArrowRight) if m.alt => mv(b, m.shift, EditorState::mv_word_right),
            Key::Named(NamedKey::ArrowLeft) if m.shift => b.extend(EditorState::mv_left),
            Key::Named(NamedKey::ArrowRight) if m.shift => b.extend(EditorState::mv_right),
            Key::Named(NamedKey::ArrowLeft) => b.left(),
            Key::Named(NamedKey::ArrowRight) => b.right(),
            Key::Named(NamedKey::ArrowUp) if m.cmd => mv(b, m.shift, EditorState::mv_doc_start),
            Key::Named(NamedKey::ArrowDown) if m.cmd => mv(b, m.shift, EditorState::mv_doc_end),
            Key::Named(NamedKey::ArrowUp) => vertical = Some(-1),
            Key::Named(NamedKey::ArrowDown) => vertical = Some(1),
            Key::Named(NamedKey::PageUp) => vertical = Some(-page_lines),
            Key::Named(NamedKey::PageDown) => vertical = Some(page_lines),
            Key::Named(NamedKey::Home) if m.ctrl || m.cmd => {
                mv(b, m.shift, EditorState::mv_doc_start)
            }
            Key::Named(NamedKey::End) if m.ctrl || m.cmd => mv(b, m.shift, EditorState::mv_doc_end),
            Key::Named(NamedKey::Home) => mv(b, m.shift, EditorState::mv_home),
            Key::Named(NamedKey::End) => mv(b, m.shift, EditorState::mv_end),
            Key::Named(NamedKey::Backspace) if m.cmd => b.kill_to_start(),
            Key::Named(NamedKey::Backspace) if m.alt => b.delete_word_back(),
            Key::Named(NamedKey::Backspace) => b.backspace(),
            Key::Named(NamedKey::Delete) if m.alt => b.delete_word_forward(),
            Key::Named(NamedKey::Delete) => b.delete(),
            Key::Named(NamedKey::Enter) if !m.cmd && !m.ctrl => b.insert("\n"),
            Key::Named(NamedKey::Tab) if !m.cmd && !m.ctrl && !m.alt && !m.shift => b.insert(TAB),
            Key::Named(NamedKey::Space) if !m.cmd && !m.ctrl => b.insert(" "),
            _ if m.cmd || m.ctrl => return EditOutcome::Ignored,
            _ => {
                let typed: String = event
                    .text
                    .as_deref()
                    .unwrap_or("")
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                if typed.is_empty() {
                    return EditOutcome::Ignored;
                }
                b.insert(&typed);
            }
        }
        if let Some(delta) = vertical {
            let anchor = edit.buffer.anchor.or(Some(edit.buffer.cursor));
            edit.vertical(delta);
            edit.buffer.anchor = if m.shift { anchor } else { None };
        } else {
            edit.goal = None;
        }
        if edit.buffer.buffer != before {
            self.edited(now);
        } else {
            self.reveal_caret();
        }
        EditOutcome::Handled
    }
}

fn read_preview(path: &Path) -> std::io::Result<Vec<u8>> {
    use rift_core::sys::UnixMode;
    use std::io::Read;
    // Nonblocking open plus descriptor metadata closes the FIFO replacement race.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .nonblocking_open()
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("Only regular files can be previewed"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(std::io::Error::other("Too large to preview (limit 4 MB)"));
    }
    Ok(bytes)
}

fn sans(engine: &mut TextEngine, s: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        s,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

/// Draw over `rect` = [x, y, w, h]; `rel` is the path shown in the header.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &mut ViewerState,
    rel: &str,
    rect: [f32; 4],
    focused: bool,
) -> Vec<([f32; 4], ViewerHit)> {
    let [x, y, w, h] = rect;
    let mut hits = Vec::new();
    // opaque even with `window_opacity` below 100 %: the viewer covers the
    // pane's own output, which must not show through it
    layer.rect([x, y], [w, h], theme.bg, 0.0);

    // ── header (the pane's title bar, Warp's file pane): the path centred,
    // then Rendered | Raw (Markdown), ⋮ and × at the right ──
    let cy = y + HEADER_H / 2.0;
    // the shell panes' title bar shade, a file icon (accent while focused)
    layer.rect([x, y], [w, HEADER_H], Color::BLACK.with_alpha(0.08), 0.0);
    icons::draw(
        layer,
        Icon::File,
        [x + 8.0 + 7.0, cy],
        14.0,
        if focused {
            theme.accent
        } else {
            theme.text_dim
        },
    );
    // the path and the controls stay clear of the icon (x + 8 … x + 22)
    let title_left = x + 30.0;
    let close = [x + w - 28.0, cy - 12.0, 24.0, 24.0];
    let menu = [close[0] - 2.0 - 24.0, close[1], 24.0, 24.0];
    icons::draw(
        layer,
        Icon::Close,
        [close[0] + 12.0, cy],
        11.0,
        theme.text_dim,
    );
    icons::draw(
        layer,
        Icon::More,
        [menu[0] + 12.0, cy],
        14.0,
        theme.text_dim,
    );
    hits.push((close, ViewerHit::Close));
    hits.push((menu, ViewerHit::Menu));
    let mut controls_left = menu[0] - 6.0;
    if let Some(edit) = &state.edit {
        controls_left = edit_controls(
            layer,
            engine,
            theme,
            edit,
            [title_left, controls_left, cy],
            &mut hits,
        );
    }
    if !state.markdown.is_empty() {
        // segmented control: the selected segment raised, the other dim
        let labels = [("Rendered", true), ("Raw", false)];
        let seg_w: Vec<f32> = labels
            .iter()
            .map(|(l, _)| engine.measure(l, 11.5, FONT_SANS, Weight::MEDIUM) + 22.0)
            .collect();
        let total = seg_w.iter().sum::<f32>() + 4.0;
        let bx = controls_left - total;
        if bx > title_left {
            layer.bordered(
                [bx, cy - 11.0],
                [total, 22.0],
                Color::WHITE.with_alpha(0.03),
                Color::WHITE.with_alpha(0.10),
                4.0,
                1.0,
            );
            let mut sx = bx + 2.0;
            for ((label, mode), sw) in labels.iter().zip(&seg_w) {
                let on = state.rendered == *mode;
                if on {
                    layer.rect(
                        [sx, cy - 9.0],
                        [*sw, 18.0],
                        Color::WHITE.with_alpha(0.11),
                        4.5,
                    );
                }
                let tw = engine.measure(label, 11.5, FONT_SANS, Weight::MEDIUM);
                layer.text(
                    sans(engine, label, 11.5, Weight::MEDIUM),
                    (sx + (sw - tw) / 2.0).round(),
                    (cy - 7.75).round(),
                    if on {
                        theme.text_bright
                    } else {
                        theme.text_dim
                    },
                );
                hits.push(([sx, cy - 11.0, *sw, 22.0], ViewerHit::Mode(*mode)));
                sx += sw;
            }
            controls_left = bx - 8.0;
        }
    }
    // the path centres on the pane; when that would cut it, it starts at
    // the left edge instead, elided (tail kept) clear of the controls
    let mid = x + w / 2.0;
    let centred = (2.0 * (mid - title_left).min(controls_left - mid)).max(0.0);
    let full = engine.measure(rel, 12.0, FONT_SANS, Weight::NORMAL);
    let left_room = (controls_left - title_left).max(0.0);
    let room = if full <= centred { centred } else { left_room };
    let rel = super::editor::elide_head(rel, room, |t| {
        engine.measure(t, 12.0, FONT_SANS, Weight::NORMAL)
    });
    if room > 0.0 {
        let tw = engine.measure(&rel, 12.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            sans(engine, &rel, 12.0, Weight::NORMAL),
            if full <= centred {
                (mid - tw / 2.0).round()
            } else {
                title_left
            },
            (cy - 8.0).round(),
            if focused { theme.fg } else { theme.text_dim },
        );
    }
    let mut body_top = y + HEADER_H;
    if let Some(find) = &state.find {
        render_find_bar(layer, engine, theme, find, [x, body_top, w]);
        body_top += FIND_H + 1.0;
    }
    if state.edit.as_ref().is_some_and(|e| e.conflict) {
        conflict_bar(layer, engine, theme, [x, body_top, w], &mut hits);
        body_top += BAR_H + 1.0;
    }
    // the lint summary under the text
    let mut bottom = y + h;
    let summary = state
        .edit
        .as_ref()
        .and_then(ConfigEdit::summary)
        .or_else(|| state.read_only.clone());
    if let Some(summary) = summary {
        let errors = state.edit.as_ref().is_some_and(|e| e.errors() > 0);
        bottom -= BAR_H;
        status_bar(
            layer,
            engine,
            theme,
            &summary,
            errors,
            [x, bottom, w],
            &mut hits,
        );
    }
    let h = bottom - y;
    state.view_h = (y + h - body_top).max(0.0);
    state.view_w = w;
    state.body_top = body_top;
    state.scroll_by(0.0);
    state.apply_reveal();
    if let Some(e) = &state.error {
        layer.text(
            sans(engine, e, 13.0, Weight::NORMAL),
            x + 16.0,
            body_top + 16.0,
            theme.text_muted,
        );
        return hits;
    }
    if state.showing_rendered() {
        let bounds = TextBounds {
            left: x as i32,
            top: body_top as i32,
            right: (x + w) as i32,
            bottom: (y + h) as i32,
        };
        super::markdown_view::layout(&mut state.md_layout, engine, theme, &state.markdown, w);
        state.scroll_by(0.0);
        let runs = super::markdown_view::render(
            layer,
            &state.md_layout,
            theme,
            x,
            body_top - state.scroll,
            bounds,
            &state.code_scroll,
        );
        hits.extend(runs.into_iter().map(|(r, i)| (r, ViewerHit::RunCode(i))));
        scrollbar(layer, state, x, w, body_top);
        return hits;
    }

    let mono = mono_family();
    let digits = state.lines.len().max(1).to_string().len();
    let gutter = engine.measure(&"0".repeat(digits), 12.0, mono, Weight::NORMAL)
        + 28.0
        + state.edit.as_ref().map_or(0.0, |_| LINT_GUTTER);
    let bounds = TextBounds {
        left: x as i32,
        top: body_top as i32,
        right: (x + w) as i32,
        bottom: (y + h) as i32,
    };
    // horizontal scroll: the line numbers stay, the text slides under them
    if state.max_line_w.is_none() {
        let longest = state
            .lines
            .iter()
            .max_by_key(|l| l.chars().count())
            .map_or(0.0, |l| engine.measure(l, FONT, mono, Weight::NORMAL));
        state.max_line_w = Some(longest);
    }
    state.content_w = gutter + state.max_line_w.unwrap_or(0.0) + 24.0;
    state.scroll_x_by(0.0, body_top);
    state.text_x = x + gutter;
    state.gutter = gutter;
    state.char_w = engine.measure("0", FONT, mono, Weight::NORMAL);
    let sx = state.scroll_x;
    let text_bounds = TextBounds {
        left: (x + gutter - 2.0) as i32,
        ..bounds
    };
    let lang = state.lang.unwrap_or(Lang::Plain);
    let first = ((state.scroll - 12.0) / LINE).floor().max(0.0) as usize;
    let count = (state.view_h / LINE).ceil() as usize + 2;
    let color_of = |k: Kind| match k {
        Kind::Keyword => theme.syntax.command,
        Kind::String => theme.syntax.string,
        Kind::Comment => theme.text_muted,
        Kind::Number => theme.syntax.flag,
    };
    let matches: &[(usize, usize, usize)] = state.find.as_ref().map_or(&[], |f| &f.matches);
    let current = state.find.as_ref().and_then(|f| f.current);
    let selected = state.sel.map(|(a, b)| a.min(b)..=a.max(b));
    let edit = state.edit.as_ref();
    // editing: each line's start in the buffer, the selection, the caret
    let starts: Vec<usize> = edit.map_or_else(Vec::new, |e| {
        std::iter::once(0)
            .chain(e.buffer.buffer.match_indices('\n').map(|(i, _)| i + 1))
            .collect()
    });
    let text_sel = edit.and_then(|e| e.buffer.selection());
    let caret = edit.filter(|_| focused).map(|e| e.buffer.cursor);
    let text_left = x + gutter - 2.0;
    for i in first..(first + count).min(state.lines.len()) {
        let ly = body_top + 12.0 + i as f32 * LINE - state.scroll;
        if let Some(e) = edit {
            let buf = &e.buffer.buffer;
            let ls = starts.get(i).copied().unwrap_or(buf.len());
            let le = starts.get(i + 1).map_or(buf.len(), |s| s - 1);
            let raw = &buf[ls..le];
            let x_at = |engine: &mut TextEngine, byte: usize| {
                let prefix = raw[..byte.min(raw.len())].replace('\t', "    ");
                x + gutter - sx + engine.measure(&prefix, FONT, mono, Weight::NORMAL)
            };
            let (top, bot) = (ly.max(body_top), (ly + LINE).min(y + h));
            // the selection: its part of this line (the newline as a cell)
            if let Some(r) = &text_sel
                && r.start <= le
                && r.end > ls
                && bot > top
            {
                let left = x_at(engine, r.start.max(ls) - ls);
                let right = if r.end > le {
                    x_at(engine, le - ls) + state.char_w
                } else {
                    x_at(engine, r.end - ls)
                };
                let (l, rr) = (left.max(text_left), right.min(x + w));
                if rr > l {
                    layer.rect([l, top], [rr - l, bot - top], theme.selection, 0.0);
                }
            }
            // lint: a dot in the gutter, the span underlined
            let mut worst: Option<Severity> = None;
            for d in e
                .diagnostics
                .iter()
                .filter(|d| d.line.min(starts.len().saturating_sub(1)) == i)
            {
                let color = severity_color(theme, d.severity);
                let byte = |col: usize| raw.char_indices().nth(col).map_or(raw.len(), |(b, _)| b);
                let left = x_at(engine, byte(d.cols.start));
                let right = if d.cols.end > d.cols.start {
                    x_at(engine, byte(d.cols.end))
                } else {
                    left + state.char_w
                }
                .max(left + state.char_w);
                let (l, rr) = (left.max(text_left), right.min(x + w));
                let uy = ly + LINE - 3.0;
                if rr > l && uy > body_top && uy < y + h {
                    layer.hline(l, uy, rr - l, 1.5, color);
                }
                if worst != Some(Severity::Error) {
                    worst = Some(d.severity);
                }
            }
            if let Some(s) = worst
                && bot > top
            {
                if ly >= body_top - 1.0 && ly + LINE <= y + h + 1.0 {
                    layer.circle([x + 9.0, ly + LINE / 2.0], 6.0, severity_color(theme, s));
                }
                hits.push(([x, top, 16.0, bot - top], ViewerHit::Lint(i)));
            }
            // the caret
            if let Some(c) = caret
                && c >= ls
                && c <= le
            {
                let cx = x_at(engine, c - ls);
                if cx >= text_left && cx < x + w && ly + 2.0 >= body_top && ly + LINE - 2.0 <= y + h
                {
                    layer.rect([cx, ly + 2.0], [2.0, LINE - 4.0], theme.cursor, 0.0);
                }
            }
        }
        if selected.as_ref().is_some_and(|r| r.contains(&i)) {
            let top = ly.max(body_top);
            let bottom = (ly + LINE).min(y + h);
            if bottom > top {
                layer.rect(
                    [x, top],
                    [w, bottom - top],
                    theme.accent.with_alpha(0.14),
                    0.0,
                );
                layer.rect([x, top], [2.0, bottom - top], theme.accent, 0.0);
            }
        }
        // match highlights (terminal search tokens); quads draw under text
        let from = matches.partition_point(|m| m.0 < i);
        for (k, &(_, start, len)) in matches[from..].iter().take_while(|m| m.0 == i).enumerate() {
            let line = &state.lines[i];
            let mx = engine.measure(&line[..start], FONT, mono, Weight::NORMAL);
            let mw = engine.measure(&line[start..start + len], FONT, mono, Weight::NORMAL);
            let top = ly.max(body_top);
            let bottom = (ly + LINE).min(y + h);
            let left = (x + gutter + mx - sx).max(x + gutter - 2.0);
            let right = x + gutter + mx - sx + mw.max(2.0);
            if bottom > top && right > left {
                layer.rect(
                    [left, top],
                    [right - left, bottom - top],
                    if current == Some(from + k) {
                        theme.search_active
                    } else {
                        theme.search_match
                    },
                    2.0,
                );
            }
        }
        let no = (i + 1).to_string();
        let nw = engine.measure(&no, 12.0, mono, Weight::NORMAL);
        let mut nb = Buffer::new(&mut engine.font_system, Metrics::new(12.0, LINE));
        nb.set_text(
            &mut engine.font_system,
            &no,
            &Attrs::new().family(Family::Name(mono)),
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(
            nb,
            x + gutter - 14.0 - nw,
            ly + 1.0,
            theme.text_muted,
            bounds,
        );

        let line = &state.lines[i];
        let mut hl = state.states.get(i).copied().unwrap_or_default();
        let spans = code_highlight::line(lang, &mut hl, line);
        let base = Attrs::new()
            .family(Family::Name(mono))
            .color(text_color(theme.fg));
        let mut colored: Vec<(std::ops::Range<usize>, Color)> = Vec::new();
        let mut at = 0;
        for (r, k) in &spans {
            if r.start > at {
                colored.push((at..r.start, theme.fg));
            }
            colored.push((r.clone(), color_of(*k)));
            at = r.end;
        }
        if at < line.len() {
            colored.push((at..line.len(), theme.fg));
        }
        // dark text on a match highlight, like terminal search
        let line_matches: Vec<std::ops::Range<usize>> = matches
            [matches.partition_point(|m| m.0 < i)..]
            .iter()
            .take_while(|m| m.0 == i)
            .map(|&(_, s, l)| s..s + l)
            .collect();
        let colored = recolor(colored, &line_matches, Color::hex("#1e1e1e").unwrap());
        let rich: Vec<(&str, Attrs)> = colored
            .iter()
            .map(|(r, c)| (&line[r.clone()], base.clone().color(text_color(*c))))
            .collect();
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(FONT, LINE));
        buf.set_rich_text(
            &mut engine.font_system,
            rich,
            &base,
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(buf, x + gutter - sx, ly, theme.fg, text_bounds);
    }
    scrollbar(layer, state, x, w, body_top);
    hits
}

/// Scroll thumb at the right edge while the content overflows.
fn scrollbar(layer: &mut Layer, state: &ViewerState, x: f32, w: f32, body_top: f32) {
    let content = state.content_h();
    if content > state.view_h {
        let track = state.view_h - 12.0;
        let thumb = (track * state.view_h / content).max(24.0);
        let max = content - state.view_h;
        let ty = body_top + 6.0 + (track - thumb) * (state.scroll / max);
        layer.rect(
            [x + w - 7.0, ty],
            [3.0, thumb],
            Color::WHITE.with_alpha(0.18),
            1.5,
        );
    }
}

fn severity_color(theme: &Theme, s: Severity) -> Color {
    match s {
        Severity::Error => theme.error,
        Severity::Warning => theme.warning,
    }
}

/// A 22 px header button (the Rendered | Raw control's metrics): filled
/// (`primary`) or outlined; `enabled` false dims it.
#[allow(clippy::too_many_arguments)]
fn header_button(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    label: &str,
    right: f32,
    cy: f32,
    primary: bool,
    enabled: bool,
) -> [f32; 4] {
    let tw = engine.measure(label, 11.5, FONT_SANS, Weight::MEDIUM);
    let bw = tw + 22.0;
    let r = [right - bw, cy - 11.0, bw, 22.0];
    let ink = if primary && enabled {
        layer.rect([r[0], r[1]], [r[2], r[3]], theme.accent, 4.0);
        theme.bg
    } else {
        layer.bordered(
            [r[0], r[1]],
            [r[2], r[3]],
            Color::WHITE.with_alpha(0.03),
            Color::WHITE.with_alpha(0.10),
            4.0,
            1.0,
        );
        if enabled {
            theme.text_bright
        } else {
            theme.text_muted
        }
    };
    layer.text(
        sans(engine, label, 11.5, Weight::MEDIUM),
        (r[0] + (bw - tw) / 2.0).round(),
        (cy - 7.75).round(),
        ink,
    );
    r
}

/// config.toml's header controls left of ⋮: Save and Dismiss while the
/// text differs from the file, "Saved" briefly after a save. Returns the
/// new right edge for the path. `at` = [title_left, right, cy].
fn edit_controls(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    edit: &ConfigEdit,
    [title_left, right, cy]: [f32; 3],
    hits: &mut Vec<([f32; 4], ViewerHit)>,
) -> f32 {
    let mut right = right;
    if edit.dirty() {
        let enabled = edit.can_save();
        let need =
            engine.measure("SaveDismiss", 11.5, FONT_SANS, Weight::MEDIUM) + 22.0 * 2.0 + 6.0;
        if right - need < title_left {
            return right;
        }
        let save = header_button(layer, engine, theme, "Save", right, cy, true, enabled);
        hits.push((save, ViewerHit::Save));
        right = save[0] - 6.0;
        let dismiss = header_button(layer, engine, theme, "Dismiss", right, cy, false, true);
        hits.push((dismiss, ViewerHit::Dismiss));
        right = dismiss[0] - 8.0;
    } else if edit.saved_at.is_some() {
        let tw = engine.measure("Saved", 11.5, FONT_SANS, Weight::MEDIUM);
        if right - tw - 20.0 >= title_left {
            let tx = right - tw - 2.0;
            layer.text(
                sans(engine, "Saved", 11.5, Weight::MEDIUM),
                tx.round(),
                (cy - 7.75).round(),
                theme.text_dim,
            );
            icons::draw(layer, Icon::Check, [tx - 9.0, cy], 11.0, theme.success);
            right = tx - 18.0;
        }
    }
    right
}

/// The strip under the header while the file changed on disk under
/// unsaved edits: what happened, Keep mine and Reload.
fn conflict_bar(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    [x, y, w]: [f32; 3],
    hits: &mut Vec<([f32; 4], ViewerHit)>,
) {
    layer.rect([x, y], [w, BAR_H], theme.bg_chrome, 0.0);
    let cy = y + BAR_H / 2.0;
    let reload = header_button(layer, engine, theme, "Reload", x + w - 10.0, cy, true, true);
    let keep = header_button(
        layer,
        engine,
        theme,
        "Keep mine",
        reload[0] - 6.0,
        cy,
        false,
        true,
    );
    hits.push((reload, ViewerHit::Reload));
    hits.push((keep, ViewerHit::KeepMine));
    layer.circle([x + 14.0, cy], 6.0, theme.warning);
    let room = keep[0] - 8.0 - (x + 24.0);
    let text = super::tabbar::elide(
        engine,
        "config.toml changed outside this pane",
        12.0,
        room.max(0.0),
    );
    if room > 0.0 {
        layer.text(
            sans(engine, &text, 12.0, Weight::NORMAL),
            x + 24.0,
            (cy - 8.0).round(),
            theme.text_ui,
        );
    }
    layer.hline(x, y + BAR_H, w, 1.0, theme.border);
}

/// The lint summary under the text: a dot of the worst severity and
/// "2 errors · 1 warning — line 4: …"; a click goes to the next problem.
fn status_bar(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    summary: &str,
    errors: bool,
    [x, y, w]: [f32; 3],
    hits: &mut Vec<([f32; 4], ViewerHit)>,
) {
    layer.rect([x, y], [w, BAR_H], theme.bg_chrome, 0.0);
    layer.hline(x, y, w, 1.0, theme.border);
    let cy = y + BAR_H / 2.0;
    layer.circle(
        [x + 14.0, cy],
        6.0,
        if errors { theme.error } else { theme.warning },
    );
    let room = (w - 24.0 - 12.0).max(0.0);
    let text = super::tabbar::elide(engine, summary, 12.0, room);
    layer.text(
        sans(engine, &text, 12.0, Weight::NORMAL),
        x + 24.0,
        (cy - 8.0).round(),
        theme.text_ui,
    );
    hits.push(([x, y, w, BAR_H], ViewerHit::Status));
}

/// Find strip under the header: field with magnifier, query + caret, counter
/// (same tokens as the terminal search bar).
fn render_find_bar(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    find: &ViewerFind,
    [x, y, w]: [f32; 3],
) {
    layer.rect([x, y], [w, FIND_H], theme.bg_chrome, 0.0);
    let field = [x + 12.0, y + 6.0, (w - 24.0).max(0.0), FIND_H - 12.0];
    layer.bordered(
        [field[0], field[1]],
        [field[2], field[3]],
        theme.surface,
        Color::rgba(1.0, 1.0, 1.0, 0.11),
        4.0,
        1.0,
    );
    let my = field[1] + (field[3] - 11.0) / 2.0;
    layer.bordered(
        [field[0] + 10.0, my],
        [11.0, 11.0],
        Color::TRANSPARENT,
        theme.text_dim,
        4.0,
        1.6,
    );
    layer.rect([field[0] + 19.5, my + 9.5], [5.0, 1.7], theme.text_dim, 1.0);

    let counter = find.counter();
    let cw = engine.measure(&counter, 11.5, FONT_SANS, Weight::NORMAL);
    let right = field[0] + field[2] - 10.0;
    layer.text(
        sans(engine, &counter, 11.5, Weight::NORMAL),
        right - cw,
        field[1] + (field[3] - 15.5) / 2.0,
        theme.text_muted,
    );

    let qx = field[0] + 32.0;
    let q_right = right - cw - 8.0;
    let avail = (q_right - qx).max(0.0);
    let mono = mono_family();
    let qw = engine.measure(&find.query, 13.0, mono, Weight::NORMAL);
    let shift = crate::views::search::query_scroll(qw + 3.0, avail);
    if !find.query.is_empty() {
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(13.0, 18.0));
        buf.set_text(
            &mut engine.font_system,
            &find.query,
            &Attrs::new().family(Family::Name(mono)),
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(
            buf,
            qx - shift,
            field[1] + (field[3] - 18.0) / 2.0,
            theme.text_bright,
            TextBounds {
                left: qx.floor() as i32,
                top: field[1] as i32,
                right: q_right.ceil() as i32,
                bottom: (field[1] + field[3]) as i32,
            },
        );
    }
    if avail > 0.0 {
        layer.rect(
            [qx + qw - shift + 1.0, field[1] + (field[3] - 16.0) / 2.0],
            [2.0, 16.0],
            theme.accent,
            0.0,
        );
    }
    layer.hline(x, y + FIND_H, w, 1.0, theme.border);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::KeyPress;
    use winit::keyboard::{Key, NamedKey};

    fn editing(text: &str) -> ViewerState {
        let mut v = ViewerState {
            path: Some(PathBuf::from("/cfg/config.toml")),
            raw: Some(text.to_string()),
            view_h: 200.0,
            view_w: 400.0,
            ..Default::default()
        };
        v.start_edit(None);
        v
    }

    fn buf(v: &ViewerState) -> &str {
        &v.edit.as_ref().unwrap().buffer.buffer
    }

    fn key(v: &mut ViewerState, k: KeyPress, m: EditMods) -> EditOutcome {
        v.edit_key(&k, m, Instant::now())
    }

    const PLAIN: EditMods = EditMods {
        shift: false,
        alt: false,
        ctrl: false,
        cmd: false,
    };
    const SHIFT: EditMods = EditMods {
        shift: true,
        ..PLAIN
    };
    const ALT: EditMods = EditMods { alt: true, ..PLAIN };

    /// The platform's ⌘ (Ctrl off macOS).
    fn primary(shift: bool) -> EditMods {
        EditMods {
            shift,
            cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            ..PLAIN
        }
    }

    #[test]
    fn config_edit_keys_type_move_select_and_undo() {
        let mut v = editing("a = 1\nlong_key = 2\n");
        assert_eq!(
            v.lines,
            ["a = 1", "long_key = 2", ""],
            "the empty last line stays"
        );
        assert!(!v.dirty());
        // typing at the start, Backspace, Enter, Tab
        assert_eq!(key(&mut v, text("x"), PLAIN), EditOutcome::Handled);
        assert_eq!(buf(&v), "xa = 1\nlong_key = 2\n");
        assert!(v.dirty());
        key(&mut v, named(NamedKey::Backspace), PLAIN);
        assert!(!v.dirty(), "back to the file's text: clean");
        key(&mut v, named(NamedKey::Enter), PLAIN);
        key(&mut v, named(NamedKey::Tab), PLAIN);
        assert_eq!(buf(&v), "\n  a = 1\nlong_key = 2\n");
        assert_eq!(v.lines[1], "  a = 1");
        // ↑ / ↓ keep the column across a shorter line
        key(&mut v, named(NamedKey::End), PLAIN);
        key(&mut v, named(NamedKey::ArrowDown), PLAIN);
        assert_eq!(v.edit.as_ref().unwrap().caret(), (2, 7));
        key(&mut v, named(NamedKey::ArrowDown), PLAIN);
        assert_eq!(
            v.edit.as_ref().unwrap().caret(),
            (3, 0),
            "the empty last line"
        );
        key(&mut v, named(NamedKey::ArrowUp), PLAIN);
        assert_eq!(
            v.edit.as_ref().unwrap().caret(),
            (2, 7),
            "the goal column again"
        );
        // ⇧ selects; ⌥← moves by word; ⌘C copies, ⌘X cuts
        key(&mut v, named(NamedKey::Home), PLAIN);
        key(&mut v, named(NamedKey::ArrowRight), ALT);
        assert_eq!(v.edit.as_ref().unwrap().caret(), (2, 8));
        key(&mut v, named(NamedKey::Home), SHIFT);
        assert_eq!(
            key(&mut v, text("c"), primary(false)),
            EditOutcome::Copy("long_key".into())
        );
        assert_eq!(
            key(&mut v, text("x"), primary(false)),
            EditOutcome::Copy("long_key".into())
        );
        assert_eq!(buf(&v), "\n  a = 1\n = 2\n");
        // ⌘Z undoes the cut, ⇧⌘Z does it again
        key(&mut v, text("z"), primary(false));
        assert_eq!(buf(&v), "\n  a = 1\nlong_key = 2\n");
        key(&mut v, text("z"), primary(true));
        assert_eq!(buf(&v), "\n  a = 1\n = 2\n");
        // ⌘A, then a key replaces it all
        key(&mut v, text("a"), primary(false));
        key(&mut v, text("q"), PLAIN);
        assert_eq!(buf(&v), "q");
        assert_eq!(v.lines, ["q"]);
        // ⌘S and ⌘V go to the app; Esc drops a selection, then closes
        assert_eq!(key(&mut v, text("s"), primary(false)), EditOutcome::Save);
        assert_eq!(key(&mut v, text("v"), primary(false)), EditOutcome::Paste);
        key(&mut v, named(NamedKey::Home), SHIFT);
        assert_eq!(
            key(&mut v, named(NamedKey::Escape), PLAIN),
            EditOutcome::Handled
        );
        assert_eq!(v.edit.as_ref().unwrap().buffer.selection(), None);
        assert_eq!(
            key(&mut v, named(NamedKey::Escape), PLAIN),
            EditOutcome::Close
        );
        // other chords stay app shortcuts
        assert_eq!(key(&mut v, text("t"), primary(false)), EditOutcome::Ignored);
        // a paste: CRLF as LF
        key(&mut v, named(NamedKey::End), PLAIN);
        v.edit_insert("\r\nb = 3", Instant::now());
        assert_eq!(buf(&v), "q\nb = 3");
    }

    #[test]
    fn a_clean_config_pane_follows_the_file_a_dirty_one_flags_a_conflict() {
        let mut v = editing("a = 1\n");
        v.scroll = 0.0;
        // clean: the file's new text replaces the buffer, caret kept
        v.edit.as_mut().unwrap().buffer.cursor = 3;
        assert!(v.disk_changed("a = 1\nb = 2\n".into()));
        assert_eq!(buf(&v), "a = 1\nb = 2\n");
        assert_eq!(v.lines.len(), 3);
        assert_eq!(v.edit.as_ref().unwrap().buffer.cursor, 3);
        assert!(!v.dirty() && !v.edit.as_ref().unwrap().conflict);
        assert!(
            !v.disk_changed("a = 1\nb = 2\n".into()),
            "the same text: nothing"
        );
        // dirty: never replaced; a conflict
        v.edit_insert("# mine\n", Instant::now());
        let mine = buf(&v).to_string();
        assert!(v.disk_changed("a = 9\n".into()));
        assert_eq!(buf(&v), mine, "unsaved edits are never replaced");
        let e = v.edit.as_ref().unwrap();
        assert!(e.conflict && e.dirty());
        // Keep mine, then a save makes the file the buffer: clean
        v.edit.as_mut().unwrap().conflict = false;
        v.edit.as_mut().unwrap().saved(Instant::now());
        assert!(!v.dirty());
        // our own write read back by the watcher: no conflict
        v.edit_insert("c = 3\n", Instant::now());
        let text = buf(&v).to_string();
        assert!(v.disk_changed(text));
        assert!(!v.edit.as_ref().unwrap().conflict && !v.dirty());
        // Reload / Dismiss: the file's text again
        v.edit_insert("junk", Instant::now());
        assert!(v.disk_changed("z = 1\n".into()));
        v.revert();
        assert_eq!(buf(&v), "z = 1\n");
        let e = v.edit.as_ref().unwrap();
        assert!(!e.conflict && !e.dirty());
    }

    #[test]
    fn save_is_shut_while_lint_finds_an_error_and_lint_waits_for_a_pause() {
        let mut v = editing("font_size = 13\n");
        assert!(
            !v.edit.as_ref().unwrap().can_save(),
            "clean: nothing to save"
        );
        let t0 = Instant::now();
        v.edit.as_mut().unwrap().buffer.cursor = 4;
        v.edit_key(&text("s"), PLAIN, t0);
        let e = v.edit.as_ref().unwrap();
        assert_eq!(e.buffer.buffer, "fonts_size = 13\n");
        assert!(e.diagnostics.is_empty() && e.lint_due == Some(t0 + LINT_DELAY));
        // not yet due: no lint; then the error, Save shut
        assert_eq!(v.tick(t0 + LINT_DELAY / 2), (false, Some(t0 + LINT_DELAY)));
        assert!(v.tick(t0 + LINT_DELAY).0, "due: it lints");
        let e = v.edit.as_ref().unwrap();
        assert_eq!(e.errors(), 1);
        assert_eq!(e.diagnostics[0].line, 0);
        assert!(!e.can_save());
        assert!(
            e.summary().unwrap().starts_with("1 error — line 1: "),
            "{:?}",
            e.summary()
        );
        // a removed key only warns: Save stays open
        v.edit_insert("\nbackground_processes = true", Instant::now());
        let e = v.edit.as_mut().unwrap();
        e.buffer.buffer = "font_size = 13\nbackground_processes = true\n".into();
        e.lint();
        assert_eq!((e.errors(), e.warnings()), (0, 1));
        assert!(e.can_save());
        assert_eq!(
            e.summary().unwrap(),
            format!("1 warning — line 2: {}", e.diagnostics[0].message)
        );
        // the status bar walks the problems, wrapping
        e.buffer.buffer = "a = [\nfont_sise = 1\n".into();
        e.lint();
        assert!(e.jump_next());
        let first = e.caret();
        e.jump_next();
        e.jump_next();
        assert_eq!(e.caret(), first, "wraps to the first");
    }

    #[test]
    fn a_click_places_the_caret_by_cell_and_a_missing_file_edits_empty() {
        let mut v = editing("a\tb\nxyz\n");
        v.body_top = 100.0;
        v.text_x = 50.0;
        v.char_w = 10.0;
        let y0 = 100.0 + 12.0 + 5.0;
        // a tab spans four cells: its far half lands after it
        assert_eq!(v.edit_offset_at(50.0 + 14.0, y0), Some(1));
        assert_eq!(v.edit_offset_at(50.0 + 34.0, y0), Some(2));
        assert_eq!(v.edit_offset_at(50.0 + 500.0, y0), Some(3), "past the end");
        assert_eq!(
            v.edit_offset_at(0.0, y0 + LINE),
            Some(4),
            "left of the text"
        );
        assert_eq!(
            v.edit_offset_at(60.0, y0 + 9.0 * LINE),
            Some(8),
            "below: the end"
        );
        assert_eq!(v.edit_offset_at(60.0, 90.0), None, "above the text");
        // a press and a drag select
        assert!(v.edit_press(50.0 + 11.0, y0 + LINE, false));
        assert!(v.edit_drag(50.0 + 30.0, y0 + LINE));
        assert_eq!(v.edit.as_ref().unwrap().buffer.selected_text(), Some("yz"));
        v.edit.as_mut().unwrap().dragging = false;
        assert!(!v.edit_drag(50.0, y0), "no drag after the release");
        // config.toml not there yet: an empty buffer (Save creates it)
        let mut v = ViewerState {
            path: Some(PathBuf::from("/nonexistent-rift-test/config.toml")),
            error: Some("No such file or directory".into()),
            ..Default::default()
        };
        v.start_edit(None);
        assert!(v.error.is_none());
        assert_eq!(buf(&v), "");
        // a file that is not UTF-8 (no exact text) stays read-only
        let mut v = viewer(&["x"]);
        v.path = Some(std::env::temp_dir());
        v.start_edit(None);
        assert!(v.edit.is_none());
    }

    #[test]
    fn recolor_splits_runs_at_matches() {
        let (a, b, dark) = (
            Color::rgba(1.0, 0.0, 0.0, 1.0),
            Color::rgba(0.0, 1.0, 0.0, 1.0),
            Color::rgba(0.0, 0.0, 0.0, 1.0),
        );
        let out = recolor(vec![(0..4, a), (4..10, b)], &[2..6, 8..9], dark);
        assert_eq!(
            out,
            vec![
                (0..2, a),
                (2..4, dark),
                (4..6, dark),
                (6..8, b),
                (8..9, dark),
                (9..10, b)
            ]
        );
        assert_eq!(recolor(vec![(0..3, a)], &[], dark), vec![(0..3, a)]);
    }

    fn viewer(lines: &[&str]) -> ViewerState {
        ViewerState {
            path: Some(PathBuf::from("/x.txt")),
            lines: lines.iter().map(|s| s.to_string()).collect(),
            view_h: 100.0,
            ..Default::default()
        }
    }

    fn text(s: &str) -> KeyPress {
        KeyPress::new(Key::Character(s.into()), Some(s.into()), None)
    }

    fn named(k: NamedKey) -> KeyPress {
        KeyPress::new(Key::Named(k), None, None)
    }

    #[test]
    fn rendered_markdown_sends_only_a_selection_never_its_scroll_line() {
        let src: Vec<String> = (1..=60).map(|i| format!("## H{i}\n\ntext {i}")).collect();
        let mut v = viewer(&[]);
        v.lines = src.join("\n\n").lines().map(str::to_string).collect();
        v.markdown = rift_core::markdown::parse(&v.lines);
        v.rendered = true;
        v.scroll = 400.0;
        assert!(v.showing_rendered());
        assert_eq!(v.send_range(), None, "rendered scroll is no source line");
        v.sel = Some((4, 2));
        assert_eq!(v.send_range(), Some((2, 4)));
        v.sel = None;
        v.set_rendered(false);
        v.scroll = 400.0;
        assert_eq!(v.send_range(), Some((v.top_line(), v.top_line())));
    }

    #[test]
    fn find_matches_case_insensitive_multi_and_unicode() {
        let lines: Vec<String> = ["Foo foo FOO", "bar", "ÄpfelÄ äpfel", "aaaa"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            find_matches(&lines, "foo"),
            [(0, 0, 3), (0, 4, 3), (0, 8, 3)]
        );
        // multi-byte: ranges land on char boundaries and slice back to the hit
        let hits = find_matches(&lines, "äpfel");
        assert_eq!(hits, [(2, 0, 6), (2, 9, 6)]);
        for &(l, s, n) in &hits {
            assert!(lines[l][s..s + n].to_lowercase() == "äpfel");
        }
        assert_eq!(find_matches(&lines, "ä"), [(2, 0, 2), (2, 6, 2), (2, 9, 2)]);
        // non-overlapping
        assert_eq!(find_matches(&lines, "aa"), [(3, 0, 2), (3, 2, 2)]);
        assert!(find_matches(&lines, "").is_empty());
        assert!(find_matches(&lines, "zzz").is_empty());
    }

    #[test]
    fn find_keys_navigate_wrap_and_close() {
        let mut v = viewer(&["alpha", "beta", "ALPHA beta alpha"]);
        assert!(
            !v.handle_find_key(&text("a"), false),
            "closed find consumes nothing"
        );
        v.open_find();
        assert_eq!(v.find.as_ref().unwrap().counter(), "Find in file");
        for c in ["a", "l", "p"] {
            assert!(v.handle_find_key(&text(c), false));
        }
        let f = v.find.as_ref().unwrap();
        assert_eq!(f.query, "alp");
        assert_eq!(f.matches, [(0, 0, 3), (2, 0, 3), (2, 11, 3)]);
        assert_eq!(f.counter(), "1/3");
        assert!(v.handle_find_key(&named(NamedKey::Enter), false));
        assert!(v.handle_find_key(&named(NamedKey::Enter), false));
        assert_eq!(v.find.as_ref().unwrap().current, Some(2));
        assert!(v.handle_find_key(&named(NamedKey::Enter), false));
        assert_eq!(v.find.as_ref().unwrap().current, Some(0), "wraps forward");
        assert!(v.handle_find_key(&named(NamedKey::Enter), true));
        assert_eq!(v.find.as_ref().unwrap().current, Some(2), "wraps backward");
        assert!(v.handle_find_key(&text("z"), false));
        assert_eq!(v.find.as_ref().unwrap().counter(), "No matches");
        assert!(v.handle_find_key(&named(NamedKey::Backspace), false));
        assert_eq!(v.find.as_ref().unwrap().matches.len(), 3);
        // unrelated keys fall through to viewer scrolling
        assert!(!v.handle_find_key(&named(NamedKey::ArrowDown), false));
        assert!(v.handle_find_key(&named(NamedKey::Escape), false));
        assert!(v.find.is_none());
        assert!(v.is_open(), "Esc closes find, not the viewer");
    }

    #[test]
    fn find_selects_from_viewport_and_scrolls_match_into_view() {
        let mut lines = vec!["x"; 200];
        lines[5] = "needle";
        lines[150] = "needle";
        let mut v = viewer(&lines);
        v.scroll = 12.0 + 100.0 * LINE; // top line 100
        v.open_find();
        for c in "need".chars() {
            v.handle_find_key(&text(&c.to_string()), false);
        }
        let f = v.find.as_ref().unwrap();
        assert_eq!(f.current, Some(1), "first match at/after the viewport");
        let top = 12.0 + 150.0 * LINE;
        assert!(v.scroll <= top && top + LINE <= v.scroll + v.view_h);
        v.handle_find_key(&named(NamedKey::Enter), false);
        let top = 12.0 + 5.0 * LINE;
        assert!(v.scroll <= top && top + LINE <= v.scroll + v.view_h);
        // a new file resets find
        v.close();
        assert!(v.find.is_none());
        v.open_find();
        assert!(v.find.is_none(), "no find without an open file");
    }

    #[test]
    fn a_requested_line_survives_the_load_and_scrolls_into_view() {
        // what open_viewer_at leaves: the file loading, a line pending
        let mut v = ViewerState {
            path: Some(PathBuf::from("/x.md")),
            generation: 7,
            reveal_line: Some(149),
            ..Default::default()
        };
        let mut loaded = viewer(&vec!["x"; 200]);
        loaded.generation = 7;
        loaded.rendered = true;
        assert!(v.accept(loaded));
        assert_eq!(v.reveal_line, Some(149), "kept across the load");
        assert!(!v.rendered, "a line of a Markdown file shows its source");
        v.apply_reveal();
        let top = 12.0 + 149.0 * LINE;
        assert!(
            v.scroll <= top && top + LINE <= v.scroll + v.view_h,
            "{}",
            v.scroll
        );
        assert_eq!(v.reveal_line, None, "applied once");
        // past the end: the last line
        v.reveal_line = Some(10_000);
        v.apply_reveal();
        assert!(v.scroll + v.view_h >= 12.0 + 200.0 * LINE);
    }

    #[test]
    #[cfg(unix)]
    fn preview_rejects_special_and_oversized_files_and_loads_text() {
        let dir = std::env::temp_dir().join(format!("rift-preview-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_preview(&fifo).is_err());
        assert!(read_preview(&dir).is_err());
        let big = dir.join("large");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(read_preview(&big).is_err());
        let text = dir.join("a.rs");
        std::fs::write(&text, "fn main() {}\n").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut viewer = ViewerState::default();
        viewer.open(&text, move |loaded| {
            tx.send(loaded).unwrap();
        });
        let loaded = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(viewer.accept(loaded));
        assert_eq!(viewer.lines, ["fn main() {}"]);
        assert!(viewer.error.is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        viewer.open(&text, move |loaded| {
            tx.send(loaded).unwrap();
        });
        viewer.close();
        assert!(!viewer.accept(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()));
        assert!(!viewer.is_open());
        assert!(
            !viewer.showing_rendered(),
            "source files have no rendered view"
        );
        // Markdown opens rendered; find (on the source lines) switches to raw
        let md = dir.join("README.md");
        std::fs::write(&md, "# Title\n\nbody\n").unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        viewer.open(&md, move |loaded| {
            tx.send(loaded).unwrap();
        });
        assert!(viewer.accept(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap()));
        assert!(viewer.showing_rendered());
        assert_eq!(viewer.markdown.len(), 2);
        viewer.scroll = 40.0;
        viewer.set_rendered(false);
        assert_eq!(viewer.scroll, 0.0, "switching views starts at the top");
        viewer.set_rendered(true);
        // sideways scroll of the source view stops at both ends
        viewer.rendered = false;
        viewer.content_w = 900.0;
        viewer.view_w = 600.0;
        viewer.scroll_x_by(250.0, 0.0);
        assert_eq!(viewer.scroll_x, 250.0);
        viewer.scroll_x_by(500.0, 0.0);
        assert_eq!(viewer.scroll_x, 300.0, "clamped to the widest line");
        viewer.scroll_x_by(-1000.0, 0.0);
        assert_eq!(viewer.scroll_x, 0.0);
        viewer.rendered = true;
        viewer.open_find();
        assert!(!viewer.rendered && viewer.find.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
