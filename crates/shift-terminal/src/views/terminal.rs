use std::time::Duration;

use shift_core::{Block, BlockId, BlockState, Color, CursorStyle, Theme};
use shift_pty::alacritty_terminal::grid::Dimensions;
use shift_pty::alacritty_terminal::index::{Column, Line, Point};
use shift_pty::alacritty_terminal::selection::SelectionRange;
use shift_pty::alacritty_terminal::term::TermMode;
use shift_pty::alacritty_terminal::term::cell::Flags;
use shift_pty::alacritty_terminal::vte::ansi::{self, NamedColor};
use shift_pty::{EventProxy, Session};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

/// Terminal cell metrics + content origin, all logical px.
#[derive(Debug, Clone, Copy)]
pub struct GridLayout {
    /// Left edge of the block border / raw content area (design: 8px).
    pub x: f32,
    /// Top of the first row.
    pub y: f32,
    /// Right edge of the content area.
    pub right: f32,
    pub cell_w: f32,
    pub line_h: f32,
    pub font_size: f32,
    pub cols: usize,
    pub rows: usize,
    pub ligatures: bool,
    pub cursor_style: CursorStyle,
    /// The owned editor draws the cursor; the grid's own cursor stays hidden
    /// (shell-prompt mode keeps the prompt rows visible without it).
    pub editor_owns_cursor: bool,
}

/// Block content indent: 2px border + 14px padding (design).
pub const BLOCK_INDENT: f32 = 16.0;
/// Vertical gap between consecutive blocks' left borders (design 01).
const BLOCK_GAP: f32 = 2.0;

/// Clickable header rect reported back to the app for collapse toggling.
pub struct HitRect {
    pub rect: [f32; 4],
    pub block: BlockId,
}

/// What occupies one visible row — cached per frame so mouse handlers can
/// map (x, y) → grid coordinates without recomputing layout (blocks mutate
/// on the reader thread between frame and click; abs indices are stable).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RowKind {
    Grid {
        abs: u64,
        content_x: f32,
    },
    Header(BlockId),
    /// Blank soft-`clear` padding row (no mouse targets).
    Pad,
}

#[derive(Debug, Clone, Copy)]
pub struct RowInfo {
    pub y: f32,
    pub kind: RowKind,
}

#[derive(Default)]
pub struct RenderOutput {
    /// Header rects (minus chevron): click selects the block.
    pub hits: Vec<HitRect>,
    /// Chevron rects on block headers: click collapses.
    pub chevron_hits: Vec<HitRect>,
    /// Copy-button rects on block headers.
    pub copy_hits: Vec<HitRect>,
    pub display_len: usize,
    pub any_running: bool,
    /// Per visible row, top-to-bottom (see `RowKind`).
    pub rows: Vec<RowInfo>,
    pub alt_screen: bool,
    /// Visible URL spans: (rect, url).
    pub url_hits: Vec<([f32; 4], String)>,
    /// Bookmark indicator dots: (rect, block).
    pub bookmark_hits: Vec<([f32; 4], BlockId)>,
    /// Block whose header scrolled above the viewport while its rows are
    /// still visible at the top (drives the sticky command header).
    pub sticky_block: Option<BlockId>,
}

/// Byte-scan for http(s) URLs in a line: (char col, char len, url).
pub fn urls_in(text: &str) -> Vec<(usize, usize, String)> {
    const STOP: &[char] = &[' ', '\t', '"', '\'', '<', '>', ')', ']', '}', '`'];
    let chars: Vec<char> = text.chars().collect();
    let lower: String = text.to_lowercase();
    let mut out = Vec::new();
    let mut search_from = 0usize; // char index
    let lower_chars: Vec<char> = lower.chars().collect();
    while search_from + 8 <= chars.len() {
        // find "http" at char level
        let rel = lower_chars[search_from..]
            .windows(4)
            .position(|w| w == ['h', 't', 't', 'p']);
        let Some(rel) = rel else { break };
        let start = search_from + rel;
        let rest: String = lower_chars[start..].iter().collect();
        let scheme_len = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            search_from = start + 4;
            continue;
        };
        let mut end = start + scheme_len;
        while end < chars.len() && !STOP.contains(&chars[end]) {
            end += 1;
        }
        // trim trailing punctuation
        while end > start + scheme_len && matches!(chars[end - 1], '.' | ',' | ';' | ':') {
            end -= 1;
        }
        if end > start + scheme_len {
            let url: String = chars[start..end].iter().collect();
            out.push((start, end - start, url));
        }
        search_from = end.max(start + 4);
    }
    out
}

