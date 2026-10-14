//! Read-only file viewer: opens from the explorer or the review panel as a
//! pane of the active tab's split. Line numbers, lightweight highlighting
//! (`rift_core::code_highlight`), only the lines on screen are shaped.
//! Markdown files open rendered (`markdown_view`), with a Rendered | Raw
//! switch in the header.
//! Esc or ✕ closes it; the terminal underneath keeps running. ⌘F opens a
//! find bar scoped to the open file (case-insensitive plain text).

use std::path::{Path, PathBuf};

use crate::views::icons::{self, Icon};
use rift_core::code_highlight::{self, Kind, Lang, State};
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerHit {
    Close,
    /// The ⋮ button: the viewer menu (insert path, copy path, find).
    Menu,
    /// Rendered | Raw segments (Markdown files).
    Mode(bool),
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
    state.view_h = (y + h - body_top).max(0.0);
    state.view_w = w;
    state.body_top = body_top;
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
    if state.showing_rendered() {
        let bounds = TextBounds {
            left: x as i32,
            top: body_top as i32,
            right: (x + w) as i32,
            bottom: (y + h) as i32,
        };
        super::markdown_view::layout(&mut state.md_layout, engine, theme, &state.markdown, w);
        state.scroll_by(0.0);
        super::markdown_view::render(
            layer,
            &state.md_layout,
            theme,
            x,
            body_top - state.scroll,
            bounds,
            &state.code_scroll,
        );
        scrollbar(layer, state, x, w, body_top);
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
