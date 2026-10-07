//! Read-only file viewer: opens from the explorer or the review panel over
//! the active tab's pane area. Line numbers, lightweight highlighting
//! (`shift_core::code_highlight`), only the lines on screen are shaped.
//! Esc or ✕ closes it; the terminal underneath keeps running. ⌘F opens a
//! find bar scoped to the open file (case-insensitive plain text).

use std::path::{Path, PathBuf};

use shift_core::code_highlight::{self, Kind, Lang, State};
use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

/// Files past this are not loaded.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;
pub const HEADER_H: f32 = 38.0;
const FONT: f32 = 13.0;
const LINE: f32 = 20.0;
/// Height of the find strip under the header while find is open.
const FIND_H: f32 = 40.0;

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
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerHit {
    Close,
    InsertPath,
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

    pub fn accept(&mut self, loaded: ViewerState) -> bool {
        if self.path.is_none() || self.generation != loaded.generation {
            return false;
        }
        *self = loaded;
        true
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
                }
            },
        }
    }

    pub fn content_h(&self) -> f32 {
        self.lines.len() as f32 * LINE + 24.0
    }

    pub fn scroll_by(&mut self, px: f32) {
        let max = (self.content_h() - self.view_h).max(0.0);
        self.scroll = (self.scroll + px).clamp(0.0, max);
    }

    pub fn page(&self) -> f32 {
        (self.view_h - LINE * 2.0).max(LINE)
    }

    /// Open (or keep) the find bar; a no-op while no file is open.
    pub fn open_find(&mut self) {
        if self.is_open() && self.find.is_none() {
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
                find.query.pop();
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

fn read_preview(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    // Nonblocking open plus descriptor metadata closes the FIFO replacement race.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
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
) -> Vec<([f32; 4], ViewerHit)> {
    let [x, y, w, h] = rect;
    let mut hits = Vec::new();
    layer.rect([x, y], [w, h], theme.bg, 0.0);

    // ── header: path, read-only chip, insert path, close ──
    layer.rect([x, y], [w, HEADER_H], theme.bg_chrome, 0.0);
    layer.text(
        sans(engine, rel, 12.5, Weight::MEDIUM),
        x + 16.0,
        y + 11.0,
        theme.text_bright,
    );
    let rel_w = engine.measure(rel, 12.5, FONT_SANS, Weight::MEDIUM);
    let chip = "read-only";
    let chip_w = engine.measure(chip, 10.5, FONT_SANS, Weight::MEDIUM) + 12.0;
    layer.rect(
        [x + 16.0 + rel_w + 10.0, y + 10.0],
        [chip_w, 18.0],
        Color::WHITE.with_alpha(0.06),
        5.0,
    );
    layer.text(
        sans(engine, chip, 10.5, Weight::MEDIUM),
        x + 16.0 + rel_w + 16.0,
        y + 12.0,
        theme.text_muted,
    );
    let close = [x + w - 36.0, y + 7.0, 24.0, 24.0];
    layer.text(
        sans(engine, "✕", 12.0, Weight::NORMAL),
        close[0] + 7.0,
        close[1] + 4.0,
        theme.text_muted,
    );
    hits.push((close, ViewerHit::Close));
    let insert_label = "Insert path";
    let iw = engine.measure(insert_label, 11.0, FONT_SANS, Weight::MEDIUM) + 14.0;
    let insert = [close[0] - 8.0 - iw, y + 9.0, iw, 20.0];
    layer.rect(
        [insert[0], insert[1]],
        [insert[2], insert[3]],
        Color::WHITE.with_alpha(0.06),
        5.0,
    );
    layer.text(
        sans(engine, insert_label, 11.0, Weight::MEDIUM),
        insert[0] + 7.0,
        insert[1] + 3.0,
        theme.text_ui,
    );
    hits.push((insert, ViewerHit::InsertPath));
    layer.hline(x, y + HEADER_H, w, 1.0, theme.border);

    let mut body_top = y + HEADER_H + 1.0;
    if let Some(find) = &state.find {
        render_find_bar(layer, engine, theme, find, [x, body_top, w]);
        body_top += FIND_H + 1.0;
    }
    state.view_h = (y + h - body_top).max(0.0);
    state.scroll_by(0.0);
    if let Some(e) = &state.error {
        layer.text(
            sans(engine, e, 13.0, Weight::NORMAL),
            x + 16.0,
            body_top + 16.0,
            theme.text_muted,
        );
        return hits;
    }

    let mono = mono_family();
    let digits = state.lines.len().max(1).to_string().len();
    let gutter = engine.measure(&"0".repeat(digits), 12.0, mono, Weight::NORMAL) + 28.0;
    let bounds = TextBounds {
        left: x as i32,
        top: body_top as i32,
        right: (x + w) as i32,
        bottom: (y + h) as i32,
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
    for i in first..(first + count).min(state.lines.len()) {
        let ly = body_top + 12.0 + i as f32 * LINE - state.scroll;
        // match highlights (terminal search tokens); quads draw under text
        let from = matches.partition_point(|m| m.0 < i);
        for (k, &(_, start, len)) in matches[from..].iter().take_while(|m| m.0 == i).enumerate() {
            let line = &state.lines[i];
            let mx = engine.measure(&line[..start], FONT, mono, Weight::NORMAL);
            let mw = engine.measure(&line[start..start + len], FONT, mono, Weight::NORMAL);
            let top = ly.max(body_top);
            let bottom = (ly + LINE).min(y + h);
            if bottom > top {
                layer.rect(
                    [x + gutter + mx, top],
                    [mw.max(2.0), bottom - top],
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
        layer.text_clipped(buf, x + gutter, ly, theme.fg, bounds);
    }

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
    hits
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
        6.0,
        1.0,
    );
    let my = field[1] + (field[3] - 11.0) / 2.0;
    layer.bordered(
        [field[0] + 10.0, my],
        [11.0, 11.0],
        Color::TRANSPARENT,
        theme.text_dim,
        5.5,
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
    fn preview_rejects_special_and_oversized_files_and_loads_text() {
        let dir = std::env::temp_dir().join(format!("shift-preview-{}", std::process::id()));
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
        std::fs::remove_dir_all(dir).unwrap();
    }
}