/// The path-like token covering char column `col` of `text`, split from a
/// trailing `:line[:col]` (compiler/grep output). Surrounding quotes,
/// brackets and trailing punctuation are dropped.
pub fn path_token_at(text: &str, col: usize) -> Option<(String, Option<u32>)> {
    let chars: Vec<char> = text.chars().collect();
    if col >= chars.len() {
        return None;
    }
    let stop = |c: char| c.is_whitespace() || "\"'`<>()[]{}|,;".contains(c);
    if stop(chars[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && !stop(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < chars.len() && !stop(chars[end]) {
        end += 1;
    }
    let mut token: String = chars[start..end].iter().collect();
    while token.ends_with(['.', ':']) {
        token.pop();
    }
    // `src/main.rs:42:7` → (src/main.rs, 42)
    let mut line = None;
    let mut parts: Vec<&str> = token.split(':').collect();
    while parts.len() > 1 && parts.last().is_some_and(|p| p.parse::<u32>().is_ok()) {
        line = parts.pop().and_then(|p| p.parse().ok()).or(line);
    }
    let path = parts.join(":");
    let looks_like_path = path.contains('/') || path.contains('.');
    (!path.is_empty() && looks_like_path).then_some((path, line))
}

/// Sprites for the pane's image placements that touch a visible grid row.
/// A placement's top is found from any of its visible rows, so an image
/// that starts above the viewport still draws its lower part.
pub fn image_sprites(
    session: &Session,
    rows: &[RowInfo],
    grid: &GridLayout,
    alt_screen: bool,
) -> Vec<shift_renderer::Sprite> {
    let g = session.graphics.lock().unwrap();
    if g.placements.is_empty() {
        return Vec::new();
    }
    let visible: Vec<(u64, f32, f32)> = rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::Grid { abs, content_x } => Some((abs, r.y, content_x)),
            _ => None,
        })
        .collect();
    let (Some(first), Some(last)) = (visible.first(), visible.last()) else {
        return Vec::new();
    };
    let clip = [
        grid.x,
        first.1,
        grid.right - grid.x,
        last.1 + grid.line_h - first.1,
    ];
    let mut out = Vec::new();
    for p in &g.placements {
        if p.alt != alt_screen {
            continue;
        }
        let end = p.line + p.rows as u64;
        let Some(&(abs, y, content_x)) = visible.iter().find(|(a, _, _)| *a >= p.line && *a < end)
        else {
            continue;
        };
        let Some(img) = g.image(p.image) else {
            continue;
        };
        let top = y - (abs - p.line) as f32 * grid.line_h;
        out.push(shift_renderer::Sprite {
            key: img.key,
            rgba: img.rgba.clone(),
            size: [img.width, img.height],
            rect: [
                content_x + p.col as f32 * grid.cell_w,
                top,
                p.cols as f32 * grid.cell_w,
                p.rows as f32 * grid.line_h,
            ],
            src: p.src.map(|v| v as f32),
            clip,
            under_text: p.z < 0,
        });
    }
    out
}

/// Every path-like token of `text`: (start col, char len, path). The span
/// covers the whole token (including a `:line:col` suffix).
pub fn path_tokens(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let stop = |c: char| c.is_whitespace() || "\"'`<>()[]{}|,;".contains(c);
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if stop(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !stop(chars[i]) {
            i += 1;
        }
        let token: String = chars[start..i].iter().collect();
        if token.contains("://") {
            continue; // URLs are hinted from the URL detector
        }
        if let Some((path, _)) = path_token_at(text, start) {
            out.push((start, i - start, path));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayLine {
    Grid(u64),
    Header(usize), // index into blocks snapshot
    /// Blank viewport padding inserted by a soft `clear`.
    Pad,
}

/// Build the pane's display-line list: every kept grid line, with block
/// prompt echoes replaced by a synthesized header row and collapsed output
/// skipped. Pure — unit-tested against a blocks snapshot.
#[cfg(test)]
pub fn build_display_list(
    blocks: &[Block],
    scrolled: u64,
    screen_lines: usize,
    history: u64,
    alt_screen: bool,
    hide_prompt: bool,
    clear_pad: Option<(u64, usize)>,
) -> Vec<DisplayLine> {
    build_display_list_filtered(
        blocks,
        scrolled,
        screen_lines,
        history,
        alt_screen,
        hide_prompt,
        clear_pad,
        &std::collections::HashSet::new(),
    )
}

/// Output lines of the filtered block that do not contain the filter query
/// (case-insensitive) — skipped by the display list (Warp block filter).
pub fn filter_hidden(
    session: &Session,
    filter: Option<&(BlockId, String)>,
) -> std::collections::HashSet<u64> {
    let mut hidden = std::collections::HashSet::new();
    let Some((id, query)) = filter else {
        return hidden;
    };
    if query.is_empty() {
        return hidden;
    }
    let Some((start, end)) = session
        .blocks
        .lock()
        .unwrap()
        .iter()
        .find(|b| b.id == *id)
        .and_then(|b| b.output_range())
    else {
        return hidden;
    };
    let query = query.to_lowercase();
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    for abs in start..end {
        let keep = search_text(&term, scrolled, abs)
            .is_some_and(|t| t.text.to_lowercase().contains(&query));
        if !keep {
            hidden.insert(abs);
        }
    }
    hidden
}

/// The display-line list (`build_display_list`) minus `hidden` grid lines
/// (block filter).
#[allow(clippy::too_many_arguments)]
pub fn build_display_list_filtered(
    blocks: &[Block],
    scrolled: u64,
    screen_lines: usize,
    history: u64,
    alt_screen: bool,
    hide_prompt: bool,
    clear_pad: Option<(u64, usize)>,
    hidden: &std::collections::HashSet<u64>,
) -> Vec<DisplayLine> {
    let mut display: Vec<DisplayLine> = Vec::new();
    if alt_screen {
        for row in 0..screen_lines {
            display.push(DisplayLine::Grid(scrolled + row as u64));
        }
        return display;
    }
    let oldest = scrolled.saturating_sub(history);
    let mut newest = scrolled + screen_lines as u64;
    // owned editor replaces the live shell prompt: hide its rows and
    // everything after (blank screen tail)
    if hide_prompt
        && let Some(last) = blocks.last()
        && matches!(last.state, BlockState::Prompt)
    {
        newest = newest.min(last.prompt_start);
    }
    // soft `clear`: pad rows push pre-clear content above the fold
    let mut pad_pending = clear_pad.filter(|(line, _)| *line >= oldest);
    let mut abs = oldest;
    while abs < newest {
        if let Some((line, n)) = pad_pending
            && abs == line
        {
            display.extend(std::iter::repeat_n(DisplayLine::Pad, n));
            pad_pending = None;
        }
        // does a block header replace this row?
        let mut replaced = false;
        for (bi, block) in blocks.iter().enumerate() {
            if let Some((s, o)) = block.suppressed_range()
                && abs == s
            {
                display.push(DisplayLine::Header(bi));
                let next = if block.collapsed {
                    block.end.unwrap_or(o).max(o)
                } else {
                    o
                };
                // always advance: anchors squeezed onto one row (output
                // start at or above the prompt) would otherwise loop forever
                abs = next.max(abs + 1);
                replaced = true;
                break;
            }
        }
        if !replaced {
            if !hidden.contains(&abs) {
                display.push(DisplayLine::Grid(abs));
            }
            abs += 1;
        }
    }
    // clear happened at (or past) the hidden live prompt: pads go at the end
    if let Some((line, n)) = pad_pending
        && line >= newest
    {
        display.extend(std::iter::repeat_n(DisplayLine::Pad, n));
    }
    display
}

fn cube_component(v: u8) -> f32 {
    if v == 0 {
        0.0
    } else {
        (55 + 40 * v as u16) as f32 / 255.0
    }
}

pub fn indexed_color(theme: &Theme, idx: u8) -> Color {
    match idx {
        0..=15 => theme.ansi[idx as usize],
        16..=231 => {
            let i = idx - 16;
            Color::rgba(
                cube_component(i / 36),
                cube_component((i % 36) / 6),
                cube_component(i % 6),
                1.0,
            )
        }
        232..=255 => {
            let g = (8 + 10 * (idx as u16 - 232)) as f32 / 255.0;
            Color::rgba(g, g, g, 1.0)
        }
    }
}

fn named_color(theme: &Theme, name: NamedColor) -> Color {
    use NamedColor::*;
    match name {
        Black | DimBlack => theme.ansi[0],
        Red | DimRed => theme.ansi[1],
        Green | DimGreen => theme.ansi[2],
        Yellow | DimYellow => theme.ansi[3],
        Blue | DimBlue => theme.ansi[4],
        Magenta | DimMagenta => theme.ansi[5],
        Cyan | DimCyan => theme.ansi[6],
        White | DimWhite => theme.ansi[7],
        BrightBlack => theme.ansi[8],
        BrightRed => theme.ansi[9],
        BrightGreen => theme.ansi[10],
        BrightYellow => theme.ansi[11],
        BrightBlue => theme.ansi[12],
        BrightMagenta => theme.ansi[13],
        BrightCyan => theme.ansi[14],
        BrightWhite => theme.ansi[15],
        Foreground | BrightForeground => theme.fg,
        Background => theme.bg,
        Cursor => theme.cursor,
        DimForeground => theme.text_dim,
    }
}

pub fn resolve_color(theme: &Theme, color: ansi::Color) -> Color {
    match color {
        ansi::Color::Named(n) => named_color(theme, n),
        ansi::Color::Indexed(i) => indexed_color(theme, i),
        ansi::Color::Spec(rgb) => Color::rgba(
            rgb.r as f32 / 255.0,
            rgb.g as f32 / 255.0,
            rgb.b as f32 / 255.0,
            1.0,
        ),
    }
}

pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 10_000 {
        format!("{:.2}s", ms as f64 / 1000.0)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

fn mono_attrs(ligatures: bool) -> Attrs<'static> {
    let attrs = Attrs::new().family(Family::Name(mono_family()));
    if ligatures {
        attrs
    } else {
        use shift_renderer::glyphon::cosmic_text::{FeatureTag, FontFeatures};
        let mut f = FontFeatures::new();
        f.set(FeatureTag::STANDARD_LIGATURES, 0);
        f.set(FeatureTag::CONTEXTUAL_ALTERNATES, 0);
        attrs.font_features(f)
    }
}

fn make_text(
    engine: &mut TextEngine,
    text: &str,
    font_size: f32,
    line_h: f32,
    family: &'static str,
    weight: Weight,
) -> std::sync::Arc<Buffer> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    0u8.hash(&mut h); // cache-key namespace: plain runs
    text.hash(&mut h);
    font_size.to_bits().hash(&mut h);
    line_h.to_bits().hash(&mut h);
    family.hash(&mut h);
    weight.0.hash(&mut h);
    engine.shaped(h.finish(), |fs| {
        let mut buf = Buffer::new(fs, Metrics::new(font_size, line_h));
        buf.set_size(fs, None, None);
        buf.set_text(
            fs,
            text,
            &Attrs::new().family(Family::Name(family)).weight(weight),
            Shaping::Advanced,
            None,
        );
        buf
    })
}

/// Active search: the compiled matcher + the current match anchor.
pub struct SearchCtx {
    pub matcher: Matcher,
    pub current: Option<(u64, usize)>,
}

/// Find-bar matcher: plain text (optionally case-sensitive) or a regex.
#[derive(Clone)]
pub enum Matcher {
    Plain { needle: Vec<char>, case: bool },
    Regex(regex::Regex),
}

impl Matcher {
    /// `None` for an empty query or an invalid regex (the error string).
    pub fn new(query: &str, case: bool, is_regex: bool) -> Result<Option<Matcher>, String> {
        if query.is_empty() {
            return Ok(None);
        }
        if is_regex {
            return regex::RegexBuilder::new(query)
                .case_insensitive(!case)
                .build()
                .map(|r| Some(Matcher::Regex(r)))
                .map_err(|e| e.to_string());
        }
        let needle = query
            .chars()
            .map(|c| if case { c } else { fold(c) })
            .collect();
        Ok(Some(Matcher::Plain { needle, case }))
    }

    /// Matches in a grid line as (grid column, column span). A match that
    /// covers any char of a cell covers the whole cell: a wide glyph spans
    /// two columns, and a combining mark belongs to its base cell.
    pub fn find_line(&self, line: &SearchLine) -> Vec<(usize, usize)> {
        self.find_cols(&line.text)
            .into_iter()
            .map(|(s, n)| {
                let start = line.cols[s];
                (start, line.ends[s + n - 1] - start)
            })
            .collect()
    }

    /// [`Matcher::find_line`] over `text` read as cells: one char per
    /// column, NUL standing for the right half of a wide glyph.
    #[cfg(test)]
    pub fn find(&self, text: &str) -> Vec<(usize, usize)> {
        self.find_line(&SearchLine::from_cells(text))
    }

    fn find_cols(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            Matcher::Plain { needle, case } => {
                let hay: Vec<char> = text
                    .chars()
                    .map(|c| if *case { c } else { fold(c) })
                    .collect();
                let mut out = Vec::new();
                if needle.is_empty() || hay.len() < needle.len() {
                    return out;
                }
                let mut i = 0;
                while i + needle.len() <= hay.len() {
                    if hay[i..i + needle.len()] == needle[..] {
                        out.push((i, needle.len()));
                        i += needle.len();
                    } else {
                        i += 1;
                    }
                }
                out
            }
            Matcher::Regex(re) => {
                // byte offsets → char columns (cells map 1:1 to chars)
                let col = |b: usize| text[..b].chars().count();
                re.find_iter(text)
                    .filter(|m| !m.is_empty())
                    .map(|m| {
                        let start = col(m.start());
                        (start, col(m.end()) - start)
                    })
                    .collect()
            }
        }
    }
}

/// One-char case fold (keeps columns aligned, unlike `to_lowercase`).
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Line text of one absolute grid line (trailing spaces trimmed). One char
/// per cell, so a char index is a grid column.
fn line_text(
    term: &shift_pty::alacritty_terminal::term::Term<EventProxy>,
    scrolled: u64,
    abs: u64,
) -> Option<String> {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return None;
    }
    let row = &g[Line(rel as i32)];
    let mut text: String = (0..g.columns()).map(|c| row[Column(c)].c).collect();
    while text.ends_with(' ') {
        text.pop();
    }
    Some(text)
}

/// Searchable text of one grid line: each cell's char followed by its
/// combining marks, wide-glyph spacers left out (so "日本" and decomposed
/// "café" are found). `cols[i]` is the grid column of char `i`, `ends[i]`
/// the column after its cell.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SearchLine {
    pub text: String,
    cols: Vec<usize>,
    ends: Vec<usize>,
}

