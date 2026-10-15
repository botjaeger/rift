use std::time::Duration;

use rift_core::{Block, BlockId, BlockState, Color, CursorStyle, Theme};
use rift_pty::alacritty_terminal::grid::Dimensions;
use rift_pty::alacritty_terminal::index::{Column, Line, Point};
use rift_pty::alacritty_terminal::selection::SelectionRange;
use rift_pty::alacritty_terminal::term::TermMode;
use rift_pty::alacritty_terminal::term::cell::Flags;
use rift_pty::alacritty_terminal::vte::ansi::{self, NamedColor};
use rift_pty::{EventProxy, Session};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

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
    /// Blinking text (SGR 5) is in its visible phase.
    pub text_blink_on: bool,
    /// Blocks start right above the input while they do not fill the pane
    /// (`blocks_start = "bottom"` with the input at the bottom).
    pub bottom_up: bool,
    /// Pointer position (block headers show their copy button on hover).
    pub mouse: Option<(f32, f32)>,
}

/// Set when a frame drew blinking text: the app keeps repainting to blink it.
pub static BLINK_DRAWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    /// Duration rects on block headers (hover: the full time).
    pub duration_hits: Vec<HitRect>,
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
    /// Whether the sticky block has not ended yet (from the blocks snapshot
    /// this frame took: the app must not lock them again while the reader
    /// thread is busy).
    pub sticky_open: bool,
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
) -> Vec<rift_renderer::Sprite> {
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
        out.push(rift_renderer::Sprite {
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

/// The viewport left beside a panel docked over the pane (history,
/// completion) that covers `top..bottom`: the output moves out from under it,
/// as in Warp, so the newest lines stay in view. Render only — the program
/// keeps its rows. At least one row stays.
pub fn beside_panel(grid: GridLayout, top: f32, bottom: f32) -> GridLayout {
    let end = grid.y + grid.rows as f32 * grid.line_h;
    let (y, end) = if bottom >= end {
        (grid.y, top.min(end)) // docked below the output
    } else {
        (bottom.max(grid.y), end) // docked above it (input at the top)
    };
    let rows = ((end - y) / grid.line_h + 1e-3).floor().max(1.0) as usize;
    GridLayout {
        y,
        rows: rows.min(grid.rows),
        ..grid
    }
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
    clear_pad: Option<u64>,
) -> Vec<DisplayLine> {
    build_display_list_filtered(
        blocks,
        scrolled,
        screen_lines,
        history,
        alt_screen,
        hide_prompt,
        clear_pad,
        screen_lines,
        None,
        &std::collections::HashSet::new(),
    )
}

/// Where the live screen's content ends: past the cursor row and the last
/// non-blank line, before the blank screen tail.
pub fn live_end(term: &rift_pty::alacritty_terminal::term::Term<EventProxy>, scrolled: u64) -> u64 {
    let cursor_abs = scrolled + term.grid().cursor.point.line.0.max(0) as u64;
    let newest = scrolled + term.grid().screen_lines() as u64;
    (cursor_abs + 1..newest)
        .rev()
        .find(|&abs| line_text(term, scrolled, abs).is_some_and(|t| !t.trim().is_empty()))
        .map_or(cursor_abs + 1, |abs| abs + 1)
}

/// Blank lines between a finished block's end and the next block's prompt.
fn stray_blank_lines(session: &Session) -> std::collections::HashSet<u64> {
    // the blocks lock is released before the term lock is taken
    let anchors: Vec<(Option<u64>, u64)> = session
        .blocks
        .lock()
        .unwrap()
        .iter()
        .map(|b| (b.end, b.prompt_start))
        .collect();
    let gaps = gaps_between(&anchors);
    if gaps.is_empty() {
        return std::collections::HashSet::new();
    }
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    gaps.into_iter()
        .flat_map(|(from, to)| from..to)
        .filter(|&abs| line_text(&term, scrolled, abs).is_some_and(|t| t.trim().is_empty()))
        .collect()
}

/// `(end, next prompt start)` of each finished block followed by a gap, from
/// each block's `(end, prompt_start)`.
fn gaps_between(anchors: &[(Option<u64>, u64)]) -> Vec<(u64, u64)> {
    anchors
        .windows(2)
        .filter_map(|w| {
            let end = w[0].0?;
            (end < w[1].1).then_some((end, w[1].1))
        })
        .collect()
}

/// Lines the display list skips: blank lines that belong to no block (between
/// a finished block's end and the next prompt — the shell's empty prompts
/// for Enter pressed while a command ran), and the output lines of the
/// filtered block that do not contain the filter query (case-insensitive;
/// Warp block filter).
pub fn filter_hidden(
    session: &Session,
    filter: Option<&(BlockId, String)>,
) -> std::collections::HashSet<u64> {
    let mut hidden = stray_blank_lines(session);
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
    // decomposed like the searched text (`SearchLine::push`)
    use unicode_normalization::UnicodeNormalization as _;
    let query = query.nfd().collect::<String>().to_lowercase();
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
/// (block filter). `view_rows` = the rows the pane shows (fewer than
/// `screen_lines` beside a docked panel): soft-`clear` padding fills them.
/// `bottom_up` = `Some(live end)`: content shorter than the view sits at
/// its bottom (pads above it), and after a `clear` the new content does
/// too; the blank screen rows past the live end (`live_end`) are left out,
/// so a command just submitted or running stays down there instead of
/// riding up on them.
#[allow(clippy::too_many_arguments)]
pub fn build_display_list_filtered(
    blocks: &[Block],
    scrolled: u64,
    screen_lines: usize,
    history: u64,
    alt_screen: bool,
    hide_prompt: bool,
    clear_pad: Option<u64>,
    view_rows: usize,
    bottom_up: Option<u64>,
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
    if let Some(end) = bottom_up {
        newest = newest.min(end.max(oldest));
    }
    let bottom_up = bottom_up.is_some();
    // soft `clear`: pad rows push pre-clear content above the fold
    let mut pad_pending = clear_pad.filter(|line| *line >= oldest);
    let mut pad_at = None;
    let mut abs = oldest;
    while abs < newest {
        if pad_pending == Some(abs) {
            pad_at = Some(display.len());
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
    if let Some(line) = pad_pending
        && line >= newest
    {
        pad_at = Some(display.len());
    }
    // the pads fill the viewport around the post-clear content: below it,
    // so it starts at the top of the viewport, or (bottom_up) above it, so
    // it starts right above the input (Warp); they vanish once it fills
    // the screen
    if let Some(at) = pad_at {
        let pads = view_rows.saturating_sub(display.len() - at);
        let pad = std::iter::repeat_n(DisplayLine::Pad, pads);
        if bottom_up {
            display.splice(at..at, pad);
        } else {
            display.extend(pad);
        }
    }
    if bottom_up && display.len() < view_rows {
        let pads = view_rows - display.len();
        display.splice(0..0, std::iter::repeat_n(DisplayLine::Pad, pads));
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

fn rgb_color(c: ansi::Rgb) -> Color {
    Color::rgba(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        1.0,
    )
}

/// Palette slot `index` as the program sees it: its own OSC 4/10/11/12
/// setting, else the theme's (0–255 palette, 256 foreground, 257
/// background, 258 cursor).
pub fn palette_color(theme: &Theme, session: &Session, index: usize) -> Option<Color> {
    if let Some(rgb) = session.term.lock().colors()[index] {
        return Some(rgb_color(rgb));
    }
    theme_color(theme, index)
}

/// Palette slot `index` of the theme alone.
fn theme_color(theme: &Theme, index: usize) -> Option<Color> {
    match index {
        0..=255 => Some(indexed_color(theme, index as u8)),
        256 => Some(theme.fg),
        257 => Some(theme.bg.with_alpha(1.0)),
        258 => Some(theme.cursor),
        _ => None,
    }
}

/// Every theme palette slot (0–258) as the bytes a color query reports.
pub fn theme_rgbs(theme: &Theme) -> Vec<ansi::Rgb> {
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    (0..=258)
        .filter_map(|i| theme_color(theme, i))
        .map(|c| ansi::Rgb {
            r: to8(c.r),
            g: to8(c.g),
            b: to8(c.b),
        })
        .collect()
}

/// This pane's default colors when the program changed them: OSC 4 for the
/// 16 ANSI colors, OSC 10/11/12 for foreground, background and cursor, and
/// DECSCNM (`CSI ? 5 h`) swapping foreground and background. `None` while
/// the theme applies unchanged.
pub fn pane_theme(theme: &Theme, session: &Session) -> Option<Theme> {
    term_theme(theme, &session.term.lock())
}

/// [`pane_theme`] of a locked term.
fn term_theme<T>(
    theme: &Theme,
    term: &rift_pty::alacritty_terminal::term::Term<T>,
) -> Option<Theme> {
    let reversed = term.reverse_screen();
    let colors = term.colors();
    let overridden = (0..16).chain(256..=258).any(|i| colors[i].is_some());
    if !reversed && !overridden {
        return None;
    }
    let mut t = theme.clone();
    for (i, slot) in t.ansi.iter_mut().enumerate() {
        if let Some(rgb) = colors[i] {
            *slot = rgb_color(rgb);
        }
    }
    if let Some(rgb) = colors[NamedColor::Foreground as usize] {
        t.fg = rgb_color(rgb);
    }
    if let Some(rgb) = colors[NamedColor::Background as usize] {
        t.bg = rgb_color(rgb);
    }
    if let Some(rgb) = colors[NamedColor::Cursor as usize] {
        t.cursor = rgb_color(rgb);
    }
    if reversed {
        std::mem::swap(&mut t.fg, &mut t.bg);
        // the block header's command reads on the light ground too
        t.text_bright = t.fg;
        t.text_ui = t.fg;
    }
    Some(t)
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

/// The full duration for a tooltip: `412 ms`, `8.214 s`, `1 min 18.432 s`,
/// `2 h 3 min 4 s`.
pub fn format_duration_full(d: Duration) -> String {
    let ms = d.as_millis();
    let secs = d.as_secs_f64();
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{secs:.3} s")
    } else if ms < 3_600_000 {
        format!("{} min {:.3} s", ms / 60_000, secs % 60.0)
    } else {
        let s = d.as_secs();
        format!("{} h {} min {} s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

/// A block header's duration tooltip: how long it ran, and when it ended
/// (`Took 1 min 18.432 s · finished 03:48:05 pm, October 11, 2026`).
pub fn duration_detail(block: &Block) -> Option<String> {
    match block.state {
        BlockState::Running { started } => Some(format!(
            "Running for {}",
            format_duration_full(started.elapsed())
        )),
        BlockState::Done { duration, .. } => {
            let took = format!("Took {}", format_duration_full(duration));
            if block.restored {
                return Some(format!("{took} · restored from a previous session"));
            }
            let Some(at) = block.finished_at else {
                return Some(took);
            };
            let t = super::prompt_chips::LocalTime::at(at);
            let secs = at
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() % 60);
            // `03:48 pm` with the seconds spliced in
            let clock = t.time12().replacen(' ', &format!(":{secs:02} "), 1);
            Some(format!("{took} · finished {clock}, {}", t.date()))
        }
        BlockState::Prompt => None,
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
        use rift_renderer::glyphon::cosmic_text::{FeatureTag, FontFeatures};
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
        // lines are searched decomposed (`SearchLine::push`)
        use unicode_normalization::UnicodeNormalization as _;
        let query: String = query.nfd().collect();
        let query = query.as_str();
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
    term: &rift_pty::alacritty_terminal::term::Term<EventProxy>,
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
    /// One char of the cell at `col..end`, stored canonically decomposed
    /// (NFD) so "é" and "e\u{301}" match each other; every char of the
    /// decomposition maps to the same cell.
    fn push(&mut self, c: char, col: usize, end: usize) {
        unicode_normalization::char::decompose_canonical(c, |d| {
            self.text.push(d);
            self.cols.push(col);
            self.ends.push(end);
        });
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
    term: &rift_pty::alacritty_terminal::term::Term<T>,
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

/// How a cell's cluster asks to be drawn: VS16 (U+FE0F) selects the emoji,
/// VS15 (U+FE0E) the text glyph. Font fallback alone picks whichever font
/// has the character first (a text heart for "❤️", a color watch for "⌚︎").
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Presentation {
    Default,
    Emoji,
    /// The text glyph, from this monochrome family.
    Text(&'static str),
}

fn presentation(zerowidth: Option<&[char]>) -> Presentation {
    match zerowidth {
        Some(zw) if zw.contains(&'\u{fe0f}') => Presentation::Emoji,
        Some(zw) if zw.contains(&'\u{fe0e}') => Presentation::Text(SYMBOL_FAMILY),
        _ => Presentation::Default,
    }
}

/// How `ch` draws: an emoji that defaults to text (⏱, ☀, ✔) and comes
/// without VS16 takes the text glyph, as in Warp and iTerm2. Fallback alone
/// would reach the color font first whenever the terminal font lacks it.
/// The text glyph comes from the first symbol font that has it; with none,
/// the emoji stays.
fn resolve_presentation(engine: &mut TextEngine, ch: char, p: Presentation) -> Presentation {
    use rift_pty::alacritty_terminal::term::width;
    // and one that defaults to emoji (⌚, ⏰) is drawn in color
    if p == Presentation::Default && !ch.is_ascii() && width::emoji_default(ch) {
        return Presentation::Emoji;
    }
    let wants_text = match p {
        Presentation::Text(_) => true,
        Presentation::Default => {
            !ch.is_ascii()
                && width::text_default_emoji(ch)
                && !engine.family_covers(mono_family(), ch)
        }
        Presentation::Emoji => false,
    };
    if !wants_text {
        return p;
    }
    match TEXT_FAMILIES.iter().find(|f| engine.family_covers(f, ch)) {
        Some(f) => Presentation::Text(f),
        None => p,
    }
}

/// Color emoji font for emoji presentation (fallback covers its absence).
#[cfg(target_os = "macos")]
const EMOJI_FAMILY: &str = "Apple Color Emoji";
#[cfg(not(target_os = "macos"))]
const EMOJI_FAMILY: &str = "Noto Color Emoji";
/// Monochrome symbol font for text presentation.
#[cfg(target_os = "macos")]
const SYMBOL_FAMILY: &str = "Apple Symbols";
#[cfg(not(target_os = "macos"))]
const SYMBOL_FAMILY: &str = "DejaVu Sans";
/// Monochrome fonts tried in order for a text glyph (STIX Two Math is the
/// only macOS font with a text ⏱).
#[cfg(target_os = "macos")]
const TEXT_FAMILIES: &[&str] = &[SYMBOL_FAMILY, "STIX Two Math", "Menlo"];
#[cfg(not(target_os = "macos"))]
const TEXT_FAMILIES: &[&str] = &[SYMBOL_FAMILY, "Noto Sans Symbols 2", "Noto Sans Math"];

const WIDE_SPACERS: Flags = Flags::WIDE_CHAR_SPACER.union(Flags::LEADING_WIDE_CHAR_SPACER);

/// The glyph a cell draws. alacritty keeps a tab's first cell as '\t' (for
/// copying); shaped, it would jump to the font's tab stop and push the rest
/// of the row right (vttest's "tab to right margin" letters landed past
/// column 80). Hidden (SGR 8) text draws nothing.
fn drawn_char(c: char, flags: Flags) -> char {
    if flags.contains(Flags::HIDDEN) || c == '\t' {
        ' '
    } else {
        c
    }
}

/// A terminal grid (the shown one, or the primary one under the alternate
/// screen).
type CellGrid =
    rift_pty::alacritty_terminal::grid::Grid<rift_pty::alacritty_terminal::term::cell::Cell>;

/// Text of one grid line for copying: no wide-glyph spacers, combining marks
/// kept. The flag is true when the row soft-wraps into the next one (its
/// trailing spaces are content then and are kept).
fn copy_line(g: &CellGrid, scrolled: u64, abs: u64) -> Option<(String, bool)> {
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
    copy_line(term.grid(), scrolled, abs).map(|(text, _)| text.trim_end_matches(' ').to_string())
}

/// OSC 8 hyperlink runs on one absolute line: (col, len, uri).
fn hyperlinks_in(
    term: &rift_pty::alacritty_terminal::term::Term<EventProxy>,
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

/// Match anchors over the absolute lines `shown` (oldest first: the rows
/// the pane draws) — see [`newest_matches`].
pub fn collect_matches(session: &Session, shown: &[u64], matcher: &Matcher, cap: usize) -> Found {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let lines = shown
        .iter()
        .filter_map(|&abs| Some((abs, search_text(&term, scrolled, abs)?)));
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
    // the output's rows are on the primary grid, also under a full-screen
    // program
    let g = term.primary_grid();
    let mut out = String::new();
    // rows gone from scrollback have no text
    for abs in start.max(oldest_row(g, scrolled))..end {
        if let Some((text, wrapped)) = copy_line(g, scrolled, abs) {
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

/// [`block_output_text`] bounded for the remote gateway
/// ([`rift_remote::bound_output`]: the last 500 lines, at most 64 KiB), also
/// for a running block (output start through the cursor row). Only the
/// rows of the kept tail are read as text; lines before it, and rows gone
/// from scrollback, count as dropped. Returns (text, dropped lines).
pub fn block_output_tail(session: &Session, block_id: BlockId) -> Option<(String, u64)> {
    let rows = {
        let blocks = session.blocks.lock().unwrap();
        output_rows(blocks.iter().find(|b| b.id == block_id)?)?
    };
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    Some(output_tail(&term, scrolled, rows))
}

/// Where a block's output lies: its first absolute row, the row after its
/// last (`None` while it runs: through the cursor row) and whether its last
/// line is unfinished. `None` before it has output.
fn output_rows(block: &Block) -> Option<(u64, Option<u64>, bool)> {
    match block.state {
        BlockState::Running { .. } => Some((block.output_start?, None, true)),
        _ => {
            let (start, end) = block.output_range()?;
            Some((start, Some(end), block.unterminated))
        }
    }
}

/// The row after a block's last (`end`, or for a running block the one
/// after the cursor row).
fn output_end(g: &CellGrid, scrolled: u64, end: Option<u64>) -> u64 {
    // running: the cursor row is the last, and its line break is not output yet
    end.unwrap_or_else(|| scrolled + g.cursor.point.line.0.max(0) as u64 + 1)
}

/// The oldest absolute row `g` still holds (rows before it are gone from
/// scrollback).
fn oldest_row(g: &CellGrid, scrolled: u64) -> u64 {
    scrolled.saturating_sub(g.history_size() as u64)
}

/// Whether absolute row `abs` soft-wraps into the next one; `None` when it
/// is gone from scrollback (or not on the grid).
fn row_wraps(g: &CellGrid, scrolled: u64, abs: u64) -> Option<bool> {
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 || g.columns() == 0 {
        return None;
    }
    Some(
        g[Line(rel as i32)][Column(g.columns() - 1)]
            .flags
            .contains(Flags::WRAPLINE),
    )
}

/// [`block_output_tail`] of a locked term: (text, lines before it). Read
/// from the primary grid, where the output is also while a full-screen
/// program shows the alternate one.
fn output_tail<T>(
    term: &rift_pty::alacritty_terminal::term::Term<T>,
    scrolled: u64,
    (start, end, unterminated): (u64, Option<u64>, bool),
) -> (String, u64) {
    let g = term.primary_grid();
    let end = output_end(g, scrolled, end);
    let wrapped = |abs: u64| row_wraps(g, scrolled, abs);
    // read rows backwards up to the start of the line that makes the tail
    // longer than the bound (in lines or bytes); the bound cuts the rest
    let mut rows = Vec::new();
    let mut first = end;
    let (mut lines, mut bytes) = (0usize, 0usize);
    while first > start {
        let abs = first - 1;
        let Some(row) = copy_line(g, scrolled, abs) else {
            break; // gone from scrollback
        };
        bytes += row.0.len() + 1;
        rows.push(row);
        first = abs;
        if abs == start || wrapped(abs - 1) != Some(true) {
            lines += 1;
            if lines > rift_remote::MAX_OUTPUT_LINES || bytes > rift_remote::MAX_OUTPUT_BYTES {
                break;
            }
        }
    }
    // lines before the tail: each row gone from scrollback counts as one
    // (its soft wraps are unknown, so for a block trimmed at the head this
    // is an estimate), counted without visiting them; held rows by their
    // line ends
    let held = start.max(oldest_row(g, scrolled)).min(first);
    let skipped = (held - start)
        + (held..first)
            .filter(|&abs| wrapped(abs) != Some(true))
            .count() as u64;
    let mut out = String::with_capacity(bytes);
    for (text, wrapped) in rows.into_iter().rev() {
        out.push_str(&text);
        // a soft-wrapped row continues on the next one: no line break
        if !wrapped {
            out.push('\n');
        }
    }
    if unterminated && out.ends_with('\n') {
        out.pop();
    }
    let (out, dropped) = rift_remote::bound_output(&out);
    (out, skipped + dropped)
}

/// Output lines `from` … of a block for the remote gateway's `output`: at
/// most `limit` lines (capped at 500) and 64 KiB ([`rift_remote::bound_page`]),
/// numbered from the block's first output line the way
/// [`block_output_tail`]'s dropped count is (soft-wrapped rows joined, a row
/// gone from scrollback one line). `None` for a block the gateway never
/// sends (unknown, or being typed at the prompt).
pub fn block_output_range(
    session: &Session,
    block_id: BlockId,
    from: u64,
    limit: usize,
) -> Option<rift_remote::OutputPage> {
    let rows = {
        let blocks = session.blocks.lock().unwrap();
        let block = blocks.iter().find(|b| b.id == block_id)?;
        if !remote_sent(block) {
            return None;
        }
        output_rows(block)
    };
    let Some(rows) = rows else {
        return Some(rift_remote::OutputPage {
            from,
            ..Default::default()
        });
    };
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    Some(output_range(&term, scrolled, rows, from, limit))
}

/// [`block_output_range`] of a locked term, read from the primary grid
/// (see [`output_tail`]).
fn output_range<T>(
    term: &rift_pty::alacritty_terminal::term::Term<T>,
    scrolled: u64,
    (start, end, unterminated): (u64, Option<u64>, bool),
    from: u64,
    limit: usize,
) -> rift_remote::OutputPage {
    let g = term.primary_grid();
    let limit = limit.min(rift_remote::MAX_OUTPUT_PAGE_LINES) as u64;
    let end = output_end(g, scrolled, end);
    // each row gone from scrollback is one line: counted, never visited
    let held = start.max(oldest_row(g, scrolled)).min(end.max(start));
    let mut total = held - start;
    // the first line held at or after `from`, the text from there
    let mut first: Option<u64> = None;
    let mut text = String::new();
    let mut reached_end = false;
    for abs in held..end {
        if abs == start || row_wraps(g, scrolled, abs - 1) != Some(true) {
            total += 1;
        }
        let line = total - 1;
        // flags only from here: the text is read for the page's rows alone
        if line < from
            || first.is_some_and(|f| line >= f + limit)
            || text.len() > rift_remote::MAX_OUTPUT_BYTES
        {
            continue;
        }
        let Some((row, wraps)) = copy_line(g, scrolled, abs) else {
            continue; // not on the grid
        };
        first.get_or_insert(line);
        text.push_str(&row);
        if !wraps {
            text.push('\n');
        }
        reached_end = abs + 1 == end;
    }
    if unterminated && reached_end && text.ends_with('\n') {
        text.pop();
    }
    rift_remote::OutputPage {
        from: first.unwrap_or(from.max(total)),
        text: rift_remote::bound_page(&text, limit as usize).to_string(),
        total,
    }
}

/// A block the remote gateway sends: one that ran or runs (not a prompt
/// being typed, not an empty command line).
fn remote_sent(b: &Block) -> bool {
    match b.state {
        BlockState::Prompt => false,
        BlockState::Running { .. } => true,
        BlockState::Done { .. } => !b.command.is_empty(),
    }
}

/// The pane's visible screen for the remote gateway: none unless it shows
/// the alternate screen or `force`. Colours resolve like the renderer's
/// (the pane's theme with the program's palette changes, reverse video
/// swapped); the theme's default foreground and background are `None`.
/// Returns the session's output generation with it, read under the same
/// term lock; while that is still `since`, no screen is built (`None`: the
/// caller's is current).
pub fn remote_screen(
    session: &Session,
    theme: &Theme,
    force: bool,
    since: Option<u64>,
) -> (u64, Option<Option<rift_remote::Screen>>) {
    let term = session.term.lock();
    let generation = session
        .generation
        .load(std::sync::atomic::Ordering::Acquire);
    if since == Some(generation) {
        return (generation, None);
    }
    (generation, Some(screen_of(&term, theme, force)))
}

/// [`remote_screen`] of a locked term.
fn screen_of<T>(
    term: &rift_pty::alacritty_terminal::term::Term<T>,
    theme: &Theme,
    force: bool,
) -> Option<rift_remote::Screen> {
    use rift_remote::protocol::Run;
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    if !alt && !force {
        return None;
    }
    let pane = term_theme(theme, term);
    let theme = pane.as_ref().unwrap_or(theme);
    // 256-color slots the program redefined (OSC 4); the 16 ANSI ones and
    // the defaults are in `theme` already (as in `render_row`)
    let palette = |c: ansi::Color| match c {
        ansi::Color::Indexed(i) if i >= 16 => term.colors()[i as usize]
            .map(rgb_color)
            .unwrap_or_else(|| resolve_color(theme, c)),
        c => resolve_color(theme, c),
    };
    let g = term.grid();
    let cols = g.columns().min(rift_remote::MAX_SCREEN_COLS as usize);
    let rows = g.screen_lines().min(rift_remote::MAX_SCREEN_ROWS as usize);
    // cells compare by their 8-bit colours (what the hex says); the hex
    // text is made once per run
    let rgb8 = |c: Color| {
        let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [b(c.r), b(c.g), b(c.b)]
    };
    let hex = |[r, g, b]: [u8; 3]| format!("{r:02x}{g:02x}{b:02x}");
    let (default_fg, default_bg) = (rgb8(theme.fg), rgb8(theme.bg));
    // a run's style: (fg, bg, flags), `None` for a default colour
    type Style = (Option<[u8; 3]>, Option<[u8; 3]>, u8);
    let mut lines = Vec::with_capacity(rows);
    for r in 0..rows {
        let row = &g[Line(r as i32)];
        let mut runs: Vec<Run> = Vec::new();
        // the last run's style
        let mut style: Option<Style> = None;
        for col in 0..cols {
            let cell = &row[Column(col)];
            let flags = cell.flags;
            if flags.intersects(WIDE_SPACERS) {
                continue;
            }
            let (mut fg, mut bg) = (palette(cell.fg), palette(cell.bg));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let (fg, bg) = (rgb8(fg), rgb8(bg));
            let cell_style = (
                (fg != default_fg).then_some(fg),
                (bg != default_bg).then_some(bg),
                [
                    (Flags::BOLD, Run::BOLD),
                    (Flags::ITALIC, Run::ITALIC),
                    (Flags::ALL_UNDERLINES, Run::UNDERLINE),
                    (Flags::DIM, Run::DIM),
                    (Flags::STRIKEOUT, Run::STRIKE),
                ]
                .iter()
                .filter(|(f, _)| flags.intersects(*f))
                .fold(0, |acc, (_, bit)| acc | bit),
            );
            if style != Some(cell_style) {
                style = Some(cell_style);
                runs.push(Run {
                    text: String::new(),
                    fg: cell_style.0.map(hex),
                    bg: cell_style.1.map(hex),
                    flags: cell_style.2,
                });
            }
            let run = runs.last_mut().expect("a run per style");
            // a wide glyph whose second half is cut off would make the row
            // one column too wide: a space stands in for it
            if flags.contains(Flags::WIDE_CHAR) && col + 1 == cols {
                run.text.push(' ');
                continue;
            }
            run.text.push(drawn_char(cell.c, flags));
            if !flags.contains(Flags::HIDDEN)
                && let Some(zw) = cell.zerowidth()
            {
                run.text.extend(zw.iter().take(rift_remote::MAX_CELL_MARKS));
            }
        }
        // blank cells in the default style at the end of the row
        if let Some(last) = runs.last_mut()
            && (&last.fg, &last.bg, last.flags) == (&None, &None, 0)
        {
            let kept = last.text.trim_end_matches(' ').len();
            last.text.truncate(kept);
            if last.text.is_empty() {
                runs.pop();
            }
        }
        lines.push(runs);
    }
    let cursor = term.grid().cursor.point;
    let shown = term.mode().contains(TermMode::SHOW_CURSOR)
        && (cursor.line.0 as usize) < rows
        && cursor.column.0 < cols;
    Some(rift_remote::Screen {
        cols: cols as u16,
        rows: rows as u16,
        alt,
        cursor: shown.then_some([cursor.line.0 as u16, cursor.column.0 as u16]),
        lines,
    })
}

/// Blocks of `session` for the remote gateway: commands that ran or run (a
/// prompt being typed is skipped), older than `before` when given, at most
/// `limit` (capped at 30), newest last; whether older ones exist; and the
/// pane's oldest such block (ids below it are gone). `meta`: without
/// output (empty, 0 dropped), no grid read at all.
pub fn remote_blocks(
    session: &Session,
    before: Option<BlockId>,
    limit: usize,
    meta: bool,
) -> rift_remote::Page {
    use rift_remote::protocol::{Block as Remote, BlockState as State};
    let limit = limit.min(rift_remote::MAX_BLOCKS);
    let sent = |b: &&Block| remote_sent(b);
    // snapshot first: block_output_tail takes the blocks lock itself
    let (mut picked, oldest) = {
        let store = session.blocks.lock().unwrap();
        let picked: Vec<Block> = store
            .iter()
            .rev()
            .filter(|b| before.is_none_or(|x| b.id < x))
            .filter(sent)
            .take(limit + 1)
            .cloned()
            .collect();
        (picked, store.iter().find(sent).map(|b| b.id))
    };
    let has_older = picked.len() > limit;
    picked.truncate(limit);
    picked.reverse();
    let blocks = picked
        .into_iter()
        .map(|b| {
            let (output, dropped) = if meta {
                Default::default()
            } else {
                block_output_tail(session, b.id).unwrap_or_default()
            };
            let (state, exit, duration) = match b.state {
                BlockState::Running { started } => (State::Running, None, started.elapsed()),
                BlockState::Done { exit, duration } => (State::Done, exit, duration),
                BlockState::Prompt => (State::Running, None, Duration::ZERO),
            };
            Remote {
                id: b.id,
                command: b.command,
                state,
                exit,
                duration_ms: duration.as_millis() as u64,
                cwd: b.cwd,
                branch: b.branch,
                output,
                dropped,
                restored: b.restored,
            }
        })
        .collect();
    rift_remote::Page {
        blocks,
        has_older,
        oldest,
    }
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
        grid.rows,
        (grid.bottom_up && !alt_screen).then(|| live_end(&term, scrolled)),
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
    let live_end = live_end(&term, scrolled);

    // keyboard-selected block: abs row range for the highlight treatment
    let selected_range = selected.and_then(|id| {
        blocks
            .iter()
            .find(|b| b.id == id)
            .map(|b| (b.prompt_start, b.end.unwrap_or(live_end)))
    });
    let row_selected = |abs: u64| selected_range.is_some_and(|(s, e)| abs >= s && abs < e);
    // a selected failed block keeps its error color; any other takes the accent
    let sel_color = selected
        .and_then(|id| blocks.iter().find(|b| b.id == id))
        .map_or(theme.accent, |b| selection_color(theme, b));

    // only the selected block gets a wash; exit state lives on the left
    // border and badge alone (design 01: unselected failed blocks are not tinted)
    let row_bg =
        |abs: u64| -> Option<Color> { row_selected(abs).then(|| sel_color.with_alpha(0.08)) };

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
                    wash_row(layer, grid, y, bg);
                }
                let content_x = if alt_screen {
                    grid.x
                } else if let Some(color) = border_color(*abs) {
                    if is_sel {
                        layer.rect([grid.x, y], [3.0, border_h(r)], sel_color, 0.0);
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
                    wash_row(layer, grid, y, sel_color.with_alpha(0.08));
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
                let sel = selected == Some(block.id);
                render_header(
                    layer, engine, theme, grid, block, y, bh, sel, note, &mut out,
                );
            }
        }
    }

    // sticky command header: first visible row belongs to a block whose
    // header row is above the viewport
    if !alt_screen && let Some(DisplayLine::Grid(top_abs)) = visible.first() {
        let sticky = blocks.iter().find(|b| {
            b.suppressed_range()
                .is_some_and(|(s, _)| s < *top_abs && *top_abs < b.end.unwrap_or(u64::MAX))
        });
        out.sticky_block = sticky.map(|b| b.id);
        out.sticky_open = sticky.is_some_and(|b| b.end.is_none());
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
    selected: bool,
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
        selected,
        None,
        &mut sink,
    );
    Some(rect)
}

/// Edge and wash color of a selected block: a failed command stays red,
/// anything else takes the accent.
fn selection_color(theme: &Theme, block: &Block) -> Color {
    match block.state {
        BlockState::Done { exit: Some(n), .. } if n != 0 => theme.error,
        _ => theme.accent,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_grid_line(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    term: &rift_pty::alacritty_terminal::term::Term<EventProxy>,
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
    // DECDWL/DECDHL: every cell is two columns wide
    let attr = LineSize::of(row.attr);
    let sx = if attr == LineSize::Single { 1.0 } else { 2.0 };

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
        presentation: Presentation,
    }
    let mut spans: Vec<Span> = Vec::new();
    let cols = (g.columns() / sx as usize).min(grid.cols + 8);
    // cell backgrounds (merged runs, block cursor) draw before decorations
    let mut backs: Vec<([f32; 2], [f32; 2], Color, f32)> = Vec::new();
    let mut decos: Vec<([f32; 2], [f32; 2], Color)> = Vec::new();
    let mut blocks: Vec<([f32; 2], [f32; 2], Color)> = Vec::new();

    for col in 0..cols {
        let cell = &row[Column(col)];
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }

        // 256-color slots the program redefined (OSC 4); the 16 ANSI ones
        // and the defaults arrive already applied in `theme` (`pane_theme`)
        let palette = |c: ansi::Color| match c {
            ansi::Color::Indexed(i) if i >= 16 => term.colors()[i as usize]
                .map(rgb_color)
                .unwrap_or_else(|| resolve_color(theme, c)),
            c => resolve_color(theme, c),
        };
        let mut fg = palette(cell.fg);
        let mut bg = palette(cell.bg);
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
        } * sx;
        let cx = x + col as f32 * grid.cell_w * sx;

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
        let mut ch = drawn_char(cell.c, flags);
        // block elements fill the whole cell (line height included), as
        // in Warp: the font's glyphs stop short of the line box and left
        // gaps in block art (Claude Code's logo)
        if attr == LineSize::Single
            && !flags.contains(Flags::HIDDEN)
            && let Some((parts, alpha)) = block_element(ch)
        {
            let ink = fg.with_alpha(fg.a * alpha);
            // half a pixel of overlap hides the antialiased seams between
            // parts and neighbouring cells (opaque ink only: translucent
            // shades would darken where they overlap)
            let pad = if alpha >= 1.0 { 0.5 } else { 0.0 };
            for [x0, y0, x1, y1] in parts {
                blocks.push((
                    [cx + x0 * cell_px, y + y0 * grid.line_h],
                    [(x1 - x0) * cell_px + pad, (y1 - y0) * grid.line_h + pad],
                    ink,
                ));
            }
            ch = ' ';
        }
        let presentation = resolve_presentation(engine, ch, presentation(cell.zerowidth()));
        if flags.contains(Flags::BLINK) {
            BLINK_DRAWN.store(true, std::sync::atomic::Ordering::Relaxed);
            if !grid.text_blink_on {
                ch = ' ';
            }
        }

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

        if !spans.last().is_some_and(|l| {
            l.fg == fg && l.weight == weight && l.italic == italic && l.presentation == presentation
        }) {
            spans.push(Span {
                text: String::new(),
                fg,
                weight,
                italic,
                presentation,
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
    for (pos, size, color) in blocks.into_iter().chain(decos) {
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
    attr.hash(&mut h);
    grid.cell_w.to_bits().hash(&mut h);
    for s in &spans {
        s.text.hash(&mut h);
        s.weight.0.hash(&mut h);
        s.italic.hash(&mut h);
        s.presentation.hash(&mut h);
        s.fg.r.to_bits().hash(&mut h);
        s.fg.g.to_bits().hash(&mut h);
        s.fg.b.to_bits().hash(&mut h);
        s.fg.a.to_bits().hash(&mut h);
    }
    // double-height lines shape at twice the size and show one half;
    // double-width lines space each glyph across two cells
    let tall = matches!(attr, LineSize::Top | LineSize::Bottom);
    let (font_size, line_h) = if tall {
        (grid.font_size * 2.0, grid.line_h * 2.0)
    } else {
        (grid.font_size, grid.line_h)
    };
    let spacing = if attr == LineSize::Wide {
        grid.cell_w / grid.font_size
    } else {
        0.0
    };
    let buf = engine.shaped(h.finish(), |fs| {
        let mut buf = Buffer::new(fs, Metrics::new(font_size, line_h));
        buf.set_size(fs, None, Some(line_h));
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|s| {
                let attrs = mono_attrs(grid.ligatures)
                    .weight(s.weight)
                    .color(rift_renderer::text_color(s.fg));
                // only DECDWL lines set tracking
                let attrs = if spacing > 0.0 {
                    attrs.letter_spacing(spacing)
                } else {
                    attrs
                };
                let attrs = if s.italic {
                    attrs.style(rift_renderer::glyphon::Style::Italic)
                } else {
                    attrs
                };
                let attrs = match s.presentation {
                    Presentation::Default => attrs,
                    Presentation::Emoji => attrs.family(Family::Name(EMOJI_FAMILY)),
                    Presentation::Text(family) => attrs.family(Family::Name(family)),
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
    match attr {
        LineSize::Single => layer.text(buf, x, y, theme.fg),
        LineSize::Wide => layer.text(buf, x + grid.cell_w / 2.0, y, theme.fg),
        LineSize::Top | LineSize::Bottom => {
            let bounds = TextBounds {
                left: x.floor() as i32,
                top: y.floor() as i32,
                right: (x + cols as f32 * grid.cell_w * sx).ceil() as i32,
                bottom: (y + grid.line_h).ceil() as i32,
            };
            let top = if attr == LineSize::Top {
                y
            } else {
                y - grid.line_h
            };
            layer.text_clipped(buf, x, top, theme.fg, bounds);
        }
    }
}

/// Block element `c` (U+2580–U+259F) as filled parts of the cell, each
/// `[x0, y0, x1, y1]` in cell fractions, and the ink's opacity (the shades
/// ░▒▓ are the full cell at 25/50/75 %). None for any other character.
fn block_element(c: char) -> Option<(Vec<[f32; 4]>, f32)> {
    const UL: [f32; 4] = [0.0, 0.0, 0.5, 0.5];
    const UR: [f32; 4] = [0.5, 0.0, 1.0, 0.5];
    const LL: [f32; 4] = [0.0, 0.5, 0.5, 1.0];
    const LR: [f32; 4] = [0.5, 0.5, 1.0, 1.0];
    let eighth = |n: u32| n as f32 / 8.0;
    let parts = match c as u32 {
        0x2580 => vec![[0.0, 0.0, 1.0, 0.5]],
        // lower one to seven eighths, then the full block
        n @ 0x2581..=0x2588 => vec![[0.0, 1.0 - eighth(n - 0x2580), 1.0, 1.0]],
        // left seven eighths down to one eighth
        n @ 0x2589..=0x258F => vec![[0.0, 0.0, eighth(0x2590 - n), 1.0]],
        0x2590 => vec![[0.5, 0.0, 1.0, 1.0]],
        0x2591 => return Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.25)),
        0x2592 => return Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.5)),
        0x2593 => return Some((vec![[0.0, 0.0, 1.0, 1.0]], 0.75)),
        0x2594 => vec![[0.0, 0.0, 1.0, eighth(1)]],
        0x2595 => vec![[eighth(7), 0.0, 1.0, 1.0]],
        0x2596 => vec![LL],
        0x2597 => vec![LR],
        0x2598 => vec![UL],
        0x2599 => vec![UL, LL, LR],
        0x259A => vec![UL, LR],
        0x259B => vec![UL, UR, LL],
        0x259C => vec![UL, UR, LR],
        0x259D => vec![UR],
        0x259E => vec![UR, LL],
        0x259F => vec![UR, LL, LR],
        _ => return None,
    };
    Some((parts, 1.0))
}

/// A line's DEC size (`row.attr`: ESC # 3/4/6, ESC # 5 resets).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum LineSize {
    Single,
    /// DECDWL: double width.
    Wide,
    /// DECDHL top half.
    Top,
    /// DECDHL bottom half.
    Bottom,
}

impl LineSize {
    fn of(attr: u8) -> Self {
        match attr {
            3 => Self::Top,
            4 => Self::Bottom,
            6 => Self::Wide,
            _ => Self::Single,
        }
    }
}

/// Shared clock for running-block spinners (all spin in phase).
fn block_spin_epoch() -> std::time::Instant {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(std::time::Instant::now)
}

/// A translucent wash over the row at `y`, snapped to whole pixels so
/// neighbouring rows meet without overlapping (an overlap doubles the alpha
/// into a seam line).
fn wash_row(layer: &mut Layer, grid: &GridLayout, y: f32, color: Color) {
    let (top, bottom) = (y.round(), (y + grid.line_h).round());
    layer.rect(
        [grid.x, top],
        [grid.right - grid.x, bottom - top],
        color,
        0.0,
    );
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
    selected: bool,
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

    // left border across the header row: the selected block's 3 px edge
    // runs through its header too
    if selected {
        layer.rect(
            [grid.x, y],
            [3.0, border_h],
            selection_color(theme, block),
            0.0,
        );
    } else {
        layer.rect([grid.x, y], [2.0, border_h], border, 0.0);
    }

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
    // the duration is a right-aligned column at least six cells wide
    // (`1m18s`, `9.99s`), so badges line up from header to header
    let dur_cell = grid.cell_w * (12.0 / grid.font_size);
    let duration_col = duration_text.chars().count().max(6) as f32 * dur_cell;
    let duration_w = if duration_text.is_empty() {
        0.0
    } else {
        duration_col + 11.0
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
    // a filter note keeps its room after the command (the command gives
    // way down to eight cells, then the note is cut too)
    let note_room = filter_note.map_or(0.0, |n| {
        engine.measure(n, 11.5, FONT_SANS, Weight::NORMAL) + 14.0 + 11.0
    });
    let cmd_limit = (left_limit - note_room).max(cmd_x + 8.0 * grid.cell_w);
    let fit = ((cmd_limit.min(left_limit) - cmd_x) / grid.cell_w)
        .floor()
        .max(1.0) as usize;
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
        // never under the badge and duration on the right
        let room = left_limit - after_cmd_x - 14.0;
        let full: Vec<char> = note.chars().collect();
        let mut note = note.to_string();
        let mut tw = engine.measure(&note, 11.5, FONT_SANS, Weight::NORMAL);
        let mut keep = full.len();
        while tw > room && keep > 0 {
            keep -= 1;
            note = full[..keep]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + "…";
            tw = engine.measure(&note, 11.5, FONT_SANS, Weight::NORMAL);
        }
        let note = note.as_str();
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
        let mut nbuf = rift_renderer::glyphon::Buffer::new(
            &mut engine.font_system,
            rift_renderer::glyphon::Metrics::new(11.5, grid.line_h),
        );
        nbuf.set_text(
            &mut engine.font_system,
            &note,
            &rift_renderer::glyphon::Attrs::new()
                .family(rift_renderer::glyphon::Family::Name(mono_family()))
                .style(rift_renderer::glyphon::Style::Italic),
            rift_renderer::glyphon::Shaping::Advanced,
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

    // copy icon, two overlapping rounded-rect outlines: only while the
    // pointer is on the header or the block is selected (its place stays
    // reserved, so nothing shifts)
    let hovered = grid
        .mouse
        .is_some_and(|(mx, my)| mx >= grid.x && mx < right && my >= y && my < y + grid.line_h);
    if hovered || selected {
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
    }

    // duration, right-aligned in its column
    let mut cursor_x = icon_x - 10.0;
    if !duration_text.is_empty() {
        let w = duration_text.chars().count() as f32 * dur_cell;
        let dbuf = make_text(
            engine,
            &duration_text,
            12.0,
            grid.line_h,
            mono_family(),
            Weight::NORMAL,
        );
        layer.text(dbuf, cursor_x - w, y, theme.text_muted);
        out.duration_hits.push(HitRect {
            rect: [cursor_x - w, y, w, grid.line_h],
            block: block.id,
        });
        cursor_x -= duration_col + 11.0;
    }

    // badge pill, right-aligned on the duration column; never taller than
    // the row, so the pills of back-to-back headers do not overlap
    let badge_x = cursor_x - badge_w;
    let pill_h = 17.0f32.min(grid.line_h - 1.0);
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

/// `session`'s running command for a paired phone's Running list: when it
/// started and its latest line; none at the prompt.
pub fn remote_running(session: &Session) -> Option<rift_remote::protocol::PaneRunning> {
    let last = session
        .blocks
        .lock()
        .unwrap()
        .last()
        .map(|b| (b.state.clone(), b.output_start));
    let (started, from) = match last {
        Some((BlockState::Running { started }, from)) => (Some(started), from),
        _ if session.has_foreground_job() => (None, None),
        _ => return None,
    };
    // a block that printed nothing yet has no line (never its prompt or
    // command); without shell integration the screen's last line
    let line = if from.is_some() || started.is_none() {
        session.last_line(from)
    } else {
        String::new()
    };
    let started = started.and_then(|s| {
        std::time::SystemTime::now()
            .checked_sub(s.elapsed())?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs())
    });
    Some(rift_remote::protocol::PaneRunning { started, line })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    #[test]
    fn gaps_are_the_lines_between_a_finished_block_and_the_next_prompt() {
        // contiguous blocks have no gap; Enter pressed during `sleep` left
        // lines 14 and 15 to no block; a running block has no end yet
        let anchors = [
            (Some(1), 0),
            (Some(14), 1),
            (Some(17), 16),
            (None, 17),
            (None, 30),
        ];
        assert_eq!(gaps_between(&anchors), vec![(14, 16)]);
        assert!(gaps_between(&[]).is_empty());
        assert!(gaps_between(&[(Some(3), 0)]).is_empty());
    }

    #[test]
    fn tab_and_hidden_cells_draw_blank() {
        assert_eq!(drawn_char('\t', Flags::empty()), ' ');
        assert_eq!(drawn_char('x', Flags::HIDDEN), ' ');
        assert_eq!(drawn_char('x', Flags::BOLD), 'x');
    }

    #[test]
    fn full_durations_for_the_tooltip() {
        let ms = Duration::from_millis;
        assert_eq!(format_duration_full(ms(412)), "412 ms");
        assert_eq!(format_duration_full(ms(8_214)), "8.214 s");
        assert_eq!(format_duration_full(ms(78_432)), "1 min 18.432 s");
        assert_eq!(format_duration_full(ms(7_384_000)), "2 h 3 min 4 s");
    }

    #[test]
    fn block_elements_fill_their_cell_parts() {
        let parts = block_element;
        assert_eq!(parts('█'), Some((vec![[0.0, 0.0, 1.0, 1.0]], 1.0)));
        assert_eq!(parts('▄'), Some((vec![[0.0, 0.5, 1.0, 1.0]], 1.0)));
        assert_eq!(parts('▁'), Some((vec![[0.0, 0.875, 1.0, 1.0]], 1.0)));
        assert_eq!(parts('▉'), Some((vec![[0.0, 0.0, 0.875, 1.0]], 1.0)));
        assert_eq!(parts('▏'), Some((vec![[0.0, 0.0, 0.125, 1.0]], 1.0)));
        assert_eq!(parts('▐'), Some((vec![[0.5, 0.0, 1.0, 1.0]], 1.0)));
        // Claude Code's logo: ▛ is three quadrants, ▝ one
        assert_eq!(block_element('▛').unwrap().0.len(), 3);
        assert_eq!(parts('▝'), Some((vec![[0.5, 0.0, 1.0, 0.5]], 1.0)));
        assert_eq!(block_element('▒').unwrap().1, 0.5);
        assert!(block_element('a').is_none() && block_element('─').is_none());
    }

    #[test]
    fn variation_selectors_pick_the_presentation() {
        assert_eq!(presentation(None), Presentation::Default);
        assert_eq!(presentation(Some(&['\u{301}'])), Presentation::Default);
        assert_eq!(presentation(Some(&['\u{fe0f}'])), Presentation::Emoji);
        assert_eq!(
            presentation(Some(&['\u{fe0e}'])),
            Presentation::Text(SYMBOL_FAMILY)
        );
    }

    struct Noop;
    impl rift_pty::alacritty_terminal::event::EventListener for Noop {}

    /// A 8x2 grid: row 0 "日本x" + soft wrap flag at the last column, row 1
    /// "é" (e + combining acute) then "y".
    fn sample_term() -> rift_pty::alacritty_terminal::term::Term<Noop> {
        use rift_pty::alacritty_terminal::term::{Config, test::TermSize};
        let mut t = rift_pty::alacritty_terminal::term::Term::new(
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
        assert_eq!(copy_line(t.grid(), 0, 0), Some(("日本x   ".into(), true)));
        assert_eq!(copy_line(t.grid(), 0, 1), Some(("e\u{301}y".into(), false)));
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

    /// A `cols` × `rows` term with `history` lines of scrollback fed `bytes`
    /// through the app's own wrapper (so `scrolled` counts like a pane's).
    fn fed(
        cols: usize,
        rows: usize,
        history: usize,
        bytes: &[u8],
    ) -> (rift_pty::alacritty_terminal::term::Term<Noop>, u64) {
        use rift_pty::alacritty_terminal::term::{Config, test::TermSize};
        let config = Config {
            scrolling_history: history,
            ..Config::default()
        };
        let mut t =
            rift_pty::alacritty_terminal::term::Term::new(config, &TermSize::new(cols, rows), Noop);
        let mut scrolled = 0;
        let mut parser: ansi::Processor = ansi::Processor::new();
        parser.advance(
            &mut rift_pty::abs_term::AbsTerm {
                term: &mut t,
                scrolled: &mut scrolled,
                scrollback_cap: history,
                saw_clear_saved: false,
                on_resize: None,
            },
            bytes,
        );
        (t, scrolled)
    }

    fn hex(c: Color) -> String {
        c.with_alpha(1.0).to_hex()[1..].to_string()
    }

    fn run(
        text: &str,
        fg: Option<String>,
        bg: Option<String>,
        flags: u8,
    ) -> rift_remote::protocol::Run {
        rift_remote::protocol::Run {
            text: text.into(),
            fg,
            bg,
            flags,
        }
    }

    #[test]
    fn remote_screen_is_text_runs_with_resolved_colours() {
        let theme = Theme::default();
        let bytes = concat!(
            "\x1b]4;200;rgb:12/34/56\x07",
            "\x1b[?1049h\x1b[H",
            "\x1b[1;31mred\x1b[0m ok   ",
            "\x1b[2;1H\x1b[7minv\x1b[0m",
            "\x1b[3;1H日x\x1b[48;5;200m \x1b[0m",
            "\x1b[4;1H\x1b[2;3;4;9mz\x1b[0m",
            "\x1b[3;5H",
        );
        let (t, _) = fed(12, 4, 100, bytes.as_bytes());
        let s = screen_of(&t, &theme, false).unwrap();
        assert_eq!(
            (s.cols, s.rows, s.alt, s.cursor),
            (12, 4, true, Some([2, 4]))
        );
        assert_eq!(
            s.lines[0],
            [
                run("red", Some(hex(theme.ansi[1])), None, 1),
                run(" ok", None, None, 0)
            ],
            "trailing default blanks are left out"
        );
        // reverse video: the default colours swapped, so both are set
        assert_eq!(
            s.lines[1],
            [run("inv", Some(hex(theme.bg)), Some(hex(theme.fg)), 0)]
        );
        // a wide glyph is one character (its spacer skipped); OSC 4 applies
        // to a 256-colour slot; a styled blank at the end stays
        assert_eq!(
            s.lines[2],
            [
                run("日x", None, None, 0),
                run(" ", None, Some("123456".into()), 0)
            ]
        );
        assert_eq!(s.lines[3].len(), 1);
        assert_eq!(s.lines[3][0].flags, 2 | 4 | 8 | 16);
        // a hidden cursor is none; the primary screen only when forced
        let (t, _) = fed(12, 4, 100, b"\x1b[?1049hvim\x1b[?25l");
        assert_eq!(screen_of(&t, &theme, false).unwrap().cursor, None);
        let (t, _) = fed(12, 4, 100, b"$ ls\r\n\x1b[2m");
        assert_eq!(screen_of(&t, &theme, false), None);
        let s = screen_of(&t, &theme, true).unwrap();
        assert_eq!((s.alt, s.cursor), (false, Some([1, 0])));
        assert_eq!(s.lines[0], [run("$ ls", None, None, 0)]);
        assert!(s.lines[1..].iter().all(Vec::is_empty));
    }

    #[test]
    fn remote_screen_uses_the_panes_theme_and_crops_big_screens() {
        let theme = Theme::default();
        // the program's own ANSI red (OSC 4) and reverse screen (DECSCNM):
        // default-coloured text stays default (null) on the swapped ground
        let (t, _) = fed(
            12,
            2,
            100,
            b"\x1b]4;1;rgb:aa/bb/cc\x07\x1b[?5h\x1b[?1049h\x1b[31mr\x1b[0m d",
        );
        let s = screen_of(&t, &theme, false).unwrap();
        assert_eq!(
            s.lines[0],
            [
                run("r", Some("aabbcc".into()), None, 0),
                run(" d", None, None, 0)
            ]
        );
        // larger than 300 × 120: the top-left part; a cursor outside it is none
        let (t, _) = fed(310, 130, 100, b"\x1b[?1049h\x1b[126;1Hx");
        let s = screen_of(&t, &theme, false).unwrap();
        assert_eq!((s.cols, s.rows, s.cursor), (300, 120, None));
        assert_eq!(s.lines.len(), 120);
    }

    /// The rows of a done block holding everything fed: from absolute row 0
    /// up to the cursor's (empty) row.
    fn block_rows(
        t: &rift_pty::alacritty_terminal::term::Term<Noop>,
        scrolled: u64,
    ) -> (u64, Option<u64>, bool) {
        // the cursor sits on the empty row after the last line
        (
            0,
            Some(scrolled + t.grid().cursor.point.line.0 as u64),
            false,
        )
    }

    #[test]
    fn output_pages_number_lines_like_the_tails_dropped_count() {
        // short lines; then lines that soft-wrap over two rows each
        for (cols, line) in [
            (
                20,
                Box::new(|i: u64| format!("line {i}")) as Box<dyn Fn(u64) -> String>,
            ),
            (20, Box::new(|i: u64| format!("w{i:04}-{}", "x".repeat(25)))),
        ] {
            let text: String = (0..600).map(|i| format!("{}\r\n", line(i))).collect();
            let (t, scrolled) = fed(cols, 5, 10_000, text.as_bytes());
            let rows = block_rows(&t, scrolled);
            let (tail, dropped) = output_tail(&t, scrolled, rows);
            assert_eq!(dropped, 100);
            assert!(
                tail.starts_with(&format!("{}\n", line(100))),
                "{}",
                &tail[..40]
            );
            // the page at `dropped` starts with the tail's first line
            let page = output_range(&t, scrolled, rows, dropped, 2);
            assert_eq!(page.from, dropped);
            assert_eq!(page.text, format!("{}\n{}\n", line(100), line(101)));
            assert!(tail.starts_with(&page.text));
            assert_eq!(page.total, 600);
            // the first lines, the last, and past the end
            let page = output_range(&t, scrolled, rows, 0, 3);
            assert_eq!(
                page.text,
                format!("{}\n{}\n{}\n", line(0), line(1), line(2))
            );
            let page = output_range(&t, scrolled, rows, 599, 500);
            assert_eq!((page.from, page.text), (599, format!("{}\n", line(599))));
            let page = output_range(&t, scrolled, rows, 600, 10);
            assert_eq!((page.from, page.text.as_str(), page.total), (600, "", 600));
            // at most 500 lines whatever the limit
            let page = output_range(&t, scrolled, rows, 0, 10_000);
            assert_eq!(page.text.lines().count(), 500);
        }
    }

    #[test]
    fn output_pages_skip_rows_gone_from_scrollback() {
        // 50 lines of history: rows before them are gone, one line each
        let text: String = (0..200).map(|i| format!("line {i}\r\n")).collect();
        let (t, scrolled) = fed(20, 5, 50, text.as_bytes());
        let rows = block_rows(&t, scrolled);
        let (tail, dropped) = output_tail(&t, scrolled, rows);
        let first_held = scrolled - 50;
        assert_eq!(dropped, first_held);
        let page = output_range(&t, scrolled, rows, 0, 3);
        assert_eq!(page.from, first_held, "the first line still held");
        assert_eq!(page.total, 200);
        let want: String = (first_held..first_held + 3)
            .map(|i| format!("line {i}\n"))
            .collect();
        assert_eq!(page.text, want);
        assert!(tail.starts_with(&want));
        assert_eq!(output_range(&t, scrolled, rows, dropped, 3).text, want);
    }

    #[test]
    fn output_is_read_from_the_primary_grid_under_the_alternate_screen() {
        // a done block of 40 lines, and a running one through the cursor row
        let text: String = (0..40).map(|i| format!("line {i}\r\n")).collect();
        let (t, scrolled) = fed(20, 5, 1000, text.as_bytes());
        let done = block_rows(&t, scrolled);
        let running = (0, None, true);
        let pages = |t: &rift_pty::alacritty_terminal::term::Term<Noop>| {
            (
                output_range(t, scrolled, done, 0, 50),
                output_tail(t, scrolled, done),
                output_range(t, scrolled, running, 30, 50),
                output_tail(t, scrolled, running),
            )
        };
        let before = pages(&t);
        assert_eq!(before.0.text.lines().count(), 40);
        assert!(before.0.text.starts_with("line 0\nline 1\n"));
        // a full-screen program draws on the alternate screen: the pages
        // and tails are the same primary lines
        let drawn = format!("{text}\x1b[?1049h\x1b[HSECRET-IN-VIM\r\nrow2");
        let (t, alt_scrolled) = fed(20, 5, 1000, drawn.as_bytes());
        assert_eq!(alt_scrolled, scrolled);
        assert!(t.mode().contains(TermMode::ALT_SCREEN));
        let after = pages(&t);
        assert_eq!(after, before);
        assert!(!format!("{after:?}").contains("SECRET"));
    }

    #[test]
    fn rows_gone_from_scrollback_are_counted_not_visited() {
        // a block whose first 500M rows left the scrollback long ago
        let (t, scrolled) = fed(80, 24, 1000, b"a\r\nb\r\n");
        let gone = 500_000_000;
        let start = std::time::Instant::now();
        let page = output_range(&t, scrolled + gone, (0, None, true), 0, 500);
        let (tail, dropped) = output_tail(&t, scrolled + gone, (0, None, true));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        // numbered as before: each gone row one line, then the held ones
        // ("a", "b", the empty cursor row)
        assert_eq!((page.from, page.total), (gone, gone + 3));
        assert_eq!((page.text.as_str(), tail.as_str()), ("a\nb\n", "a\nb\n"));
        assert_eq!(dropped, gone);
    }

    #[test]
    fn remote_screen_cells_are_bounded() {
        let theme = Theme::default();
        // one cell carrying 300k combining marks: 16 of them go out
        let mut z = String::from("\x1b[?1049h\x1b[Ha");
        z.extend(std::iter::repeat_n('\u{0301}', 300_000));
        let (t, _) = fed(80, 24, 100, z.as_bytes());
        let s = screen_of(&t, &theme, false).unwrap();
        let want: String = std::iter::once('a')
            .chain(std::iter::repeat_n('\u{0301}', rift_remote::MAX_CELL_MARKS))
            .collect();
        assert_eq!(s.lines[0], [run(&want, None, None, 0)]);
        // a wide glyph in the last column sent (its second half cut off by
        // the 300-column crop) is a space: no row is wider than `cols`
        let (t, _) = fed(310, 4, 100, "\x1b[?1049h\x1b[1;299Hx日\x1b[31m".as_bytes());
        let s = screen_of(&t, &theme, false).unwrap();
        assert_eq!(s.cols, 300);
        assert_eq!(
            s.lines[0],
            [run(&format!("{}x", " ".repeat(298)), None, None, 0)]
        );
    }

    #[test]
    fn output_pages_are_cut_at_64_kib_and_keep_an_unfinished_last_line_open() {
        // 100 lines of 999 characters: 65 fit in 64 KiB
        let text: String = (0..100)
            .map(|_| format!("{}\r\n", "y".repeat(999)))
            .collect();
        let (t, scrolled) = fed(1000, 5, 10_000, text.as_bytes());
        let page = output_range(&t, scrolled, block_rows(&t, scrolled), 0, 500);
        assert_eq!(page.text.len(), 65 * 1000);
        assert_eq!(page.total, 100);
        // a running block: the cursor row is its unfinished last line
        let (t, scrolled) = fed(20, 5, 100, b"one\r\ntwo\r\npart");
        let page = output_range(&t, scrolled, (0, None, true), 0, 500);
        assert_eq!((page.text.as_str(), page.total), ("one\ntwo\npart", 3));
        let page = output_range(&t, scrolled, (0, None, true), 0, 1);
        assert_eq!(page.text, "one\n", "a line before the last keeps its break");
    }

    #[test]
    fn composed_and_decomposed_text_find_each_other() {
        // "café" written precomposed (U+00E9) at col 0, decomposed at col 5
        let (t, scrolled) = fed(20, 3, 10, "caf\u{e9} cafe\u{301}".as_bytes());
        let row = search_text(&t, scrolled, scrolled).unwrap();
        for q in ["caf\u{e9}", "cafe\u{301}", "CAF\u{c9}"] {
            for re in [false, true] {
                let m = Matcher::new(q, false, re).unwrap().unwrap();
                assert_eq!(m.find_line(&row), vec![(0, 4), (5, 4)], "{q:?} re={re}");
            }
        }
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
    fn a_docked_panel_takes_rows_from_the_viewport() {
        // 20 rows of 10px from y=100 (bottom 300)
        let grid = GridLayout {
            x: 0.0,
            y: 100.0,
            right: 800.0,
            cell_w: 8.0,
            line_h: 10.0,
            font_size: 13.0,
            cols: 100,
            rows: 20,
            ligatures: false,
            cursor_style: CursorStyle::Block,
            editor_owns_cursor: true,
            text_blink_on: true,
            bottom_up: true,
            mouse: None,
        };
        // above a bottom editor at 310: the panel covers 245..310
        let v = beside_panel(grid, 245.0, 310.0);
        assert_eq!((v.y, v.rows), (100.0, 14));
        // below a top editor: the panel covers 60..175, output starts under it
        let v = beside_panel(grid, 60.0, 175.0);
        assert_eq!((v.y, v.rows), (175.0, 12));
        // a panel taller than the pane keeps one row
        assert_eq!(beside_panel(grid, 0.0, 400.0).rows, 1);
    }

    #[test]
    fn soft_clear_pads_viewport() {
        // block on 0..4 (header at 0, output 1..4); `clear` at line 4 with a
        // 6-row viewport → pads push pre-clear content above the fold
        // and the editor hides the live prompt at 4
        let mut blocks = vec![
            block(1, 0, Some(1), Some(4), done(), false),
            block(2, 4, None, None, BlockState::Prompt, false),
        ];
        let d = build_display_list(&blocks, 4, 6, 4, false, true, Some(4));
        assert_eq!(d[0], DisplayLine::Header(0));
        assert_eq!(d[3], DisplayLine::Grid(3));
        assert_eq!(d[4..], [DisplayLine::Pad; 6]);
        // the next command (output 5..8) starts at the top of the viewport
        // (the last 6 display lines): pads fill only the rows below it
        blocks[1] = block(2, 4, Some(5), Some(8), done(), false);
        blocks.push(block(3, 8, None, None, BlockState::Prompt, false));
        let d = build_display_list(&blocks, 4, 6, 4, false, true, Some(4));
        assert_eq!(d.len(), 10); // header + 3 output + header + 3 + 2 pads
        assert_eq!(d[4], DisplayLine::Header(1));
        assert_eq!(d[8..], [DisplayLine::Pad; 2]);
        // beside a docked panel only 3 rows show: no pads push it out of view
        let d = build_display_list_filtered(
            &blocks,
            4,
            6,
            4,
            false,
            true,
            Some(4),
            3,
            None,
            &std::collections::HashSet::new(),
        );
        assert_eq!(d.len(), 8);
        assert_eq!(d[7], DisplayLine::Grid(7));
        // raw prompt (no editor): the screen rows fill the viewport, no pads
        let d = build_display_list(&blocks, 4, 6, 4, false, false, Some(4));
        assert!(!d.contains(&DisplayLine::Pad));
        assert_eq!(d[d.len() - 6], DisplayLine::Header(1));
        // pad older than kept history: ignored
        let d = build_display_list(&blocks, 40, 6, 4, false, false, Some(2));
        assert!(!d.contains(&DisplayLine::Pad));
    }

    #[test]
    fn bottom_up_blocks_sit_above_the_input() {
        let bottom = |blocks: &[Block], pad| {
            build_display_list_filtered(
                blocks,
                4,
                6,
                4,
                false,
                true,
                pad,
                6,
                Some(u64::MAX),
                &std::collections::HashSet::new(),
            )
        };
        // one block (header + 3 lines) in a 6-row view: 2 pads above it
        let mut blocks = vec![
            block(1, 0, Some(1), Some(4), done(), false),
            block(2, 4, None, None, BlockState::Prompt, false),
        ];
        let d = bottom(&blocks, None);
        assert_eq!(d[..2], [DisplayLine::Pad; 2]);
        assert_eq!(d[2], DisplayLine::Header(0));
        assert_eq!(d.len(), 6);
        // after `clear` at 4 the next command sits right above the input
        // too: the pads go between the old content and it
        blocks[1] = block(2, 4, Some(5), Some(8), done(), false);
        blocks.push(block(3, 8, None, None, BlockState::Prompt, false));
        let d = bottom(&blocks, Some(4));
        assert_eq!(d.len(), 10);
        assert_eq!(d[4..6], [DisplayLine::Pad; 2]);
        assert_eq!(d[6], DisplayLine::Header(1));
        assert_eq!(d[9], DisplayLine::Grid(7));
    }

    #[test]
    fn scrollback_history_included() {
        let d = build_display_list(&[], 10, 3, 10, false, false, None);
        assert_eq!(d.len(), 13); // 10 history + 3 screen
        assert_eq!(d[0], DisplayLine::Grid(0));
        assert_eq!(d[12], DisplayLine::Grid(12));
    }
}