impl SearchLine {
    fn push(&mut self, c: char, col: usize, end: usize) {
        self.text.push(c);
        self.cols.push(col);
        self.ends.push(end);
    }

    /// Trailing blank cells dropped.
    fn trim_end(&mut self) {
        while self.text.ends_with(' ') {
            self.text.pop();
            self.cols.pop();
            self.ends.pop();
        }
    }

    /// `text` as cells: one char per column; NUL is the right half of the
    /// wide glyph before it (no char of its own).
    #[cfg(test)]
    pub fn from_cells(text: &str) -> SearchLine {
        let cells: Vec<char> = text.chars().collect();
        let mut line = SearchLine::default();
        for (col, &c) in cells.iter().enumerate() {
            if c != '\0' {
                let end = col + 1 + cells[col + 1..].iter().take_while(|&&n| n == '\0').count();
                line.push(c, col, end);
            }
        }
        line
    }
}

fn search_text<T>(
    term: &shift_pty::alacritty_terminal::term::Term<T>,
    scrolled: u64,
    abs: u64,
) -> Option<SearchLine> {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return None;
    }
    let row = &g[Line(rel as i32)];
    let mut line = SearchLine::default();
    for col in 0..g.columns() {
        let cell = &row[Column(col)];
        if cell.flags.intersects(WIDE_SPACERS) {
            continue;
        }
        let end = col
            + if cell.flags.contains(Flags::WIDE_CHAR) {
                2
            } else {
                1
            };
        // hidden (SGR 8) text stays unsearchable, as it is undrawn
        if cell.flags.contains(Flags::HIDDEN) {
            line.push(' ', col, end);
            continue;
        }
        line.push(cell.c, col, end);
        for &zw in cell.zerowidth().unwrap_or_default() {
            line.push(zw, col, end);
        }
    }
    line.trim_end();
    Some(line)
}

const WIDE_SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

/// Text of one grid line for copying: no wide-glyph spacers, combining marks
/// kept. The flag is true when the row soft-wraps into the next one (its
/// trailing spaces are content then and are kept).
fn copy_line<T>(
    term: &shift_pty::alacritty_terminal::term::Term<T>,
    scrolled: u64,
    abs: u64,
) -> Option<(String, bool)> {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return None;
    }
    let row = &g[Line(rel as i32)];
    let cols = g.columns();
    let wrapped = cols > 0 && row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
    let mut text = String::new();
    for c in 0..cols {
        let cell = &row[Column(c)];
        if cell.flags.intersects(WIDE_SPACERS) {
            continue;
        }
        text.push(cell.c);
        if let Some(zw) = cell.zerowidth() {
            text.extend(zw);
        }
    }
    if !wrapped {
        let trimmed = text.trim_end_matches(' ').len();
        text.truncate(trimmed);
    }
    Some((text, wrapped))
}

/// Text of absolute line `abs` of `session` (trailing spaces trimmed).
pub fn session_line_text(session: &Session, abs: u64) -> Option<String> {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    line_text(&term, scrolled, abs)
}

/// Readable text of absolute line `abs` (combining marks kept, wide-glyph
/// spacers and trailing spaces dropped) — for screen readers, unlike the
/// column-indexed [`session_line_text`].
pub fn session_read_text(session: &Session, abs: u64) -> Option<String> {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    copy_line(&term, scrolled, abs).map(|(text, _)| text.trim_end_matches(' ').to_string())
}

/// OSC 8 hyperlink runs on one absolute line: (col, len, uri).
fn hyperlinks_in(
    term: &shift_pty::alacritty_terminal::term::Term<EventProxy>,
    scrolled: u64,
    abs: u64,
) -> Vec<(usize, usize, String)> {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return Vec::new();
    }
    let row = &g[Line(rel as i32)];
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    for col in 0..g.columns() {
        let Some(link) = row[Column(col)].hyperlink() else {
            continue;
        };
        let uri = link.uri().to_string();
        match out.last_mut() {
            Some((start, len, u)) if *u == uri && *start + *len == col => *len += 1,
            _ => out.push((col, 1, uri)),
        }
    }
    out
}

/// Search result: the newest `cap` anchors plus the count of every match.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Found {
    /// (abs line, char col) anchors, oldest first: the newest `cap` matches.
    pub anchors: Vec<(u64, usize)>,
    /// Every match in the scanned lines (>= `anchors.len()`).
    pub total: usize,
}

/// Match anchors over `lines` (given oldest -> newest): scans newest first,
/// keeps the newest `cap` anchors (returned oldest first) and counts every
/// match, so older matches past the cap are counted but not stored.
pub fn newest_matches<I>(lines: I, matcher: &Matcher, cap: usize) -> Found
where
    I: DoubleEndedIterator<Item = (u64, SearchLine)>,
{
    let mut found = Found::default();
    for (abs, line) in lines.rev() {
        let hits = matcher.find_line(&line);
        found.total += hits.len();
        for (col, _) in hits.into_iter().rev() {
            if found.anchors.len() >= cap {
                break;
            }
            found.anchors.push((abs, col));
        }
    }
    found.anchors.reverse();
    found
}

/// Match anchors across kept history + screen (see [`newest_matches`]).
pub fn collect_matches(session: &Session, matcher: &Matcher, cap: usize) -> Found {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let g = term.grid();
    let oldest = scrolled.saturating_sub(g.history_size() as u64);
    let newest = scrolled + g.screen_lines() as u64;
    let lines = (oldest..newest).filter_map(|abs| Some((abs, search_text(&term, scrolled, abs)?)));
    newest_matches(lines, matcher, cap)
}

/// Text of the on-screen grid rows (trailing blank rows dropped) — what a
/// user sees at the bottom of the pane. Script-driver assertions only.
pub fn visible_text(session: &Session) -> String {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let rows = term.grid().screen_lines() as u64;
    let mut lines: Vec<String> = (scrolled..scrolled + rows)
        .filter_map(|abs| line_text(&term, scrolled, abs))
        .collect();
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Output text of a block (for the copy button / copy action).
pub fn block_output_text(session: &Session, block_id: BlockId) -> Option<String> {
    let (start, end, unterminated) = {
        let blocks = session.blocks.lock().unwrap();
        let block = blocks.iter().find(|b| b.id == block_id)?;
        let (start, end) = block.output_range()?;
        (start, end, block.unterminated)
    };
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let mut out = String::new();
    for abs in start..end {
        if let Some((text, wrapped)) = copy_line(&term, scrolled, abs) {
            out.push_str(&text);
            // a soft-wrapped row continues on the next one: no line break
            if !wrapped {
                out.push('\n');
            }
        }
    }
    // the last row's line break was the terminal's, not the output's
    if unterminated && out.ends_with('\n') {
        out.pop();
    }
    Some(out)
}

/// Render the pane. Owns the viewport: `view_offset` counts display lines
/// scrolled up from the bottom.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    session: &Session,
    grid: &GridLayout,
    view_offset: &mut usize,
    hide_prompt: bool,
    search: Option<&SearchCtx>,
    selected: Option<BlockId>,
    filter: Option<&(BlockId, String)>,
) -> RenderOutput {
    use std::sync::atomic::Ordering;
    session.dirty.store(false, Ordering::Release);

    let hidden = filter_hidden(session, filter);
    let (blocks, clear_pad) = {
        let store = session.blocks.lock().unwrap();
        (store.snapshot(), store.clear_pad())
    };
    // header note for the filtered block: `filter "q" · shown/total`
    let filter_note = filter.and_then(|(id, q)| {
        let (s, e) = blocks.iter().find(|b| b.id == *id)?.output_range()?;
        let total = (e - s) as usize;
        Some((
            *id,
            format!("filter \"{q}\" · {}/{total} lines", total - hidden.len()),
        ))
    });
    let scrolled = session.scrolled.load(Ordering::Acquire);
    let term = session.term.lock();

    let mut out = RenderOutput {
        any_running: blocks
            .iter()
            .any(|b| matches!(b.state, BlockState::Running { .. })),
        ..Default::default()
    };

    let alt_screen = term.mode().contains(TermMode::ALT_SCREEN);
    out.alt_screen = alt_screen;
    let g = term.grid();
    let screen_lines = g.screen_lines();
    let history = g.history_size() as u64;

    let display = build_display_list_filtered(
        &blocks,
        scrolled,
        screen_lines,
        history,
        alt_screen,
        hide_prompt,
        clear_pad,
        &hidden,
    );
    if alt_screen {
        *view_offset = 0;
    }
    out.display_len = display.len();

    // ── viewport slice ──
    let rows = grid.rows.min(display.len());
    *view_offset = (*view_offset).min(display.len() - rows);
    let start = display.len() - rows - *view_offset;
    let visible = &display[start..start + rows];

    let cursor = term.renderable_content().cursor;
    let show_cursor = term.mode().contains(TermMode::SHOW_CURSOR) && !grid.editor_owns_cursor;
    let cursor_abs = scrolled + cursor.point.line.0.max(0) as u64;
    let sel_range = term.selection.as_ref().and_then(|s| s.to_range(&term));
    // an open (running) block ends at its last used row, the cursor row or
    // the last non-blank line, not at the bottom of the blank screen tail
    let live_end = {
        let newest = scrolled + screen_lines as u64;
        (cursor_abs + 1..newest)
            .rev()
            .find(|&abs| line_text(&term, scrolled, abs).is_some_and(|t| !t.trim().is_empty()))
            .map_or(cursor_abs + 1, |abs| abs + 1)
    };

    // keyboard-selected block: abs row range for the highlight treatment
    let selected_range = selected.and_then(|id| {
        blocks
            .iter()
            .find(|b| b.id == id)
            .map(|b| (b.prompt_start, b.end.unwrap_or(live_end)))
    });
    let row_selected = |abs: u64| selected_range.is_some_and(|(s, e)| abs >= s && abs < e);

    // only the keyboard-selected block gets a wash; exit state lives on the
    // left border and badge alone (design 01: failed blocks are not tinted)
    let row_bg =
        |abs: u64| -> Option<Color> { row_selected(abs).then(|| theme.accent.with_alpha(0.08)) };

    // block membership for border drawing: abs line → block state color
    let border_color = |abs: u64| -> Option<Color> {
        for b in &blocks {
            let start = b.prompt_start;
            let end = b.end.unwrap_or(live_end);
            if abs >= start && abs < end {
                if matches!(b.state, BlockState::Prompt) {
                    return None; // raw prompt rows, no border yet
                }
                return Some(match b.state {
                    BlockState::Running { .. } => theme.accent,
                    BlockState::Done { exit: Some(0), .. } => theme.success,
                    BlockState::Done { exit: Some(_), .. } => theme.error,
                    BlockState::Done { exit: None, .. } => theme.text_muted,
                    BlockState::Prompt => unreachable!(),
                });
            }
        }
        None
    };

    // which block a display line belongs to: the last row before a change
    // of block shortens its border so blocks sit 2px apart (design 01:
    // `margin-bottom: 2px` on every block)
    let owner = |dl: &DisplayLine| -> Option<BlockId> {
        match dl {
            DisplayLine::Header(bi) => Some(blocks[*bi].id),
            DisplayLine::Grid(abs) => blocks
                .iter()
                .find(|b| *abs >= b.prompt_start && *abs < b.end.unwrap_or(u64::MAX))
                .map(|b| b.id),
            DisplayLine::Pad => None,
        }
    };
    let border_h = |r: usize| -> f32 {
        let this = owner(&visible[r]);
        let next = visible.get(r + 1).and_then(owner);
        if this.is_some() && next != this && visible.get(r + 1).is_some() {
            grid.line_h - BLOCK_GAP
        } else {
            grid.line_h + 0.5
        }
    };
    for (r, dl) in visible.iter().enumerate() {
        let y = grid.y + r as f32 * grid.line_h;
        match dl {
            DisplayLine::Grid(abs) => {
                let is_sel = !alt_screen && row_selected(*abs);
                if !alt_screen && let Some(bg) = row_bg(*abs) {
                    // block wash under content (quads draw in push order)
                    layer.rect(
                        [grid.x, y],
                        [grid.right - grid.x, grid.line_h + 0.5],
                        bg,
                        0.0,
                    );
                }
                let content_x = if alt_screen {
                    grid.x
                } else if let Some(color) = border_color(*abs) {
                    if is_sel {
                        layer.rect([grid.x, y], [3.0, border_h(r)], theme.accent, 0.0);
                    } else {
                        layer.rect([grid.x, y], [2.0, border_h(r)], color, 0.0);
                    }
                    grid.x + BLOCK_INDENT
                } else {
                    grid.x
                };
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Grid {
                        abs: *abs,
                        content_x,
                    },
                });
                // clickable URLs: subtle underline + hit rect for cmd+click
                if !alt_screen && let Some(text) = line_text(&term, scrolled, *abs) {
                    for (col, len, url) in urls_in(&text) {
                        let ux = content_x + col as f32 * grid.cell_w;
                        let uw = len as f32 * grid.cell_w;
                        layer.rect(
                            [ux, y + grid.line_h - 3.0],
                            [uw, 1.0],
                            theme.text_muted.with_alpha(0.6),
                            0.0,
                        );
                        out.url_hits.push(([ux, y, uw, grid.line_h], url));
                    }
                }
                // OSC 8 hyperlinks (ls --hyperlink, gcc, delta): underlined by
                // render_grid_line, cmd+click opens the target
                for (col, len, uri) in hyperlinks_in(&term, scrolled, *abs) {
                    let ux = content_x + col as f32 * grid.cell_w;
                    out.url_hits
                        .push(([ux, y, len as f32 * grid.cell_w, grid.line_h], uri));
                }
                render_grid_line(
                    layer,
                    engine,
                    theme,
                    &term,
                    grid,
                    *abs,
                    scrolled,
                    content_x,
                    y,
                    show_cursor && *abs == cursor_abs,
                    cursor.point.column.0,
                    search,
                    sel_range,
                );
            }
            DisplayLine::Pad => {
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Pad,
                });
            }
            DisplayLine::Header(bi) => {
                let block = &blocks[*bi];
                if selected == Some(block.id) {
                    let bg = theme.accent.with_alpha(0.08);
                    layer.rect(
                        [grid.x, y],
                        [grid.right - grid.x, grid.line_h + 0.5],
                        bg,
                        0.0,
                    );
                }
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Header(block.id),
                });
                let note = filter_note
                    .as_ref()
                    .filter(|(id, _)| *id == block.id)
                    .map(|(_, n)| n.as_str());
                let bh = border_h(r);
                render_header(layer, engine, theme, grid, block, y, bh, note, &mut out);
            }
        }
    }

    // sticky command header: first visible row belongs to a block whose
    // header row is above the viewport
    if !alt_screen && let Some(DisplayLine::Grid(top_abs)) = visible.first() {
        out.sticky_block = blocks
            .iter()
            .find(|b| {
                b.suppressed_range()
                    .is_some_and(|(s, _)| s < *top_abs && *top_abs < b.end.unwrap_or(u64::MAX))
            })
            .map(|b| b.id);
    }

    // bookmark indicator dots on the right edge (position ∝ place in history)
    if !alt_screen {
        let viewport_h = rows as f32 * grid.line_h;
        for (idx, dl) in display.iter().enumerate() {
            if let DisplayLine::Header(bi) = dl {
                let b = &blocks[*bi];
                if b.bookmarked {
                    let frac = idx as f32 / display.len().max(1) as f32;
                    let dy = grid.y + frac * (viewport_h - 8.0);
                    let dx = grid.right - 5.0;
                    layer.circle([dx, dy + 4.0], 7.0, theme.warning);
                    out.bookmark_hits
                        .push(([dx - 7.0, dy - 3.0, 14.0, 14.0], b.id));
                }
            }
        }
    }

    out
}

/// Draw the pinned header for `block` at the viewport top (layer 2 — an
/// opaque bg on layer 1 could not cover layer-1 row text). Returns the
/// clickable rect (jump to block).
pub fn render_sticky_header(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    grid: &GridLayout,
    session: &Session,
    block_id: BlockId,
) -> Option<[f32; 4]> {
    let block = session
        .blocks
        .lock()
        .unwrap()
        .iter()
        .find(|b| b.id == block_id)
        .cloned()?;
    let y = grid.y;
    let rect = [grid.x, y, grid.right - grid.x, grid.line_h + 6.0];
    // opaque backdrop + hairline
    layer.rect(
        [rect[0], rect[1] - 4.0],
        [rect[2], rect[3] + 4.0],
        theme.bg,
        0.0,
    );
    layer.hline(
        rect[0],
        rect[1] + rect[3],
        rect[2],
        1.0,
        Color::rgba(1.0, 1.0, 1.0, 0.07),
    );
    let mut sink = RenderOutput::default();
    render_header(
        layer,
        engine,
        theme,
        grid,
        &block,
        y,
        grid.line_h + 0.5,
        None,
        &mut sink,
    );
    Some(rect)
}

#[allow(clippy::too_many_arguments)]
fn render_grid_line(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    term: &shift_pty::alacritty_terminal::term::Term<EventProxy>,
    grid: &GridLayout,
    abs: u64,
    scrolled: u64,
    x: f32,
    y: f32,
    cursor_here: bool,
    cursor_col: usize,
    search: Option<&SearchCtx>,
    sel_range: Option<SelectionRange>,
) {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return;
    }
    let line = Line(rel as i32);
    let row = &g[line];

    // search highlighting: per-column match state on this line
    let mut match_cols: Vec<(usize, bool)> = Vec::new(); // (col, is_current)
    if let Some(ctx) = search
        && let Some(text) = search_text(term, scrolled, abs)
    {
        for (col, len) in ctx.matcher.find_line(&text) {
            let is_current = ctx.current == Some((abs, col));
            for c in col..col + len {
                match_cols.push((c, is_current));
            }
        }
    }

    struct Span {
        text: String,
        fg: Color,
        weight: Weight,
        italic: bool,
    }
    let mut spans: Vec<Span> = Vec::new();
    let cols = g.columns().min(grid.cols + 8);
    // cell backgrounds (merged runs, block cursor) draw before decorations
    let mut backs: Vec<([f32; 2], [f32; 2], Color, f32)> = Vec::new();
    let mut decos: Vec<([f32; 2], [f32; 2], Color)> = Vec::new();

    for col in 0..cols {
        let cell = &row[Column(col)];
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }

        let mut fg = resolve_color(theme, cell.fg);
        let mut bg = resolve_color(theme, cell.bg);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = fg.with_alpha(0.66);
        }

        // selection highlight (search match wins below)
        if let Some(range) = &sel_range
            && range.contains(Point::new(line, Column(col)))
        {
            bg = theme.selection;
        }

        // search match: spec colors, dark text on the highlight
        if let Some((_, is_current)) = match_cols.iter().find(|(c, _)| *c == col) {
            bg = if *is_current {
                theme.search_active
            } else {
                theme.search_match
            };
            fg = Color::hex("#1e1e1e").unwrap();
        }

        let is_cursor = cursor_here && col == cursor_col;
        let cell_px = if flags.contains(Flags::WIDE_CHAR) {
            grid.cell_w * 2.0
        } else {
            grid.cell_w
        };
        let cx = x + col as f32 * grid.cell_w;

        let block_cursor = is_cursor && grid.cursor_style == CursorStyle::Block;
        if block_cursor {
            backs.push(([cx, y], [cell_px, grid.line_h], theme.cursor, 1.5));
        } else if bg != theme.bg {
            // one quad per run of equal backgrounds: per-cell quads left
            // antialiased seams between cells (tmux status bars)
            match backs.last_mut() {
                Some((pos, size, c, r))
                    if *r == 0.0 && *c == bg && (pos[0] + size[0] - cx).abs() < 0.01 =>
                {
                    size[0] += cell_px;
                }
                _ => backs.push(([cx, y], [cell_px, grid.line_h], bg, 0.0)),
            }
        }
        if is_cursor {
            match grid.cursor_style {
                CursorStyle::Block => {}
                CursorStyle::Bar => decos.push(([cx - 1.0, y], [2.0, grid.line_h], theme.cursor)),
                CursorStyle::Underline => {
                    decos.push(([cx, y + grid.line_h - 2.0], [cell_px, 2.0], theme.cursor))
                }
            }
        }

        let fg = if block_cursor { theme.bg } else { fg };
        let weight = if flags.contains(Flags::BOLD) {
            Weight::BOLD
        } else {
            Weight::NORMAL
        };
        let italic = flags.contains(Flags::ITALIC);
        let ch = if flags.contains(Flags::HIDDEN) {
            ' '
        } else {
            cell.c
        };

        // SGR decorations: underline variants (colored by SGR 58 when set)
        // and strikethrough, drawn as quads under the glyph layer
        if !flags.contains(Flags::HIDDEN) {
            let deco = cell
                .underline_color()
                .map(|c| resolve_color(theme, c))
                .unwrap_or(fg);
            let base = y + grid.font_size + (grid.line_h - grid.font_size) / 2.0 + 1.5;
            if flags.contains(Flags::DOUBLE_UNDERLINE) {
                decos.push(([cx, base - 1.0], [cell_px, 1.0], deco));
                decos.push(([cx, base + 1.5], [cell_px, 1.0], deco));
            } else if flags.contains(Flags::UNDERCURL) {
                // two half-cell segments offset vertically read as a wave
                let half = cell_px / 2.0;
                decos.push(([cx, base], [half, 1.2], deco));
                decos.push(([cx + half, base + 1.4], [half, 1.2], deco));
            } else if flags.intersects(Flags::DOTTED_UNDERLINE | Flags::DASHED_UNDERLINE) {
                let seg = if flags.contains(Flags::DOTTED_UNDERLINE) {
                    1.5
                } else {
                    cell_px * 0.4
                };
                let mut sx = cx;
                while sx < cx + cell_px {
                    decos.push(([sx, base], [seg.min(cx + cell_px - sx), 1.0], deco));
                    sx += seg * 2.0;
                }
            } else if flags.contains(Flags::UNDERLINE) || cell.hyperlink().is_some() {
                decos.push(([cx, base], [cell_px, 1.0], deco));
            }
            if flags.contains(Flags::STRIKEOUT) {
                decos.push(([cx, y + grid.line_h / 2.0], [cell_px, 1.0], fg));
            }
        }

        if !spans
            .last()
            .is_some_and(|l| l.fg == fg && l.weight == weight && l.italic == italic)
        {
            spans.push(Span {
                text: String::new(),
                fg,
                weight,
                italic,
            });
        }
        let Some(span) = spans.last_mut() else {
            continue;
        };
        span.text.push(ch);
        // combining marks shape onto their base char (zero advance)
        if !flags.contains(Flags::HIDDEN)
            && let Some(zw) = cell.zerowidth()
        {
            span.text.extend(zw);
        }
    }
    for (pos, size, color, radius) in backs {
        layer.rect(pos, size, color, radius);
    }
    for (pos, size, color) in decos {
        layer.rect(pos, size, color, 0.0);
    }

    if spans.iter().all(|s| s.text.trim().is_empty()) {
        return;
    }
    // trailing blank cells draw nothing (backgrounds are quads), but shaping
    // them cost ~40 KB per cached row: a 500k-line flood peaked near 3 GB
    while let Some(last) = spans.last_mut() {
        let kept = last.text.trim_end_matches(' ').len();
        if kept > 0 {
            last.text.truncate(kept);
            break;
        }
        spans.pop();
    }

    // shape cache: identical styled content re-uses last frame's buffer, so
    // scrolling only shapes lines entering the viewport
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    1u8.hash(&mut h); // cache-key namespace: grid lines
    grid.font_size.to_bits().hash(&mut h);
    grid.line_h.to_bits().hash(&mut h);
    grid.ligatures.hash(&mut h);
    for s in &spans {
        s.text.hash(&mut h);
        s.weight.0.hash(&mut h);
        s.italic.hash(&mut h);
        s.fg.r.to_bits().hash(&mut h);
        s.fg.g.to_bits().hash(&mut h);
        s.fg.b.to_bits().hash(&mut h);
        s.fg.a.to_bits().hash(&mut h);
    }
    let buf = engine.shaped(h.finish(), |fs| {
        let mut buf = Buffer::new(fs, Metrics::new(grid.font_size, grid.line_h));
        buf.set_size(fs, None, Some(grid.line_h));
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|s| {
                let attrs = mono_attrs(grid.ligatures)
                    .weight(s.weight)
                    .color(shift_renderer::text_color(s.fg));
                let attrs = if s.italic {
                    attrs.style(shift_renderer::glyphon::Style::Italic)
                } else {
                    attrs
                };
                (s.text.as_str(), attrs)
            })
            .collect();
        buf.set_rich_text(
            fs,
            rich,
            &mono_attrs(grid.ligatures),
            Shaping::Advanced,
            None,
        );
        buf
    });
    layer.text(buf, x, y, theme.fg);
}

/// Shared clock for running-block spinners (all spin in phase).
fn block_spin_epoch() -> std::time::Instant {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(std::time::Instant::now)
}

#[allow(clippy::too_many_arguments)]
fn render_header(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    grid: &GridLayout,
    block: &Block,
    y: f32,
    border_h: f32,
    filter_note: Option<&str>,
    out: &mut RenderOutput,
) {
    let (border, badge_fg, badge_text) = match block.state {
        BlockState::Running { .. } => (theme.accent, theme.warning, "running".to_string()),
        BlockState::Done { exit: Some(0), .. } => (theme.success, theme.success, "✓ 0".into()),
        BlockState::Done { exit: Some(n), .. } => (theme.error, theme.error, format!("✕ {n}")),
        BlockState::Done { exit: None, .. } => (theme.text_muted, theme.text_muted, "—".into()),
        BlockState::Prompt => return,
    };

    // left border across the header row
    layer.rect([grid.x, y], [2.0, border_h], border, 0.0);

    let content_x = grid.x + BLOCK_INDENT;

    // chevron
    let dir = if block.collapsed {
        super::Dir::Right
    } else {
        super::Dir::Down
    };
    super::chevron(
        layer,
        [content_x + 3.5, y + grid.line_h / 2.0],
        3.0,
        dir,
        1.3,
        theme.text_muted,
    );

    // left text must stop before the right-side badge/duration/copy group
    let duration_text = match block.state {
        // replayed transcript: never looks like a live command
        BlockState::Done { duration, .. } if block.restored => {
            format!("restored · {}", format_duration(duration))
        }
        BlockState::Done { duration, .. } => format_duration(duration),
        BlockState::Running { started } => format_duration(started.elapsed()),
        _ => String::new(),
    };
    let duration_w = if duration_text.is_empty() {
        0.0
    } else {
        duration_text.chars().count() as f32 * grid.cell_w * (12.0 / grid.font_size) + 11.0
    };
    let running = matches!(block.state, BlockState::Running { .. });
    // running: a 9px spinner + 6px gap lead the label (design 02)
    let spin_w = if running { 15.0 } else { 0.0 };
    let badge_w = badge_text.chars().count() as f32 * 6.8 + 16.0 + spin_w;
    let left_limit = grid.right - 13.0 - 6.0 - 10.0 - duration_w - badge_w - 12.0;

    // command (bright), one line, ellipsized to the space it has
    let cmd_x = content_x + 10.0 + 11.0;
    let cmd = if block.command.is_empty() {
        "—".to_string()
    } else {
        block.command.replace('\n', " ⏎ ")
    };
    let fit = ((left_limit - cmd_x) / grid.cell_w).floor().max(1.0) as usize;
    let cmd = if cmd.chars().count() > fit {
        let mut s: String = cmd.chars().take(fit.saturating_sub(1)).collect();
        s.push('…');
        s
    } else {
        cmd
    };
    let cmd_buf = make_text(
        engine,
        &cmd,
        grid.font_size,
        grid.line_h,
        mono_family(),
        Weight::NORMAL,
    );
    layer.text(cmd_buf, cmd_x, y, theme.text_bright);

    // collapsed note, else Warp-style prompt context: ~/path git:(branch)
    let after_cmd_x = cmd_x + cmd.chars().count() as f32 * grid.cell_w + 11.0;
    if let Some(note) = filter_note {
        let tw = engine.measure(note, 11.5, FONT_SANS, Weight::NORMAL);
        layer.rect(
            [after_cmd_x - 2.0, y + (grid.line_h - 17.0) / 2.0],
            [tw + 14.0, 17.0],
            theme.warning.with_alpha(0.13),
            5.0,
        );
        let nbuf = make_text(engine, note, 11.5, grid.line_h, FONT_SANS, Weight::NORMAL);
        layer.text(nbuf, after_cmd_x + 5.0, y, theme.warning);
    } else if block.collapsed
        && let Some((s, e)) = block.output_range()
    {
        // design 01: italic mono, muted
        let n = e - s;
        let note = format!("{n} line{} hidden", if n == 1 { "" } else { "s" });
        let mut nbuf = shift_renderer::glyphon::Buffer::new(
            &mut engine.font_system,
            shift_renderer::glyphon::Metrics::new(11.5, grid.line_h),
        );
        nbuf.set_text(
            &mut engine.font_system,
            &note,
            &shift_renderer::glyphon::Attrs::new()
                .family(shift_renderer::glyphon::Family::Name(mono_family()))
                .style(shift_renderer::glyphon::Style::Italic),
            shift_renderer::glyphon::Shaping::Advanced,
            None,
        );
        layer.text(nbuf, after_cmd_x, y, theme.text_muted);
    } else if block.cwd.is_some() || block.branch.is_some() {
        let mut ctx = block
            .cwd
            .as_deref()
            .map(super::editor::shorten_home)
            .unwrap_or_default();
        if let Some(br) = &block.branch {
            if !ctx.is_empty() {
                ctx.push(' ');
            }
            ctx.push_str(&format!("git:({br})"));
        }
        // context only where it fits whole (it is secondary to the command)
        let ctx_w = engine.measure(&ctx, 11.5, mono_family(), Weight::NORMAL);
        if after_cmd_x + 3.0 + ctx_w > left_limit {
            ctx.clear();
        }
        let xbuf = make_text(
            engine,
            &ctx,
            11.5,
            grid.line_h,
            mono_family(),
            Weight::NORMAL,
        );
        layer.text(xbuf, after_cmd_x + 3.0, y, theme.text_dim);
    }

    // right side: badge pill, duration, copy icon
    let right = grid.right;
    let icon_w = 13.0;
    let icon_x = right - icon_w - 6.0;

    // copy icon: two overlapping rounded-rect outlines
    let ic = theme.text_muted;
    let cy = y + (grid.line_h - 13.0) / 2.0;
    layer.bordered(
        [icon_x + 4.0, cy + 4.0],
        [8.5, 8.5],
        Color::TRANSPARENT,
        ic,
        2.0,
        1.3,
    );
    layer.bordered([icon_x, cy], [8.5, 8.5], Color::TRANSPARENT, ic, 2.0, 1.3);

    // duration
    let mut cursor_x = icon_x - 10.0;
    if !duration_text.is_empty() {
        let w = duration_text.chars().count() as f32 * grid.cell_w * (12.0 / grid.font_size);
        cursor_x -= w;
        let dbuf = make_text(
            engine,
            &duration_text,
            12.0,
            grid.line_h,
            mono_family(),
            Weight::NORMAL,
        );
        layer.text(dbuf, cursor_x, y, theme.text_muted);
        cursor_x -= 11.0;
    }

    // badge pill
    let badge_x = cursor_x - badge_w;
    let pill_h = 17.0;
    layer.rect(
        [badge_x, y + (grid.line_h - pill_h) / 2.0],
        [badge_w, pill_h],
        badge_fg.with_alpha(0.13),
        5.0,
    );
    if running {
        // 9px ring with its top quarter open, one turn per 0.8 s
        let (cx, cy) = (badge_x + 8.0 + 4.5, y + grid.line_h / 2.0);
        let phase =
            (block_spin_epoch().elapsed().as_secs_f32() / 0.8).fract() * std::f32::consts::TAU;
        for i in 0..9 {
            let a = phase + i as f32 * std::f32::consts::TAU / 12.0;
            layer.circle([cx + 3.7 * a.cos(), cy + 3.7 * a.sin()], 1.7, badge_fg);
        }
    }
    let bbuf = make_text(
        engine,
        &badge_text,
        11.0,
        pill_h,
        mono_family(),
        Weight::SEMIBOLD,
    );
    layer.text(
        bbuf,
        badge_x + 8.0 + spin_w,
        y + (grid.line_h - pill_h) / 2.0 + 1.0,
        badge_fg,
    );

    // clickable regions: chevron collapses, the rest of the header selects
    let chevron_w = BLOCK_INDENT + 22.0;
    out.chevron_hits.push(HitRect {
        rect: [grid.x, y, chevron_w, grid.line_h],
        block: block.id,
    });
    out.hits.push(HitRect {
        rect: [
            grid.x + chevron_w,
            y,
            (badge_x - grid.x - chevron_w).max(0.0),
            grid.line_h,
        ],
        block: block.id,
    });
    // copy button
    out.copy_hits.push(HitRect {
        rect: [icon_x - 4.0, y, icon_w + 10.0, grid.line_h],
        block: block.id,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    struct Noop;
    impl shift_pty::alacritty_terminal::event::EventListener for Noop {}

    /// A 8x2 grid: row 0 "日本x" + soft wrap flag at the last column, row 1
    /// "é" (e + combining acute) then "y".
    fn sample_term() -> shift_pty::alacritty_terminal::term::Term<Noop> {
        use shift_pty::alacritty_terminal::term::{Config, test::TermSize};
        let mut t = shift_pty::alacritty_terminal::term::Term::new(
            Config::default(),
            &TermSize::new(8, 2),
            Noop,
        );
        let g = t.grid_mut();
        for (i, c) in ['日', ' ', '本', ' ', 'x'].into_iter().enumerate() {
            let cell = &mut g[Line(0)][Column(i)];
            cell.c = c;
            if c == '日' || c == '本' {
                cell.flags.insert(Flags::WIDE_CHAR);
            }
        }
        g[Line(0)][Column(1)].flags.insert(Flags::WIDE_CHAR_SPACER);
        g[Line(0)][Column(3)].flags.insert(Flags::WIDE_CHAR_SPACER);
        g[Line(0)][Column(7)].flags.insert(Flags::WRAPLINE);
        g[Line(0)][Column(5)].c = ' ';
        g[Line(1)][Column(0)].c = 'e';
        g[Line(1)][Column(0)].push_zerowidth('\u{301}');
        g[Line(1)][Column(1)].c = 'y';
        t
    }

    #[test]
    fn copy_text_skips_spacers_keeps_marks_and_joins_wraps() {
        let t = sample_term();
        // row 0 soft-wraps: no spacers, trailing blanks kept as content
        assert_eq!(copy_line(&t, 0, 0), Some(("日本x   ".into(), true)));
        assert_eq!(copy_line(&t, 0, 1), Some(("e\u{301}y".into(), false)));
        // search text: no spacers; a wide match spans both columns
        assert_eq!(search_text(&t, 0, 0).unwrap().text, "日本x");
        let m = Matcher::new("日本", true, false).unwrap().unwrap();
        assert_eq!(m.find_line(&search_text(&t, 0, 0).unwrap()), vec![(0, 4)]);
        // combining marks are searchable and map onto their base cell
        let row1 = search_text(&t, 0, 1).unwrap();
        assert_eq!(row1.text, "e\u{301}y");
        let m = Matcher::new("e\u{301}", true, false).unwrap().unwrap();
        assert_eq!(m.find_line(&row1), vec![(0, 1)]);
        let m = Matcher::new("\u{301}y", true, false).unwrap().unwrap();
        assert_eq!(m.find_line(&row1), vec![(0, 2)]);
        let m = Matcher::new("e\u{301}y", true, true).unwrap().unwrap();
        assert_eq!(m.find_line(&row1), vec![(0, 2)]);
    }

    #[test]
    fn hidden_cells_are_not_searchable() {
        let mut t = sample_term();
        let cell = &mut t.grid_mut()[Line(1)][Column(1)];
        cell.flags.insert(Flags::HIDDEN);
        let row1 = search_text(&t, 0, 1).unwrap();
        assert_eq!(row1.text, "e\u{301}");
    }

    #[test]
    fn matcher_skips_wide_glyph_spacers() {
        // cells of "日本語 ok": each wide glyph is followed by a NUL spacer
        let text = "日\0本\0語\0 ok";
        let find = |q: &str, re| Matcher::new(q, true, re).unwrap().unwrap().find(text);
        assert_eq!(find("日本", false), vec![(0, 4)]);
        assert_eq!(find("本語", false), vec![(2, 4)]);
        assert_eq!(find("ok", false), vec![(7, 2)]);
        assert_eq!(find("日.語", true), vec![(0, 6)]);
    }

    #[test]
    fn matcher_modes() {
        let find =
            |q: &str, case, re, text: &str| Matcher::new(q, case, re).unwrap().unwrap().find(text);
        assert_eq!(
            find("ab", false, false, "Ab ab AB"),
            vec![(0, 2), (3, 2), (6, 2)]
        );
        assert_eq!(find("ab", true, false, "Ab ab AB"), vec![(3, 2)]);
        assert_eq!(find(r"\d+", false, true, "a12 b345"), vec![(1, 2), (5, 3)]);
        // columns are chars, not bytes
        assert_eq!(find("b", false, true, "가b"), vec![(1, 1)]);
        assert!(Matcher::new("(", false, true).is_err());
        assert!(Matcher::new("", false, false).unwrap().is_none());
    }

    #[test]
    fn newest_matches_keeps_newest_and_counts_all() {
        let m = Matcher::new("hit", false, false).unwrap().unwrap();
        let cells = |s: &str| SearchLine::from_cells(s);
        let lines = (0..600u64).map(|i| (i, cells(&format!("hit {i}"))));
        let found = newest_matches(lines, &m, 500);
        assert_eq!(found.total, 600);
        assert_eq!(found.anchors.len(), 500);
        assert_eq!(found.anchors.first(), Some(&(100, 0)));
        assert_eq!(found.anchors.last(), Some(&(599, 0)));
        // several hits on one line: the cap keeps that line's last columns
        let lines = vec![(0u64, cells("hit")), (1, cells("hit hit hit"))];
        let found = newest_matches(lines.into_iter(), &m, 2);
        assert_eq!(found.total, 4);
        assert_eq!(found.anchors, vec![(1, 4), (1, 8)]);
        // under the cap: everything, oldest first
        let lines = vec![(3u64, cells("x hit")), (7, cells("hit"))];
        let found = newest_matches(lines.into_iter(), &m, 500);
        assert_eq!(found.total, 2);
        assert_eq!(found.anchors, vec![(3, 2), (7, 0)]);
    }

    #[test]
    fn path_tokens() {
        let line = "error: src/main.rs:42:7: bad thing";
        assert_eq!(
            path_token_at(line, 10),
            Some(("src/main.rs".into(), Some(42)))
        );
        assert_eq!(
            path_token_at("see (~/notes.txt).", 7),
            Some(("~/notes.txt".into(), None))
        );
        assert_eq!(path_token_at("open /tmp", 6), Some(("/tmp".into(), None)));
        // plain words and blanks are not paths
        assert_eq!(path_token_at("hello world", 2), None);
        assert_eq!(path_token_at("a  b", 1), None);
        assert_eq!(path_token_at("x", 5), None);
    }

    #[test]
    fn every_path_token_of_a_line() {
        assert_eq!(
            super::path_tokens("see src/a.rs:3 and ~/b.txt, not https://x.io/y or words"),
            vec![
                (4, 10, "src/a.rs".to_string()),
                (19, 7, "~/b.txt".to_string()),
            ]
        );
        assert!(super::path_tokens("plain words only").is_empty());
    }

    fn block(
        id: BlockId,
        prompt_start: u64,
        output_start: Option<u64>,
        end: Option<u64>,
        state: BlockState,
        collapsed: bool,
    ) -> Block {
        Block {
            id,
            command: format!("cmd{id}"),
            state,
            prompt_start,
            command_start: None,
            output_start,
            end,
            unterminated: false,
            collapsed,
            bookmarked: false,
            cwd: None,
            branch: None,
            restored: false,
            finished_at: None,
        }
    }

    fn done() -> BlockState {
        BlockState::Done {
            exit: Some(0),
            duration: StdDuration::from_millis(1),
        }
    }

    #[test]
    fn url_detection() {
        let found = urls_in("visit https://example.com/docs, then http://a.io.");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].2, "https://example.com/docs");
        assert_eq!(found[1].2, "http://a.io");
        assert!(urls_in("no links here").is_empty());
        // bracketed / quoted URLs stop at delimiters
        let f = urls_in("(see https://x.dev/a)");
        assert_eq!(f[0].2, "https://x.dev/a");
    }

    #[test]
    fn plain_lines_no_blocks() {
        let d = build_display_list(&[], 0, 5, 0, false, false, None);
        assert_eq!(d.len(), 5);
        assert!(matches!(d[0], DisplayLine::Grid(0)));
        assert!(matches!(d[4], DisplayLine::Grid(4)));
    }

    #[test]
    fn header_replaces_prompt_echo() {
        // prompt echo on lines 1..2, output 2..4
        let blocks = vec![block(7, 1, Some(2), Some(4), done(), false)];
        let d = build_display_list(&blocks, 0, 6, 0, false, false, None);
        // 0 grid, header, 2 grid, 3 grid, 4 grid, 5 grid
        assert_eq!(d[0], DisplayLine::Grid(0));
        assert_eq!(d[1], DisplayLine::Header(0));
        assert_eq!(d[2], DisplayLine::Grid(2));
        assert_eq!(d.len(), 6);
    }

    #[test]
    fn squeezed_anchors_never_stall_the_display_list() {
        // output_start at (or above) prompt_start: the header still shows and
        // the walk moves on (this used to spin forever inside redraw)
        for (o, end, collapsed) in [(1, 3, false), (0, 3, false), (1, 1, true), (0, 0, true)] {
            let blocks = vec![block(7, 1, Some(o), Some(end), done(), collapsed)];
            let d = build_display_list(&blocks, 0, 4, 0, false, false, None);
            assert_eq!(d[0], DisplayLine::Grid(0), "{o} {end} {collapsed}");
            assert_eq!(d[1], DisplayLine::Header(0), "{o} {end} {collapsed}");
            assert_eq!(
                d.last(),
                Some(&DisplayLine::Grid(3)),
                "{o} {end} {collapsed}"
            );
        }
    }

    #[test]
    fn collapsed_skips_output() {
        let blocks = vec![block(7, 1, Some(2), Some(4), done(), true)];
        let d = build_display_list(&blocks, 0, 6, 0, false, false, None);
        assert_eq!(
            d,
            vec![
                DisplayLine::Grid(0),
                DisplayLine::Header(0),
                DisplayLine::Grid(4),
                DisplayLine::Grid(5),
            ]
        );
    }

    #[test]
    fn hide_prompt_trims_tail() {
        let blocks = vec![
            block(1, 1, Some(2), Some(3), done(), false),
            block(2, 3, None, None, BlockState::Prompt, false),
        ];
        let d = build_display_list(&blocks, 0, 8, 0, false, true, None);
        // rows 3.. hidden (live prompt owned by editor)
        assert_eq!(
            d,
            vec![
                DisplayLine::Grid(0),
                DisplayLine::Header(0),
                DisplayLine::Grid(2),
            ]
        );
    }

    #[test]
    fn alt_screen_is_raw_rows() {
        let blocks = vec![block(1, 1, Some(2), Some(3), done(), true)];
        let d = build_display_list(&blocks, 100, 4, 50, true, true, None);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0], DisplayLine::Grid(100));
    }

    #[test]
    fn soft_clear_pads_viewport() {
        // block on 0..4 (header at 0, output 1..4); `clear` at line 4 with a
        // 6-row viewport → pads push pre-clear content above the fold
        let blocks = vec![block(1, 0, Some(1), Some(4), done(), false)];
        let d = build_display_list(&blocks, 4, 6, 4, false, false, Some((4, 6)));
        assert_eq!(d[0], DisplayLine::Header(0));
        assert_eq!(d[3], DisplayLine::Grid(3));
        for row in &d[4..10] {
            assert_eq!(*row, DisplayLine::Pad);
        }
        assert_eq!(d[10], DisplayLine::Grid(4));
        assert_eq!(d.len(), 16); // header + 3 output + 6 pads + 6 screen
        // pad older than kept history: ignored
        let d = build_display_list(&blocks, 40, 6, 4, false, false, Some((2, 6)));
        assert!(!d.contains(&DisplayLine::Pad));
    }

    #[test]
    fn scrollback_history_included() {
        let d = build_display_list(&[], 10, 3, 10, false, false, None);
        assert_eq!(d.len(), 13); // 10 history + 3 screen
        assert_eq!(d[0], DisplayLine::Grid(0));
        assert_eq!(d[12], DisplayLine::Grid(12));
    }
}
