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
    /// Regular and bold text weights (`font_weight`, `bold_font_weight`).
    pub weights: (u16, u16),
    /// `font_features` (see [`intern_features`]).
    pub features: Features,
    /// `minimum_contrast` (1 = off).
    pub min_contrast: f32,
    /// `bold_is_bright`.
    pub bold_is_bright: bool,
    /// The cursor as drawn: `cursor_style`, or hollow in an unfocused pane
    /// (`cursor_hollow_unfocused`).
    pub cursor_style: CursorStyle,
    /// The owned editor draws the cursor; the grid's own cursor stays hidden
    /// (shell-prompt mode keeps the prompt rows visible without it).
    pub editor_owns_cursor: bool,
    /// Blinking text (SGR 5) is in its visible phase.
    pub text_blink_on: bool,
    /// Blocks start right above the input while they do not fill the pane
    /// (`blocks_start = "bottom"` with the input at the bottom).
    pub bottom_up: bool,
    /// Pointer position (block headers show their copy and menu buttons on
    /// hover).
    pub mouse: Option<(f32, f32)>,
    /// The block whose output was just copied: its copy button shows a check.
    pub copied: Option<BlockId>,
    /// Device pixels per logical pixel (cell backgrounds snap to them).
    pub scale: f32,
    /// `block_spacing = "normal"`: a blank row above each block header.
    pub spaced_blocks: bool,
    /// `block_dividers`: a rule between blocks.
    pub block_dividers: bool,
    /// `jump_to_bottom_button`: offer the button over a block running past
    /// the bottom of the pane.
    pub jump_button: bool,
    /// The overlay scrollbar is on (`scrollbar` not `never`): report the
    /// list and its marks ([`RenderOutput::scroll`]).
    pub scrollbar: bool,
}

/// A cell background's rect with its edges on device pixels: rows are a
/// fractional height (15.6 px), and backgrounds of adjacent rows met on a
/// shared, antialiased edge that let the ground show through as a light
/// line (Claude Code's diff rows).
fn snap_rect(pos: [f32; 2], size: [f32; 2], scale: f32) -> ([f32; 2], [f32; 2]) {
    let s = scale.max(1.0);
    let snap = |v: f32| (v * s).round() / s;
    let (x0, y0) = (snap(pos[0]), snap(pos[1]));
    let (x1, y1) = (snap(pos[0] + size[0]), snap(pos[1] + size[1]));
    ([x0, y0], [x1 - x0, y1 - y0])
}

/// Set when a frame drew blinking text: the app keeps repainting to blink it.
pub static BLINK_DRAWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Block content indent: 2px border + 14px padding (design).
pub const BLOCK_INDENT: f32 = 16.0;
/// Vertical gap between consecutive blocks' left borders (design 01).
const BLOCK_GAP: f32 = 2.0;
/// The jump-to-bottom-of-block button's size.
pub const JUMP_BUTTON: f32 = 28.0;

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
    /// A block filter's separator row between runs (no mouse targets).
    Sep,
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
    /// Menu-button (⋮) rects on block headers: the block's context menu.
    pub menu_hits: Vec<HitRect>,
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
    /// The jump-to-bottom button (`jump_to_bottom_button`): its rect and
    /// the block under the pointer that runs past the bottom of the pane.
    pub jump_bottom: Option<([f32; 4], BlockId)>,
    /// The display list for the overlay scrollbar (with `grid.scrollbar`,
    /// off the alternate screen).
    pub scroll: Option<super::scrollbar::ScrollInfo>,
    /// The grid cursor's shape, when it was drawn (dump).
    pub cursor: Option<CursorStyle>,
    /// Cells drawn as geometry instead of glyphs: block elements, box
    /// drawing, Powerline and legacy-computing symbols (dump).
    pub art_cells: usize,
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
    /// A block filter's separator between runs of kept lines (context lines).
    Sep,
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
        &Hidden::default(),
        false,
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

/// What the display list leaves out (see [`filter_hidden`]).
#[derive(Default, Debug)]
pub struct Hidden {
    /// Grid lines skipped.
    pub lines: std::collections::HashSet<u64>,
    /// Kept lines a block filter's separator row goes above.
    pub seps: std::collections::HashSet<u64>,
    /// Each applied filter's block: (block, shown lines, output lines).
    pub counts: Vec<(BlockId, usize, usize)>,
}

/// Lines the display list skips: blank lines that belong to no block (between
/// a finished block's end and the next prompt — the shell's empty prompts
/// for Enter pressed while a command ran), and the output lines each
/// applied block filter drops ([`crate::views::block_filter`]).
pub fn filter_hidden(
    session: &Session,
    filters: Option<&crate::views::block_filter::BlockFilters>,
) -> Hidden {
    let mut hidden = Hidden {
        lines: stray_blank_lines(session),
        ..Hidden::default()
    };
    let Some(filters) = filters else {
        return hidden;
    };
    let ranges: Vec<(&crate::views::block_filter::BlockFilter, (u64, u64))> = {
        let blocks = session.blocks.lock().unwrap();
        filters
            .enabled()
            .filter_map(|f| {
                let range = blocks.iter().find(|b| b.id == f.block)?.output_range()?;
                Some((f, range))
            })
            .collect()
    };
    if ranges.is_empty() {
        return hidden;
    }
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    for (f, (start, end)) in ranges {
        let total = (end - start) as usize;
        let Some(m) = f.matcher() else {
            hidden.counts.push((f.block, total, total));
            continue;
        };
        let matched: Vec<bool> = (start..end)
            .map(|abs| {
                search_text(&term, scrolled, abs).is_some_and(|t| !m.find_line(&t).is_empty())
            })
            .collect();
        let keep = crate::views::block_filter::keep_mask(&matched, f.invert, f.context as usize);
        let mut shown = 0;
        for (i, k) in keep.iter().enumerate() {
            if *k {
                shown += 1;
            } else {
                hidden.lines.insert(start + i as u64);
            }
        }
        if f.context > 0 {
            for i in crate::views::block_filter::separators(&keep) {
                hidden.seps.insert(start + i as u64);
            }
        }
        hidden.counts.push((f.block, shown, total));
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
/// riding up on them. `spaced` (`block_spacing = "normal"`): a blank row
/// above every block header that follows other rows.
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
    hidden: &Hidden,
    spaced: bool,
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
            // a background block: the prompt its output landed after goes,
            // with no header in its place
            if let Some((s, o)) = block.hidden_prompt()
                && abs == s
                && o > s
            {
                abs = o;
                replaced = true;
                break;
            }
            if let Some((s, o)) = block.suppressed_range()
                && abs == s
            {
                if spaced && display.last().is_some_and(|d| *d != DisplayLine::Pad) {
                    display.push(DisplayLine::Pad);
                }
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
            if !hidden.lines.contains(&abs) {
                if hidden.seps.contains(&abs) {
                    display.push(DisplayLine::Sep);
                }
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

/// OpenType features as (tag, value) pairs (`font_features`).
pub type Features = &'static [([u8; 4], u16)];

/// `font_features` as a `'static` slice for [`GridLayout`] (which stays
/// `Copy`): each distinct list is stored once, so a config reload costs one
/// small allocation and a frame none.
pub fn intern_features(list: Vec<([u8; 4], u16)>) -> Features {
    static SEEN: std::sync::Mutex<Vec<Features>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = seen.iter().find(|s| ***s == list[..]) {
        return s;
    }
    let s: Features = Box::leak(list.into_boxed_slice());
    seen.push(s);
    s
}

fn mono_attrs(ligatures: bool, features: &[([u8; 4], u16)]) -> Attrs<'static> {
    let attrs = Attrs::new().family(Family::Name(mono_family()));
    if ligatures && features.is_empty() {
        return attrs;
    }
    use rift_renderer::glyphon::cosmic_text::{FeatureTag, FontFeatures};
    let mut f = FontFeatures::new();
    if !ligatures {
        f.set(FeatureTag::STANDARD_LIGATURES, 0);
        f.set(FeatureTag::CONTEXTUAL_ALTERNATES, 0);
    }
    // the user's list after: `"calt"` turns contextual alternates back on
    for (tag, value) in features {
        f.set(FeatureTag::new(tag), u32::from(*value));
    }
    attrs.font_features(f)
}

/// Bold text's colour with `bold_is_bright`: the eight ANSI colours become
/// their bright variants; anything else stays.
fn bright_for_bold(c: ansi::Color) -> ansi::Color {
    match c {
        ansi::Color::Named(n) if (n as usize) < 8 => ansi::Color::Named(n.to_bright()),
        ansi::Color::Indexed(i) if i < 8 => ansi::Color::Indexed(i + 8),
        c => c,
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

/// A command line in the terminal font with its syntax colors, as the
/// editor shows it (`editor::syntax_color`).
fn make_command_text(
    engine: &mut TextEngine,
    text: &str,
    font_size: f32,
    line_h: f32,
    theme: &Theme,
) -> std::sync::Arc<Buffer> {
    use rift_core::highlight::tokenize;
    use std::hash::{Hash, Hasher};
    let mut spans: Vec<(&str, Color)> = Vec::new();
    let mut pos = 0;
    for (range, kind) in tokenize(text) {
        if range.start > pos {
            spans.push((&text[pos..range.start], theme.fg));
        }
        spans.push((
            &text[range.clone()],
            crate::views::editor::syntax_color(theme, kind),
        ));
        pos = range.end;
    }
    if pos < text.len() {
        spans.push((&text[pos..], theme.fg));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    3u8.hash(&mut h); // cache-key namespace: highlighted command lines
    font_size.to_bits().hash(&mut h);
    line_h.to_bits().hash(&mut h);
    mono_family().hash(&mut h);
    for (s, c) in &spans {
        s.hash(&mut h);
        for v in [c.r, c.g, c.b, c.a] {
            v.to_bits().hash(&mut h);
        }
    }
    engine.shaped(h.finish(), |fs| {
        let mut buf = Buffer::new(fs, Metrics::new(font_size, line_h));
        buf.set_size(fs, None, None);
        let base = Attrs::new().family(Family::Name(mono_family()));
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, c)| (*s, base.clone().color(rift_renderer::text_color(*c))))
            .collect();
        buf.set_rich_text(fs, rich, &base, Shaping::Advanced, None);
        buf
    })
}

/// Active search: the compiled matcher + the current match anchor.
pub struct SearchCtx {
    pub matcher: Matcher,
    pub current: Option<(u64, usize)>,
    /// The rows of the kept matches (the scrollbar's ticks).
    pub lines: Vec<u64>,
}

/// Most memory one find or filter regex may compile to (bytes).
const REGEX_SIZE_LIMIT: usize = 1 << 20;

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
        // a regex source is syntax, never normalized: it runs on the line
        // composed (`SearchLine::composed`), so `[é]` stays one char
        if is_regex {
            // bounded: the regex crate runs in linear time (no
            // backtracking); these cap what one query may compile to
            return regex::RegexBuilder::new(query)
                .case_insensitive(!case)
                .size_limit(REGEX_SIZE_LIMIT)
                .dfa_size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map(|r| Some(Matcher::Regex(r)))
                .map_err(|e| e.to_string());
        }
        // plain text matches canonically: lines are searched decomposed
        // (`SearchLine::push`)
        use unicode_normalization::UnicodeNormalization as _;
        let needle = query
            .nfd()
            .map(|c| if case { c } else { fold(c) })
            .collect();
        Ok(Some(Matcher::Plain { needle, case }))
    }

    /// Matches in a grid line as (grid column, column span). A match that
    /// covers any char of a cell covers the whole cell: a wide glyph spans
    /// two columns, and a combining mark belongs to its base cell.
    pub fn find_line(&self, line: &SearchLine) -> Vec<(usize, usize)> {
        let composed;
        let line = match self {
            Matcher::Plain { .. } => line,
            Matcher::Regex(_) => {
                composed = line.composed();
                &composed
            }
        };
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
pub(crate) fn line_text(
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

    /// The same line canonically composed (NFC) cell by cell, for regex
    /// search: a regex is matched against code points, and terminal text
    /// is mostly composed, so `é` typed in a pattern finds `é` (also when
    /// the program printed `e` + U+0301).
    fn composed(&self) -> SearchLine {
        use unicode_normalization::UnicodeNormalization as _;
        let mut out = SearchLine::default();
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // one cell's chars share a column
            let mut j = i + 1;
            while j < chars.len() && self.cols[j] == self.cols[i] {
                j += 1;
            }
            let cluster: String = chars[i..j].iter().collect();
            for c in cluster.nfc() {
                out.text.push(c);
                out.cols.push(self.cols[i]);
                out.ends.push(self.ends[i]);
            }
            i = j;
        }
        out
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

/// Detected URLs on absolute row `abs`, one soft-wrapped across rows
/// included: (col, len, url) of this row's share of each, with the whole
/// URL. Looks at most `WRAP_SPAN` rows each way.
fn urls_on_row(g: &CellGrid, scrolled: u64, abs: u64) -> Vec<(usize, usize, String)> {
    let cols = g.columns();
    let (text, first) = wrapped_line(g, scrolled, abs);
    let start = (abs - first) as usize * cols;
    urls_in(text.trim_end_matches(' '))
        .into_iter()
        .filter_map(|(col, len, url)| {
            let (s, e) = (col.max(start), (col + len).min(start + cols));
            (s < e).then(|| (s - start, e - s, url))
        })
        .collect()
}

/// Path-like tokens ([`path_tokens`]) that start on absolute row `abs` of
/// `session`, one soft-wrapped onto the rows below included whole: (col,
/// cells on this row, token).
pub fn session_path_tokens(session: &Session, abs: u64) -> Vec<(usize, usize, String)> {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let g = term.grid();
    let cols = g.columns();
    if cols == 0 {
        return Vec::new();
    }
    let (text, first) = wrapped_line(g, scrolled, abs);
    let start = (abs - first) as usize * cols;
    path_tokens(text.trim_end_matches(' '))
        .into_iter()
        .filter(|(col, _, _)| (start..start + cols).contains(col))
        .map(|(col, len, token)| (col - start, len.min(start + cols - col), token))
        .collect()
}

/// The soft-wrapped line around absolute row `abs`, one char per cell (so
/// char `i` is row `first + i / cols`, column `i % cols`), and its first
/// row. Looks at most `WRAP_SPAN` rows each way.
fn wrapped_line(g: &CellGrid, scrolled: u64, abs: u64) -> (String, u64) {
    const WRAP_SPAN: u64 = 8;
    let cols = g.columns();
    // one char per cell, so string positions are columns
    let row_text = |r: u64| -> Option<String> {
        let rel = r as i64 - scrolled as i64;
        if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
            return None;
        }
        let row = &g[Line(rel as i32)];
        Some((0..cols).map(|c| row[Column(c)].c).collect())
    };
    let mut first = abs;
    while first > 0 && abs - first < WRAP_SPAN && row_wraps(g, scrolled, first - 1) == Some(true) {
        first -= 1;
    }
    let mut last = abs;
    while last - abs < WRAP_SPAN && row_wraps(g, scrolled, last) == Some(true) {
        last += 1;
    }
    let mut text = String::new();
    for r in first..=last {
        match row_text(r) {
            Some(t) => text.push_str(&t),
            None if r < abs => text.extend(std::iter::repeat_n(' ', cols)),
            None => break,
        }
    }
    (text, first)
}

/// The smart double-click unit (`smart_select`) under column `col` of
/// absolute row `abs`, a soft-wrapped one whole: its first and last cells
/// as (absolute row, column).
pub fn smart_unit_at(
    session: &Session,
    abs: u64,
    col: usize,
) -> Option<((u64, usize), (u64, usize))> {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let g = term.grid();
    let cols = g.columns();
    if cols == 0 {
        return None;
    }
    let (text, first) = wrapped_line(g, scrolled, abs);
    let r = crate::smart_select::unit_at(&text, (abs - first) as usize * cols + col)?;
    let at = |i: usize| (first + (i / cols) as u64, i % cols);
    Some((at(r.start), at(r.end - 1)))
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
    block_output_snapshot(session, block_id, usize::MAX, usize::MAX).map(|s| s.text)
}

/// A copy of a finished block's output text, at most about `max_bytes`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct OutputSnapshot {
    pub text: String,
    /// Output rows gone from scrollback before the text (not in it).
    pub trimmed_rows: u64,
    /// Every output row, held or not.
    pub total_rows: u64,
    /// Cut at `max_bytes` or `max_lines`: rows after the text are left out.
    pub truncated: bool,
}

/// [`block_output_text`] with its bounds reported: rows gone from
/// scrollback are counted, and reading stops once `max_bytes` is passed or
/// `max_lines` lines are read (the term lock is held only that long).
/// Soft-wrapped rows join into one line; the line break the terminal added
/// after unterminated output is dropped.
pub fn block_output_snapshot(
    session: &Session,
    block_id: BlockId,
    max_bytes: usize,
    max_lines: usize,
) -> Option<OutputSnapshot> {
    snapshot_of(
        &session.blocks,
        &session.scrolled,
        &session.term,
        block_id,
        max_bytes,
        max_lines,
    )
}

/// [`block_output_text`] from a session's parts (a search worker holds
/// them off the UI thread), reading at most about `max_bytes`. Also says
/// how much was there: rows that left the scrollback (or past the bound)
/// make it partial, no row left makes it unavailable. Last: the output
/// rows gone from scrollback before the text (each counted as one line).
pub fn output_text(
    blocks: &std::sync::Mutex<rift_core::BlockStore>,
    scrolled: &std::sync::atomic::AtomicU64,
    term: &rift_pty::alacritty_terminal::sync::FairMutex<
        rift_pty::alacritty_terminal::term::Term<EventProxy>,
    >,
    block_id: BlockId,
    max_bytes: usize,
) -> Option<(String, rift_core::result_search::Output, u64)> {
    use rift_core::result_search::Output;
    let snap = snapshot_of(blocks, scrolled, term, block_id, max_bytes, usize::MAX)?;
    let state = if snap.total_rows > 0 && snap.trimmed_rows >= snap.total_rows {
        Output::Unavailable
    } else if snap.trimmed_rows > 0 || snap.truncated {
        Output::Partial
    } else {
        Output::Full
    };
    Some((snap.text, state, snap.trimmed_rows))
}

/// The one reader behind [`block_output_snapshot`] and [`output_text`].
fn snapshot_of(
    blocks: &std::sync::Mutex<rift_core::BlockStore>,
    scrolled: &std::sync::atomic::AtomicU64,
    term: &rift_pty::alacritty_terminal::sync::FairMutex<
        rift_pty::alacritty_terminal::term::Term<EventProxy>,
    >,
    block_id: BlockId,
    max_bytes: usize,
    max_lines: usize,
) -> Option<OutputSnapshot> {
    let (start, end, unterminated, skip) = {
        let blocks = blocks.lock().unwrap();
        let block = blocks.iter().find(|b| b.id == block_id)?;
        let (start, end) = block.output_range()?;
        // a background block's first row begins after the prompt
        (start, end, block.unterminated, block.skip_cols)
    };
    let scrolled = scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = term.lock();
    // the output's rows are on the primary grid, also under a full-screen
    // program
    let g = term.primary_grid();
    let held = start.max(oldest_row(g, scrolled)).min(end);
    let mut snap = OutputSnapshot {
        trimmed_rows: held - start,
        total_rows: end - start,
        ..OutputSnapshot::default()
    };
    let out = &mut snap.text;
    let mut lines = 0;
    // rows gone from scrollback have no text
    for abs in held..end {
        if out.len() >= max_bytes || lines >= max_lines {
            snap.truncated = true;
            break;
        }
        if let Some((text, wrapped)) = copy_line(g, scrolled, abs) {
            if abs == start && skip > 0 {
                out.extend(text.chars().skip(skip));
            } else {
                out.push_str(&text);
            }
            // a soft-wrapped row continues on the next one: no line break
            if !wrapped {
                out.push('\n');
                lines += 1;
            }
        }
    }
    // the last row's line break was the terminal's, not the output's
    if unterminated && !snap.truncated && snap.text.ends_with('\n') {
        snap.text.pop();
    }
    Some(snap)
}

/// The whole buffer of a pane as plain text (Export buffer): every row
/// scrollback still holds and the screen, soft-wrapped rows joined, the
/// blank rows below the last text left out.
pub fn buffer_text(session: &Session) -> String {
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let g = term.primary_grid();
    let mut out = String::new();
    for abs in oldest_row(g, scrolled)..scrolled + g.screen_lines() as u64 {
        if let Some((text, wrapped)) = copy_line(g, scrolled, abs) {
            out.push_str(&text);
            if !wrapped {
                out.push('\n');
            }
        }
    }
    let kept = out.trim_end_matches(['\n', ' ']).len();
    out.truncate(kept);
    out.push('\n');
    out
}

/// A block's output as styled runs per line, for "Copy as HTML": colours
/// resolve like the renderer's (the pane's theme with the program's
/// palette changes); soft-wrapped rows join into one line. Returns the
/// lines and the output rows gone from scrollback before them.
pub fn block_output_runs(
    session: &Session,
    theme: &Theme,
    block_id: BlockId,
) -> Option<(Vec<Vec<rift_remote::protocol::Run>>, u64)> {
    let (start, end, skip) = {
        let blocks = session.blocks.lock().unwrap();
        let block = blocks.iter().find(|b| b.id == block_id)?;
        let (start, end) = block.output_range()?;
        (start, end, block.skip_cols)
    };
    let scrolled = session.scrolled.load(std::sync::atomic::Ordering::Acquire);
    let term = session.term.lock();
    let pane = term_theme(theme, &term);
    let runs = GridRuns::new(&term, pane.as_ref().unwrap_or(theme));
    let g = term.grid();
    let held = start.max(oldest_row(g, scrolled)).min(end);
    let mut lines: Vec<Vec<rift_remote::protocol::Run>> = Vec::new();
    let mut joined = false;
    for abs in held..end {
        let rel = abs as i64 - scrolled as i64;
        if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
            continue;
        }
        let mut row = runs.row(Line(rel as i32), g.columns());
        if abs == start && skip > 0 {
            // a background block's first row begins after the prompt
            let mut left = skip;
            row.retain_mut(|r| {
                let n = r.text.chars().count();
                if left >= n {
                    left -= n;
                    return false;
                }
                r.text = r.text.chars().skip(left).collect();
                left = 0;
                true
            });
        }
        if joined && let Some(last) = lines.last_mut() {
            last.extend(row);
        } else {
            lines.push(row);
        }
        joined = row_wraps(g, scrolled, abs) == Some(true);
    }
    Some((lines, held - start))
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
    // a running block's end comes from the cursor, which a program that
    // clears its screen (Claude Code answering a folder prompt) can put above
    // the block's start
    let end = output_end(g, scrolled, end).max(start);
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
    // secrets (`mask_secrets`) never leave masked text behind: before the
    // bound, which counts the bullets' bytes
    let out = rift_core::secrets::mask_owned(out);
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
    let text = rift_core::secrets::mask_owned(text);
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

/// The colour of the 2px rule beside a block in state `state` (none for a
/// prompt being typed): accent while running, success / error by exit,
/// muted without one. The desktop's border and the phone's `rules`.
fn state_rule(state: &BlockState, theme: &Theme) -> Option<Color> {
    match state {
        BlockState::Running { .. } => Some(theme.accent),
        BlockState::Done { exit: Some(0), .. } => Some(theme.success),
        BlockState::Done { exit: Some(_), .. } => Some(theme.error),
        BlockState::Done { exit: None, .. } => Some(theme.text_muted),
        BlockState::Prompt => None,
    }
}

/// The rule colour of grid line `abs` (absolute): the block holding it,
/// `live_end` closing the open one; none outside blocks and in raw prompt
/// rows.
fn block_rule_at(blocks: &[Block], theme: &Theme, live_end: u64, abs: u64) -> Option<Color> {
    blocks
        .iter()
        .find(|b| abs >= b.prompt_start && abs < b.end.unwrap_or(live_end))
        .and_then(|b| state_rule(&b.state, theme))
}

/// A colour's 8-bit channels: what the wire's `rrggbb` says, and how runs
/// compare.
fn rgb8(c: Color) -> [u8; 3] {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b)]
}

fn hex8([r, g, b]: [u8; 3]) -> String {
    format!("{r:02x}{g:02x}{b:02x}")
}

/// Rows of a term's grid as the remote gateway's text runs: colours resolve
/// like the renderer's (`theme` is the pane's, with the program's palette
/// changes; reverse video swapped); the theme's default foreground and
/// background are `None`.
struct GridRuns<'a, T> {
    term: &'a rift_pty::alacritty_terminal::term::Term<T>,
    theme: &'a Theme,
    default_fg: [u8; 3],
    default_bg: [u8; 3],
}

impl<'a, T> GridRuns<'a, T> {
    fn new(term: &'a rift_pty::alacritty_terminal::term::Term<T>, theme: &'a Theme) -> Self {
        GridRuns {
            term,
            theme,
            default_fg: rgb8(theme.fg),
            default_bg: rgb8(theme.bg),
        }
    }

    /// 256-color slots the program redefined (OSC 4); the 16 ANSI ones and
    /// the defaults are in `theme` already (as in `render_row`).
    fn palette(&self, c: ansi::Color) -> Color {
        match c {
            ansi::Color::Indexed(i) if i >= 16 => self.term.colors()[i as usize]
                .map(rgb_color)
                .unwrap_or_else(|| resolve_color(self.theme, c)),
            c => resolve_color(self.theme, c),
        }
    }

    /// Grid row `line`, its first `cols` columns.
    fn row(&self, line: Line, cols: usize) -> Vec<rift_remote::protocol::Run> {
        use rift_remote::protocol::Run;
        // a run's style: (fg, bg, flags), `None` for a default colour
        type Style = (Option<[u8; 3]>, Option<[u8; 3]>, u8);
        let row = &self.term.grid()[line];
        // secrets (`mask_secrets`) reach the phone masked, cell for cell
        let mask = rift_core::secrets::active()
            .map(|m| {
                let g = self.term.grid();
                crate::secret_mask::row_cols(
                    g,
                    line,
                    &m,
                    &crate::secret_mask::pem_body(g, line, line),
                )
            })
            .unwrap_or_default();
        let mut runs: Vec<Run> = Vec::new();
        // the last run's style
        let mut style: Option<Style> = None;
        for col in 0..cols {
            let cell = &row[Column(col)];
            let flags = cell.flags;
            if flags.intersects(WIDE_SPACERS) {
                continue;
            }
            let (mut fg, mut bg) = (self.palette(cell.fg), self.palette(cell.bg));
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            // cells compare by their 8-bit colours (what the hex says); the
            // hex text is made once per run
            let (fg, bg) = (rgb8(fg), rgb8(bg));
            let cell_style = (
                (fg != self.default_fg).then_some(fg),
                (bg != self.default_bg).then_some(bg),
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
                    fg: cell_style.0.map(hex8),
                    bg: cell_style.1.map(hex8),
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
            if !flags.contains(Flags::HIDDEN) && mask.iter().any(|(s, e)| col >= *s && col < *e) {
                run.text.push(rift_core::secrets::MASK);
                if flags.contains(Flags::WIDE_CHAR) {
                    run.text.push(rift_core::secrets::MASK);
                }
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
        runs
    }
}

/// The pane's visible screen for the remote gateway: none unless it shows
/// the alternate screen or `force`. The alternate screen is the raw grid; a
/// forced primary screen is Rift's own view of the pane (`styled_screen`):
/// block headers instead of prompt echoes. Colours resolve like the
/// renderer's (the pane's theme with the program's palette changes, reverse
/// video swapped); the theme's default foreground and background are `None`.
/// Returns the screen's generation with it: the session's output generation,
/// which for the styled screen is mixed with what else changes its rows
/// (collapsed blocks, a block's exit, cwd and branch, and `view`, the
/// phone's own scrolled view, which the styled screen follows). While that
/// is still `since`, no screen is built (`None`: the caller's is current).
/// Building one keeps `view` where it was as output arrives (as the
/// desktop's view is kept) and within the list it draws.
pub fn remote_screen(
    session: &Session,
    theme: &Theme,
    force: bool,
    since: Option<u64>,
    view: &mut RemoteView,
) -> (u64, Option<Option<rift_remote::Screen>>) {
    use std::sync::atomic::Ordering;
    let alt = session.term.lock().mode().contains(TermMode::ALT_SCREEN);
    if alt || !force {
        // read under the same term lock as the grid
        let term = session.term.lock();
        let generation = session.generation.load(Ordering::Acquire);
        if since == Some(generation) {
            return (generation, None);
        }
        return (generation, Some(screen_of(&term, theme, force)));
    }
    let generation = session.generation.load(Ordering::Acquire);
    let key = screen_key(session, generation, view.offset);
    if since == Some(key) {
        return (key, None);
    }
    let before = view.offset;
    let screen = {
        // the blocks lock is released before the term lock is taken
        let hidden = filter_hidden(session, None);
        let (blocks, clear_pad) = {
            let store = session.blocks.lock().unwrap();
            (store.snapshot(), store.clear_pad())
        };
        let scrolled = session.scrolled.load(Ordering::Acquire);
        let term = session.term.lock();
        styled_screen(&term, theme, &blocks, clear_pad, scrolled, &hidden, view)
    };
    // the view moved with new output: the screen answers to the new one
    let key = if view.offset == before {
        key
    } else {
        screen_key(session, generation, view.offset)
    };
    (key, Some(screen))
}

/// A capture behind the pane's output is said to be from this long on, and
/// replaced by the live frame past [`CAPTURE_STALE`]: phones are told, never
/// left on an old picture of a program that is working.
const CAPTURE_LATE: std::time::Duration = std::time::Duration::from_secs(3);
const CAPTURE_STALE: std::time::Duration = std::time::Duration::from_secs(10);

/// What a phone is shown of a pane whose window is in use, at the phone's
/// `size`: the pane's capture ([`Session::shadow_capture`]), while it has one
/// and the program is still on the alternate screen; `None`: the live frame.
/// The key differs from every generation [`remote_screen`] answers with, and
/// changes with the capture and the seconds it is behind by.
pub fn captured_screen(
    session: &Session,
    theme: &Theme,
    size: rift_pty::GridSize,
    since: Option<u64>,
) -> Option<(u64, Option<Option<rift_remote::Screen>>)> {
    if !session.term.lock().mode().contains(TermMode::ALT_SCREEN) {
        return None;
    }
    session
        .shadow_frame(size, |term, seq, behind| {
            if behind.is_some_and(|b| b >= CAPTURE_STALE) {
                return None;
            }
            let late = behind.filter(|b| *b >= CAPTURE_LATE).map(|b| b.as_secs());
            let key = (1 << 63) | (seq << 8) | late.unwrap_or(0);
            if since == Some(key) {
                return Some((key, None));
            }
            let screen = screen_of(term, theme, true).map(|mut s| {
                s.age_ms = late.map(|secs| secs * 1000);
                s
            });
            Some((key, Some(screen)))
        })
        .flatten()
}

/// Whether a phone is shown a capture at `size` now ([`captured_screen`]): its
/// cells are not the pane's, so a tap has nowhere to land. Past
/// [`CAPTURE_STALE`] it sees the live frame, whose cells are.
pub fn capture_shown(session: &Session, size: rift_pty::GridSize) -> bool {
    session.term.lock().mode().contains(TermMode::ALT_SCREEN)
        && session.shadow_frame(size, |_, _, behind| {
            !behind.is_some_and(|b| b >= CAPTURE_STALE)
        }) == Some(true)
}

/// A paired phone's own view of a pane's styled screen, apart from the
/// desktop's `view_offset`: display lines scrolled up from the end
/// (`offset`, 0: following the end) and the length of the list it last
/// drew (`len`), which keeps the view in place as output arrives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemoteView {
    pub offset: usize,
    pub len: usize,
}

impl RemoteView {
    /// Settle the view on a display list of `len` lines shown `rows` at a
    /// time: lines added below a scrolled view push it up by as many (it
    /// stays on what it showed), and it never passes the list's top.
    fn settle(&mut self, len: usize, rows: usize) {
        if self.offset > 0 && self.len > 0 && len > self.len {
            self.offset += len - self.len;
        }
        self.offset = self.offset.min(len.saturating_sub(rows));
        self.len = len;
    }

    /// Scroll by `lines` (positive: up), within `max` (the list's length
    /// less the rows shown: [`remote_scroll_max`]).
    pub fn scroll(&mut self, lines: i32, max: usize) {
        let moved = if lines > 0 {
            self.offset.saturating_add(lines as usize)
        } else {
            self.offset.saturating_sub(lines.unsigned_abs() as usize)
        };
        self.offset = moved.min(max);
    }
}

/// How far a phone's view of `session`'s styled screen scrolls up: its
/// display list's length less the rows it shows (0 on the alternate
/// screen, which has no such list). The list [`remote_screen`] draws, not
/// the desktop's (whose editor, filter and sticky headers differ).
pub fn remote_scroll_max(session: &Session) -> usize {
    use std::sync::atomic::Ordering;
    let hidden = filter_hidden(session, None);
    let (blocks, clear_pad) = {
        let store = session.blocks.lock().unwrap();
        (store.snapshot(), store.clear_pad())
    };
    let scrolled = session.scrolled.load(Ordering::Acquire);
    let term = session.term.lock();
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return 0;
    }
    let (display, rows) = styled_display(&term, &blocks, clear_pad, scrolled, &hidden);
    display.len().saturating_sub(rows)
}

/// The display list a styled screen draws and the rows it shows of it.
fn styled_display(
    term: &rift_pty::alacritty_terminal::term::Term<EventProxy>,
    blocks: &[Block],
    clear_pad: Option<u64>,
    scrolled: u64,
    hidden: &Hidden,
) -> (Vec<DisplayLine>, usize) {
    let g = term.grid();
    let rows = g.screen_lines().min(rift_remote::MAX_SCREEN_ROWS as usize);
    let display = build_display_list_filtered(
        blocks,
        scrolled,
        g.screen_lines(),
        g.history_size() as u64,
        false,
        true,
        clear_pad,
        rows,
        Some(live_end(term, scrolled)),
        hidden,
        false,
    );
    (display, rows)
}

/// What a styled screen depends on besides the grid: `generation` (output,
/// resizes) mixed with the blocks' headers (collapse, exit, cwd, branch)
/// and the pane's scrolled view.
fn screen_key(session: &Session, generation: u64, view_offset: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    generation.hash(&mut h);
    view_offset.hash(&mut h);
    let store = session.blocks.lock().unwrap();
    store.clear_pad().hash(&mut h);
    for b in store.iter() {
        (b.id, b.collapsed, b.restored, &b.command, &b.cwd, &b.branch).hash(&mut h);
        match b.state {
            BlockState::Prompt => 0u8.hash(&mut h),
            BlockState::Running { .. } => 1u8.hash(&mut h),
            BlockState::Done { exit, duration } => {
                (2u8, exit, duration.as_millis()).hash(&mut h);
            }
        }
    }
    h.finish()
}

/// [`remote_screen`] of a locked term: the raw grid, only on the alternate
/// screen or when `force`d.
fn screen_of<T>(
    term: &rift_pty::alacritty_terminal::term::Term<T>,
    theme: &Theme,
    force: bool,
) -> Option<rift_remote::Screen> {
    let alt = term.mode().contains(TermMode::ALT_SCREEN);
    if !alt && !force {
        return None;
    }
    let pane = term_theme(theme, term);
    let theme = pane.as_ref().unwrap_or(theme);
    let runs = GridRuns::new(term, theme);
    let g = term.grid();
    let cols = g.columns().min(rift_remote::MAX_SCREEN_COLS as usize);
    let rows = g.screen_lines().min(rift_remote::MAX_SCREEN_ROWS as usize);
    let lines = (0..rows).map(|r| runs.row(Line(r as i32), cols)).collect();
    let cursor = term.grid().cursor.point;
    let shown = term.mode().contains(TermMode::SHOW_CURSOR)
        && (cursor.line.0 as usize) < rows
        && cursor.column.0 < cols;
    Some(rift_remote::Screen {
        cols: cols as u16,
        rows: rows as u16,
        alt,
        cursor: shown.then_some([cursor.line.0 as u16, cursor.column.0 as u16]),
        fg: Some(hex8(rgb8(theme.fg))),
        bg: Some(hex8(rgb8(theme.bg))),
        rules: Vec::new(),
        lines,
        age_ms: None,
    })
}

/// The primary screen as Rift draws it, for a phone: the display list the
/// desktop renders (`build_display_list_filtered`: prompt echoes replaced
/// by a header row, collapsed output skipped, stray blank lines left out,
/// the live prompt hidden as while the editor owns the line, soft-`clear`
/// padding), `rows` lines of it bottom-anchored, `view.offset` lines up
/// from its end where the phone scrolled it (settled on this list first:
/// [`RemoteView::settle`]). The alternate screen stays the raw grid.
fn styled_screen(
    term: &rift_pty::alacritty_terminal::term::Term<EventProxy>,
    theme: &Theme,
    blocks: &[Block],
    clear_pad: Option<u64>,
    scrolled: u64,
    hidden: &Hidden,
    view: &mut RemoteView,
) -> Option<rift_remote::Screen> {
    if term.mode().contains(TermMode::ALT_SCREEN) {
        return screen_of(term, theme, true);
    }
    let pane = term_theme(theme, term);
    let theme = pane.as_ref().unwrap_or(theme);
    let g = term.grid();
    let cols = g.columns().min(rift_remote::MAX_SCREEN_COLS as usize);
    let history = g.history_size() as u64;
    let (display, rows) = styled_display(term, blocks, clear_pad, scrolled, hidden);
    view.settle(display.len(), rows);
    let view_offset = view.offset;
    // the editor owns the cursor at a prompt, which the phone does not show
    let at_prompt = matches!(blocks.last().map(|b| &b.state), Some(BlockState::Prompt));
    let cursor = term.grid().cursor.point;
    let cursor_abs = (term.mode().contains(TermMode::SHOW_CURSOR) && !at_prompt)
        .then(|| scrolled + cursor.line.0.max(0) as u64);
    let runs = GridRuns::new(term, theme);
    let live_end = live_end(term, scrolled);
    let StyledRows {
        lines,
        cursor: cursor_row,
        rules,
    } = styled_rows(
        &display,
        (rows, cols),
        blocks,
        theme,
        |abs| block_rule_at(blocks, theme, live_end, abs),
        |abs| {
            let rel = abs as i64 - scrolled as i64;
            if rel < -(history as i64) || rel >= g.screen_lines() as i64 {
                Vec::new()
            } else {
                runs.row(Line(rel as i32), cols)
            }
        },
        cursor_abs,
        view_offset,
    );
    Some(rift_remote::Screen {
        cols: cols as u16,
        rows: rows as u16,
        alt: false,
        cursor: cursor_row
            .filter(|_| cursor.column.0 < cols)
            .map(|r| [r as u16, cursor.column.0 as u16]),
        fg: Some(hex8(rgb8(theme.fg))),
        bg: Some(hex8(rgb8(theme.bg))),
        rules,
        lines,
        age_ms: None,
    })
}

/// The `rows` lines of `display` that end `view_offset` lines above its end
/// (the pane's scrolled view; 0: its last lines) as screen rows (a header for each
/// block, `grid_row` for a grid line, blank for padding), and the row of
/// `cursor_abs` among them. Pure: the part of [`styled_screen`] that does
/// not read a term. Also the rows that carry a block rule and its colour
/// (`"rrggbb"`): a header's by its block's state, a grid line's by
/// `grid_rule`.
#[allow(clippy::too_many_arguments)]
fn styled_rows(
    display: &[DisplayLine],
    (rows, cols): (usize, usize),
    blocks: &[Block],
    theme: &Theme,
    grid_rule: impl Fn(u64) -> Option<Color>,
    mut grid_row: impl FnMut(u64) -> Vec<rift_remote::protocol::Run>,
    cursor_abs: Option<u64>,
    view_offset: usize,
) -> StyledRows {
    // the pane's view: `view_offset` lines above the end, as the desktop
    // clamps it (never above the first line)
    let end = display.len() - view_offset.min(display.len().saturating_sub(rows));
    let start = end.saturating_sub(rows);
    // fewer lines than rows: blank ones above
    let mut lines = vec![Vec::new(); rows - (end - start)];
    let lead = lines.len();
    let mut cursor = None;
    let mut rules = Vec::new();
    for (i, dl) in display[start..end].iter().enumerate() {
        let rule = match dl {
            DisplayLine::Grid(abs) => grid_rule(*abs),
            DisplayLine::Header(bi) => state_rule(&blocks[*bi].state, theme),
            DisplayLine::Pad | DisplayLine::Sep => None,
        };
        if let Some(c) = rule {
            rules.push(((lead + i) as u16, hex8(rgb8(c))));
        }
        lines.push(match dl {
            DisplayLine::Grid(abs) => {
                if cursor_abs == Some(*abs) {
                    cursor = Some(lead + i);
                }
                grid_row(*abs)
            }
            DisplayLine::Header(bi) => header_runs(&blocks[*bi], theme, cols),
            DisplayLine::Pad | DisplayLine::Sep => Vec::new(),
        });
    }
    StyledRows {
        lines,
        cursor,
        rules,
    }
}

/// [`styled_rows`]'s answer: the screen rows, the row the cursor is on, and
/// the rows carrying a block rule with its colour.
struct StyledRows {
    lines: Vec<Vec<rift_remote::protocol::Run>>,
    cursor: Option<usize>,
    rules: Vec<(u16, String)>,
}

/// A block's header as one `cols`-wide row, as the desktop draws it
/// (`render_header`): chevron, the command (bold, bright), `cwd git:(branch)`
/// or the hidden-lines note of a collapsed block, and at the right the state
/// badge (`running`, `✓ 0`, `✕ n`) with the duration of a finished block. The command gives way
/// first (cut with `…`); the context is left out when it does not fit whole.
fn header_runs(block: &Block, theme: &Theme, cols: usize) -> Vec<rift_remote::protocol::Run> {
    use rift_remote::protocol::Run;
    let (badge_color, badge) = match block.state {
        BlockState::Running { .. } => (theme.warning, "running".to_string()),
        BlockState::Done { exit: Some(0), .. } => (theme.success, "✓ 0".to_string()),
        BlockState::Done { exit: Some(n), .. } => (theme.error, format!("✕ {n}")),
        BlockState::Done { exit: None, .. } => (theme.text_muted, "—".to_string()),
        BlockState::Prompt => return Vec::new(),
    };
    let duration = match block.state {
        BlockState::Done { duration, .. } if block.restored => {
            format!("restored · {}", format_duration(duration))
        }
        BlockState::Done { duration, .. } => format_duration(duration),
        // no clock: the row would change every second the command runs
        BlockState::Running { .. } | BlockState::Prompt => String::new(),
    };
    let right = if duration.is_empty() {
        badge.clone()
    } else {
        format!("{badge} {duration}")
    };
    let n = |s: &str| s.chars().count();
    // the badge needs room beside at least a chevron and a few command
    // characters; on a very narrow screen the command alone stays
    let right_w = if cols >= n(&right) + 8 { n(&right) } else { 0 };
    let left_w = cols.saturating_sub(right_w + usize::from(right_w > 0));
    let chevron = if block.collapsed { "▸ " } else { "▾ " };
    let mut cmd = if block.command.is_empty() {
        "—".to_string()
    } else {
        rift_core::secrets::mask(&block.command).replace('\n', " ⏎ ")
    };
    let room = left_w.saturating_sub(n(chevron)).max(1);
    if n(&cmd) > room {
        cmd = cmd.chars().take(room - 1).collect::<String>() + "…";
    }
    let context = if block.collapsed
        && let Some((s, e)) = block.output_range()
    {
        let lines = e - s;
        Some((
            format!("{lines} line{} hidden", if lines == 1 { "" } else { "s" }),
            theme.text_muted,
        ))
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
        Some((ctx, theme.text_dim))
    } else {
        None
    };
    let style = |text: String, color: Color, flags: u8| Run {
        text,
        fg: Some(hex8(rgb8(color))),
        bg: None,
        flags,
    };
    // the desktop's pill: the badge colour at 13 % over the ground
    let pill = theme.bg.mix(badge_color, 0.13);
    let mut runs = vec![
        style(chevron.into(), theme.text_muted, 0),
        style(cmd.clone(), theme.text_bright, Run::BOLD),
    ];
    let mut used = n(chevron) + n(&cmd);
    if let Some((ctx, color)) = context
        && used + 2 + n(&ctx) <= left_w
    {
        used += 2 + n(&ctx);
        runs.push(style(format!("  {ctx}"), color, 0));
    }
    if right_w > 0 {
        let pad = cols.saturating_sub(used + right_w);
        runs.push(Run {
            text: " ".repeat(pad),
            fg: None,
            bg: None,
            flags: 0,
        });
        runs.push(Run {
            bg: Some(hex8(rgb8(pill))),
            ..style(badge.clone(), badge_color, 0)
        });
        if !duration.is_empty() {
            runs.push(style(format!(" {duration}"), theme.text_muted, 0));
        }
    }
    runs
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
                command: rift_core::secrets::mask_owned(b.command),
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
    // the selected blocks (one, or a multi-block selection)
    selected: &[BlockId],
    filter: Option<&crate::views::block_filter::BlockFilters>,
    secrets: Option<(
        &rift_core::secrets::Masker,
        &mut crate::secret_mask::PaneSecrets,
    )>,
) -> RenderOutput {
    use std::sync::atomic::Ordering;
    session.dirty.store(false, Ordering::Release);

    let hidden = filter_hidden(session, filter);
    let (blocks, clear_pad) = {
        let store = session.blocks.lock().unwrap();
        (store.snapshot(), store.clear_pad())
    };
    // header notes of the filtered blocks: `filter "q" · shown/total`
    let filter_notes: Vec<(BlockId, String)> = hidden
        .counts
        .iter()
        .filter_map(|(id, shown, total)| {
            let f = filter?.get(*id)?;
            Some((*id, f.note(*shown, *total)))
        })
        .collect();
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
        grid.spaced_blocks,
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

    // the overlay scrollbar's list and marks
    if grid.scrollbar && !alt_screen {
        use super::scrollbar::TickKind;
        let mut ticks = Vec::new();
        for (i, dl) in display.iter().enumerate() {
            if let DisplayLine::Header(bi) = dl {
                let b = &blocks[*bi];
                let failed = matches!(b.state, BlockState::Done { exit: Some(e), .. } if e != 0);
                ticks.push((
                    i,
                    if failed {
                        TickKind::Failed
                    } else {
                        TickKind::Prompt
                    },
                ));
                if b.bookmarked {
                    ticks.push((i, TickKind::Bookmark));
                }
            }
        }
        if let Some(ctx) = search.filter(|c| !c.lines.is_empty()) {
            let shown: Vec<(u64, usize)> = display
                .iter()
                .enumerate()
                .filter_map(|(i, d)| match d {
                    DisplayLine::Grid(abs) => Some((*abs, i)),
                    _ => None,
                })
                .collect();
            for abs in &ctx.lines {
                let at = shown.partition_point(|(a, _)| a < abs);
                if let Some((a, i)) = shown.get(at)
                    && a == abs
                {
                    ticks.push((*i, TickKind::Match));
                }
            }
        }
        out.scroll = Some(super::scrollbar::ScrollInfo {
            len: display.len(),
            rows,
            start,
            ticks,
        });
    }

    // secrets on the rows shown (`mask_secrets`)
    let mut masks = secrets.map(|(masker, state)| {
        let (lo, hi) = visible
            .iter()
            .filter_map(|d| match d {
                DisplayLine::Grid(abs) => Some(*abs),
                _ => None,
            })
            .fold((u64::MAX, 0), |(lo, hi), a| (lo.min(a), hi.max(a)));
        crate::secret_mask::FrameMasks::new(
            masker,
            state,
            g,
            scrolled,
            (lo.min(hi), hi),
            std::time::Instant::now(),
        )
    });

    let cursor = term.renderable_content().cursor;
    let show_cursor = term.mode().contains(TermMode::SHOW_CURSOR) && !grid.editor_owns_cursor;
    let cursor_abs = scrolled + cursor.point.line.0.max(0) as u64;
    let sel_range = term.selection.as_ref().and_then(|s| s.to_range(&term));
    // an open (running) block ends at its last used row, the cursor row or
    // the last non-blank line, not at the bottom of the blank screen tail
    let live_end = live_end(&term, scrolled);

    // selected blocks: abs row range and color for the highlight treatment;
    // a selected failed block keeps its error color, any other takes the accent
    let selected_ranges: Vec<(u64, u64, Color)> = blocks
        .iter()
        .filter(|b| selected.contains(&b.id))
        .map(|b| {
            (
                b.prompt_start,
                b.end.unwrap_or(live_end),
                selection_color(theme, b),
            )
        })
        .collect();
    let sel_at = |abs: u64| {
        selected_ranges
            .iter()
            .find(|(s, e, _)| abs >= *s && abs < *e)
            .map(|r| r.2)
    };
    let row_selected = |abs: u64| sel_at(abs).is_some();
    let header_sel_color = |b: &Block| selection_color(theme, b);

    // only selected blocks get a wash; exit state lives on the left
    // border and badge alone (design 01: unselected failed blocks are not tinted)
    let row_bg = |abs: u64| -> Option<Color> { sel_at(abs).map(|c| c.with_alpha(0.08)) };

    // block membership for border drawing: abs line → block state color
    let border_color = |abs: u64| block_rule_at(&blocks, theme, live_end, abs);

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
            DisplayLine::Pad | DisplayLine::Sep => None,
        }
    };
    let border_h = |r: usize| -> f32 {
        let this = owner(&visible[r]);
        // a filter's separator row continues the block below it
        let after = if visible.get(r + 1) == Some(&DisplayLine::Sep) {
            r + 2
        } else {
            r + 1
        };
        let next = visible.get(after).and_then(owner);
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
                // a background block's first row: its output follows the
                // prompt it landed after, which stays out; the output starts
                // at the indent like every other row
                let skip = if alt_screen {
                    0
                } else {
                    blocks
                        .iter()
                        .find(|b| b.background && b.output_start == Some(*abs))
                        .map_or(0, |b| b.skip_cols)
                };
                let content_x = if alt_screen {
                    grid.x
                } else if let Some(color) = border_color(*abs) {
                    if is_sel {
                        let c = sel_at(*abs).unwrap_or(theme.accent);
                        layer.rect([grid.x, y], [3.0, border_h(r)], c, 0.0);
                    } else {
                        layer.rect([grid.x, y], [2.0, border_h(r)], color, 0.0);
                    }
                    grid.x + BLOCK_INDENT
                } else {
                    grid.x
                } - skip as f32 * grid.cell_w;
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Grid {
                        abs: *abs,
                        content_x,
                    },
                });
                // links, as in Warp: detected URLs and OSC 8 hyperlinks (ls
                // --hyperlink, gcc, delta) draw in the link color, underlined
                // under the pointer, on the alternate screen too (Claude Code
                // and Codex run there)
                let mut links: Vec<LinkSpan> = Vec::new();
                for (col, len, uri) in urls_on_row(term.grid(), scrolled, *abs)
                    .into_iter()
                    .chain(hyperlinks_in(&term, scrolled, *abs))
                {
                    let rect = [
                        content_x + col as f32 * grid.cell_w,
                        y,
                        len as f32 * grid.cell_w,
                        grid.line_h,
                    ];
                    let hovered = grid.mouse.is_some_and(|(mx, my)| {
                        mx >= rect[0] && mx < rect[0] + rect[2] && my >= y && my < y + grid.line_h
                    });
                    links.push(LinkSpan { col, len, hovered });
                    out.url_hits.push((rect, uri));
                }
                let masked = masks.as_mut().map(|f| f.row(g, *abs)).unwrap_or_default();
                if let Some(f) = masks.as_mut() {
                    for s in &masked {
                        let rect = [
                            content_x + s.start as f32 * grid.cell_w,
                            y,
                            (s.end - s.start) as f32 * grid.cell_w,
                            grid.line_h,
                        ];
                        f.drawn(rect, s.key);
                    }
                }
                let cursor_here = show_cursor && *abs == cursor_abs;
                if cursor_here {
                    out.cursor = Some(grid.cursor_style);
                }
                out.art_cells += render_grid_line(
                    layer,
                    engine,
                    theme,
                    &term,
                    grid,
                    *abs,
                    scrolled,
                    content_x,
                    y,
                    cursor_here,
                    cursor.point.column.0,
                    search,
                    sel_range,
                    &links,
                    skip,
                    &masked,
                );
            }
            DisplayLine::Pad => {
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Pad,
                });
            }
            DisplayLine::Sep => {
                // the block's border runs on; a dashed rule marks the gap
                // between two runs of kept lines
                let next = visible[r + 1..].iter().find_map(|d| match d {
                    DisplayLine::Grid(abs) => Some(*abs),
                    _ => None,
                });
                if let Some(abs) = next {
                    if let Some(bg) = row_bg(abs) {
                        wash_row(layer, grid, y, bg);
                    }
                    if let Some(color) = border_color(abs) {
                        let (w, c) = match sel_at(abs) {
                            Some(c) => (3.0, c),
                            None => (2.0, color),
                        };
                        layer.rect([grid.x, y], [w, grid.line_h + 0.5], c, 0.0);
                    }
                }
                let mid = (y + grid.line_h / 2.0).round();
                let mut x = grid.x + BLOCK_INDENT;
                while x + 4.0 <= grid.right {
                    layer.rect([x, mid], [4.0, 1.0], theme.text_muted, 0.0);
                    x += 8.0;
                }
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Sep,
                });
            }
            DisplayLine::Header(bi) => {
                let block = &blocks[*bi];
                let sel = selected.contains(&block.id);
                if sel {
                    wash_row(layer, grid, y, header_sel_color(block).with_alpha(0.08));
                }
                // block dividers: a rule above every header but the top
                // row's, centred in the blank row of `block_spacing = "normal"`
                if grid.block_dividers && r > 0 {
                    let ry = if visible[r - 1] == DisplayLine::Pad {
                        y - grid.line_h / 2.0
                    } else {
                        y - BLOCK_GAP / 2.0
                    };
                    layer.hline(grid.x, ry.round(), grid.right - grid.x, 1.0, theme.border);
                }
                out.rows.push(RowInfo {
                    y,
                    kind: RowKind::Header(block.id),
                });
                let note = filter_notes
                    .iter()
                    .find(|(id, _)| *id == block.id)
                    .map(|(_, n)| n.as_str());
                let bh = border_h(r);
                render_header(
                    layer, engine, theme, grid, block, y, bh, sel, note, &mut out,
                );
            }
        }
    }

    // jump to the bottom of a block (Warp): the pointer is over a block that
    // runs past the bottom of the pane and shows more than 70 px of it
    if grid.jump_button
        && !alt_screen
        && *view_offset > 0
        && let Some((mx, my)) = grid.mouse
        && mx >= grid.x
        && mx < grid.right
        && my >= grid.y
        && let Some(id) = visible
            .get(((my - grid.y) / grid.line_h) as usize)
            .and_then(owner)
        && visible.last().and_then(owner) == Some(id)
        && display.get(start + rows).and_then(owner) == Some(id)
        && visible.iter().filter(|d| owner(d) == Some(id)).count() as f32 * grid.line_h > 70.0
    {
        let size = JUMP_BUTTON;
        let bottom = grid.y + rows as f32 * grid.line_h;
        out.jump_bottom = Some((
            [grid.right - 10.0 - size, bottom - 10.0 - size, size, size],
            id,
        ));
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

/// The pinned header's click areas: its buttons, which act as on the
/// block's own header, and the rest of it (jump to block).
pub struct StickyHits {
    pub rect: [f32; 4],
    pub chevron: Vec<HitRect>,
    pub copy: Vec<HitRect>,
    pub menu: Vec<HitRect>,
}

/// Draw the pinned header for `block` at the viewport top (layer 2 — an
/// opaque bg on layer 1 could not cover layer-1 row text).
pub fn render_sticky_header(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    grid: &GridLayout,
    session: &Session,
    block_id: BlockId,
    selected: bool,
) -> Option<StickyHits> {
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
    Some(StickyHits {
        rect,
        chevron: sink.chevron_hits,
        copy: sink.copy_hits,
        menu: sink.menu_hits,
    })
}

/// Edge and wash color of a selected block: a failed command stays red,
/// anything else takes the accent.
fn selection_color(theme: &Theme, block: &Block) -> Color {
    match block.state {
        BlockState::Done { exit: Some(n), .. } if n != 0 => theme.error,
        _ => theme.accent,
    }
}

/// A link on one grid line: columns `col..col + len`, and whether the
/// pointer rests on it.
struct LinkSpan {
    col: usize,
    len: usize,
    hovered: bool,
}

/// A link's text color: the accent, lighter under the pointer (Warp).
fn link_color(theme: &Theme, hovered: bool) -> Color {
    if hovered {
        theme.accent.mix(theme.text_bright, 0.35)
    } else {
        theme.accent
    }
}

/// Draws one grid line; returns how many of its cells were drawn as
/// geometry ([`RenderOutput::art_cells`]).
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
    links: &[LinkSpan],
    from_col: usize,
    mask: &[crate::secret_mask::RowSpan],
) -> usize {
    let g = term.grid();
    let rel = abs as i64 - scrolled as i64;
    if rel < -(g.history_size() as i64) || rel >= g.screen_lines() as i64 {
        return 0;
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
    // the line's text in runs, each placed at its first column: a glyph the
    // terminal font lacks comes from a fallback font whose advance is not a
    // cell (⧉ is wider), so it gets a run of its own and the text after it
    // starts on its column again (selection and links follow the grid)
    let mut segs: Vec<(usize, Vec<Span>)> = vec![(from_col, Vec::new())];
    let mut break_next = false;
    let cols = (g.columns() / sx as usize).min(grid.cols + 8);
    // cell backgrounds (merged runs, block cursor) draw before decorations
    let mut backs: Vec<([f32; 2], [f32; 2], Color, f32)> = Vec::new();
    let mut decos: Vec<([f32; 2], [f32; 2], Color)> = Vec::new();
    let mut blocks: Vec<([f32; 2], [f32; 2], Color)> = Vec::new();
    // box drawing's diagonals, arcs and Powerline's thin separators
    let mut strokes: Vec<([f32; 2], [f32; 2], f32, Color)> = Vec::new();
    let mut art_cells = 0;

    for col in from_col.min(cols)..cols {
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
        let cell_fg = if grid.bold_is_bright && flags.contains(Flags::BOLD) {
            bright_for_bold(cell.fg)
        } else {
            cell.fg
        };
        let mut fg = palette(cell_fg);
        let mut bg = palette(cell.bg);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = fg.with_alpha(0.66);
        }
        let link = links.iter().find(|l| col >= l.col && col < l.col + l.len);
        if let Some(l) = link
            && !flags.contains(Flags::INVERSE)
        {
            fg = link_color(theme, l.hovered);
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
            let (lh, c) = (grid.line_h, theme.cursor);
            match grid.cursor_style {
                CursorStyle::Block => {}
                CursorStyle::Bar => decos.push(([cx - 1.0, y], [2.0, lh], c)),
                CursorStyle::Underline => decos.push(([cx, y + lh - 2.0], [cell_px, 2.0], c)),
                CursorStyle::DoubleUnderline => {
                    decos.push(([cx, y + lh - 1.0], [cell_px, 1.0], c));
                    decos.push(([cx, y + lh - 3.0], [cell_px, 1.0], c));
                }
                // an outline; the glyph keeps its own colour
                CursorStyle::Hollow => {
                    decos.push(([cx, y], [cell_px, 1.0], c));
                    decos.push(([cx, y + lh - 1.0], [cell_px, 1.0], c));
                    decos.push(([cx, y + 1.0], [1.0, lh - 2.0], c));
                    decos.push(([cx + cell_px - 1.0, y + 1.0], [1.0, lh - 2.0], c));
                }
            }
        }

        let weight = if flags.contains(Flags::BOLD) {
            Weight(grid.weights.1)
        } else {
            Weight(grid.weights.0)
        };
        let italic = flags.contains(Flags::ITALIC);
        let mut ch = drawn_char(cell.c, flags);
        // a secret (`mask_secrets`): one bullet per cell, nothing reflows
        let masked =
            !flags.contains(Flags::HIDDEN) && mask.iter().any(|s| col >= s.start && col < s.end);
        if masked {
            ch = rift_core::secrets::MASK;
        }
        // drawn geometry takes the glyph's colour (the ground on a block
        // cursor)
        let art = if block_cursor { theme.bg } else { fg };
        // block elements fill the whole cell (line height included), as
        // in Warp: the font's glyphs stop short of the line box and left
        // gaps in block art (Claude Code's logo)
        if attr == LineSize::Single
            && !flags.contains(Flags::HIDDEN)
            && let Some((parts, alpha)) = block_element(ch)
        {
            let ink = art.with_alpha(art.a * alpha);
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
            art_cells += 1;
        } else if attr == LineSize::Single
            && !flags.contains(Flags::HIDDEN)
            && let Some(shapes) = super::box_drawing::shapes(
                ch,
                [cx, y, cell_px, grid.line_h],
                grid.font_size,
                grid.scale,
                art.a >= 1.0,
            )
        {
            // box drawing and Powerline: lines on device pixels from edge
            // to edge of the line box, so frames and segments join up
            for shape in shapes {
                match shape {
                    super::box_drawing::Shape::Rect(pos, size) => blocks.push((pos, size, art)),
                    super::box_drawing::Shape::Line(a, b, w) => strokes.push((a, b, w, art)),
                }
            }
            ch = ' ';
            art_cells += 1;
        }
        // text keeps `minimum_contrast` against its cell (geometry above
        // keeps its colours: Powerline segments pair fg and bg on purpose)
        let fg = if block_cursor {
            theme.bg
        } else if grid.min_contrast > 1.0 && ch != ' ' {
            fg.with_min_contrast(bg, grid.min_contrast)
        } else {
            fg
        };
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
            } else if flags.contains(Flags::UNDERLINE) || link.is_some_and(|l| l.hovered) {
                decos.push(([cx, base], [cell_px, 1.0], deco));
            }
            if flags.contains(Flags::STRIKEOUT) {
                decos.push(([cx, y + grid.line_h / 2.0], [cell_px, 1.0], fg));
            }
        }

        let off_grid = !ch.is_ascii() && !engine.family_covers(mono_family(), ch);
        if off_grid || break_next {
            match segs.last_mut() {
                Some((start, s)) if s.is_empty() => *start = col,
                _ => segs.push((col, Vec::new())),
            }
        }
        // a two-cell character the font draws one cell wide (an East Asian
        // Ambiguous one with `ambiguous_width = "wide"`) would pull the
        // text after it a cell to the left: that text starts on its column
        break_next = off_grid || flags.contains(Flags::WIDE_CHAR);
        let Some((_, spans)) = segs.last_mut() else {
            continue;
        };
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
        if masked && flags.contains(Flags::WIDE_CHAR) {
            span.text.push(rift_core::secrets::MASK);
        }
        // combining marks shape onto their base char (zero advance)
        if !flags.contains(Flags::HIDDEN)
            && !masked
            && let Some(zw) = cell.zerowidth()
        {
            span.text.extend(zw);
        }
    }
    for (pos, size, color, radius) in backs {
        // the block cursor (rounded) keeps its exact cell
        let (pos, size) = if radius == 0.0 {
            snap_rect(pos, size, grid.scale)
        } else {
            (pos, size)
        };
        layer.rect(pos, size, color, radius);
    }
    for (pos, size, color) in blocks.into_iter().chain(decos) {
        layer.rect(pos, size, color, 0.0);
    }
    for (a, b, width, color) in strokes {
        layer.line(a, b, width, color);
    }

    let line_x = x;
    for (start, mut spans) in segs {
        if spans.iter().all(|s| s.text.trim().is_empty()) {
            continue;
        }
        let x = line_x + start as f32 * grid.cell_w * sx;
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
        grid.features.hash(&mut h);
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
                    let attrs = mono_attrs(grid.ligatures, grid.features)
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
                &mono_attrs(grid.ligatures, grid.features),
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
                    left: line_x.floor() as i32,
                    top: y.floor() as i32,
                    right: (line_x + cols as f32 * grid.cell_w * sx).ceil() as i32,
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
    art_cells
}

/// Block element `c` (U+2580–U+259F, and the legacy-computing sextants and
/// eighth blocks U+1FB00–U+1FB3B, U+1FB70–U+1FB8B) as filled parts of the
/// cell, each `[x0, y0, x1, y1]` in cell fractions, and the ink's opacity
/// (the shades ░▒▓ are the full cell at 25/50/75 %). None for any other
/// character.
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
        // sextants: a 2×3 grid, bit 0 the top left, row by row; the codes
        // skip the patterns ▌ ▐ █ (and blank) already encoded above
        n @ 0x1FB00..=0x1FB3B => {
            let mut bits = n - 0x1FB00 + 1;
            if bits >= 21 {
                bits += 1;
            }
            if bits >= 42 {
                bits += 1;
            }
            (0..6)
                .filter(|i| bits & (1 << i) != 0)
                .map(|i| {
                    let (x, y) = ((i % 2) as f32 / 2.0, (i / 2) as f32 / 3.0);
                    [x, y, x + 0.5, y + 1.0 / 3.0]
                })
                .collect()
        }
        // one-eighth columns 2–7, then rows 2–7
        n @ 0x1FB70..=0x1FB75 => {
            let x = eighth(n - 0x1FB70 + 1);
            vec![[x, 0.0, x + eighth(1), 1.0]]
        }
        n @ 0x1FB76..=0x1FB7B => {
            let y = eighth(n - 0x1FB76 + 1);
            vec![[0.0, y, 1.0, y + eighth(1)]]
        }
        // one-eighth frames: left and lower, left and upper, right and
        // upper, right and lower, upper and lower; rows 1, 3, 5 and 8
        0x1FB7C => vec![[0.0, 0.0, eighth(1), 1.0], [0.0, eighth(7), 1.0, 1.0]],
        0x1FB7D => vec![[0.0, 0.0, eighth(1), 1.0], [0.0, 0.0, 1.0, eighth(1)]],
        0x1FB7E => vec![[eighth(7), 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, eighth(1)]],
        0x1FB7F => vec![[eighth(7), 0.0, 1.0, 1.0], [0.0, eighth(7), 1.0, 1.0]],
        0x1FB80 => vec![[0.0, 0.0, 1.0, eighth(1)], [0.0, eighth(7), 1.0, 1.0]],
        0x1FB81 => [0, 2, 4, 7]
            .map(|r| [0.0, eighth(r), 1.0, eighth(r + 1)])
            .to_vec(),
        // upper 2, 3, 5, 6, 7 eighths; right 2, 3, 5, 6, 7 eighths
        n @ 0x1FB82..=0x1FB86 => {
            vec![[
                0.0,
                0.0,
                1.0,
                eighth([2, 3, 5, 6, 7][(n - 0x1FB82) as usize]),
            ]]
        }
        n @ 0x1FB87..=0x1FB8B => {
            vec![[
                1.0 - eighth([2, 3, 5, 6, 7][(n - 0x1FB87) as usize]),
                0.0,
                1.0,
                1.0,
            ]]
        }
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

/// `s` fitted to `max` cells as the terminal counts them (a CJK char takes
/// two, a combining mark none, a ZWJ emoji two): whole clusters only, with
/// `…` when cut. Returns the text and its width in cells.
fn fit_cells(s: &str, max: usize) -> (String, usize) {
    use rift_pty::alacritty_terminal::term::width::{Cluster, Placement};
    let mut cluster = Cluster::default();
    // (byte offset where a cluster starts, cells before it)
    let mut starts = Vec::new();
    let mut used = 0usize;
    for (i, c) in s.char_indices() {
        match cluster.next(c) {
            Some(Placement::New(w)) => {
                starts.push((i, used));
                used += w;
            }
            Some(Placement::JoinWiden) => used += 1,
            Some(Placement::JoinNarrow) => used = used.saturating_sub(1),
            Some(Placement::Join) | None => {}
        }
    }
    if used <= max {
        return (s.to_string(), used);
    }
    // keep the clusters before the first one that starts at or past the
    // room left for `…` (cells before a start = where the previous ends)
    let room = max.saturating_sub(1);
    let (byte, cells) = starts
        .iter()
        .skip(1)
        .take_while(|&&(_, before)| before <= room)
        .last()
        .copied()
        .unwrap_or((0, 0));
    (format!("{}…", &s[..byte]), cells + 1)
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
    // right side, from the edge: menu (⋮), copy icon, duration, badge pill
    let right = grid.right;
    let icon_w = 13.0;
    let menu_w = 9.0;
    let menu_x = right - menu_w - 4.0;
    let icon_x = menu_x - icon_w - 9.0;
    let badge_left = icon_x - 10.0 - duration_w - badge_w;
    // a gap before the pill, so "comm…" never touches it
    let left_limit = badge_left - 10.0;

    // command (bright), one line, ellipsized to the space it has
    let cmd_x = content_x + 10.0 + 11.0;
    let cmd = if block.command.is_empty() {
        "—".to_string()
    } else {
        rift_core::secrets::mask(&block.command).replace('\n', " ⏎ ")
    };
    // the collapsed-output note ("4 lines hidden"), when no filter note
    let hidden_note = match block.output_range() {
        Some((s, e)) if filter_note.is_none() && block.collapsed => {
            let n = e - s;
            Some(format!("{n} line{} hidden", if n == 1 { "" } else { "s" }))
        }
        _ => None,
    };
    // a note keeps its room after the command (the command gives way down
    // to eight cells, then the note is cut too)
    let note_room = match (filter_note, &hidden_note) {
        (Some(n), _) => engine.measure(n, 11.5, FONT_SANS, Weight::NORMAL) + 14.0 + 11.0,
        (None, Some(n)) => engine.measure(n, 11.5, mono_family(), Weight::NORMAL) + 11.0 + 10.0,
        (None, None) => 0.0,
    };
    let cmd_limit = (left_limit - note_room).max(cmd_x + 8.0 * grid.cell_w);
    let fit = ((cmd_limit.min(left_limit) - cmd_x) / grid.cell_w)
        .floor()
        .max(1.0) as usize;
    // cells as drawn: a CJK char is two, so counting chars ran it into
    // the badge and past the pane
    let (cmd, cmd_cells) = fit_cells(&cmd, fit);
    let cmd_buf = make_command_text(engine, &cmd, grid.font_size, grid.line_h, theme);
    layer.text(cmd_buf, cmd_x, y, theme.text_bright);

    // a note cut to `room` px (`None` when not even "…" fits)
    let fit_note = |engine: &mut TextEngine, note: &str, family, room: f32| {
        let full: Vec<char> = note.chars().collect();
        let mut note = note.to_string();
        let mut tw = engine.measure(&note, 11.5, family, Weight::NORMAL);
        let mut keep = full.len();
        while tw > room && keep > 0 {
            keep -= 1;
            note = full[..keep]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string()
                + "…";
            tw = engine.measure(&note, 11.5, family, Weight::NORMAL);
        }
        (tw <= room).then_some((note, tw))
    };

    // collapsed note, else Warp-style prompt context: ~/path git:(branch)
    let after_cmd_x = cmd_x + cmd_cells as f32 * grid.cell_w + 11.0;
    if let Some(note) = filter_note {
        // never under the badge and duration on the right
        let room = left_limit - after_cmd_x - 14.0;
        if let Some((note, tw)) = fit_note(engine, note, FONT_SANS, room) {
            layer.rect(
                [after_cmd_x - 2.0, y + (grid.line_h - 17.0) / 2.0],
                [tw + 14.0, 17.0],
                theme.warning.with_alpha(0.13),
                5.0,
            );
            let nbuf = make_text(engine, &note, 11.5, grid.line_h, FONT_SANS, Weight::NORMAL);
            layer.text(nbuf, after_cmd_x + 5.0, y, theme.warning);
        }
    } else if let Some(note) = &hidden_note {
        // design 01: italic mono, muted; never under the badge either
        let room = left_limit - after_cmd_x - 10.0;
        if let Some((note, _)) = fit_note(engine, note, mono_family(), room) {
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
        }
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

    // right side: badge pill, duration, copy icon, menu (⋮), laid out above

    // the copy icon (two overlapping rounded-rect outlines, a check for a
    // moment after a copy) and the menu's three stacked dots: only while
    // the pointer is on the header or the block is selected (their place
    // stays reserved, so nothing shifts)
    let hovered = grid
        .mouse
        .is_some_and(|(mx, my)| mx >= grid.x && mx < right && my >= y && my < y + grid.line_h);
    let copied = grid.copied == Some(block.id);
    if hovered || selected || copied {
        let ic = theme.text_muted;
        let cy = y + (grid.line_h - 13.0) / 2.0;
        if copied {
            let (a, b, c) = (
                [icon_x + 1.5, cy + 7.0],
                [icon_x + 5.0, cy + 10.5],
                [icon_x + 12.0, cy + 3.0],
            );
            layer.line(a, b, 1.6, theme.success);
            layer.line(b, c, 1.6, theme.success);
        } else {
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
        let mx = menu_x + menu_w / 2.0;
        let my = y + grid.line_h / 2.0;
        for dy in [-4.0, 0.0, 4.0] {
            layer.circle([mx, my + dy], 2.4, ic);
        }
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
    // copy and menu buttons
    out.copy_hits.push(HitRect {
        rect: [icon_x - 4.0, y, icon_w + 8.0, grid.line_h],
        block: block.id,
    });
    out.menu_hits.push(HitRect {
        rect: [menu_x - 4.0, y, menu_w + 8.0, grid.line_h],
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
    Some(rift_remote::protocol::PaneRunning {
        started,
        line,
        title: None,
        idle: false,
        agent: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    #[test]
    fn row_backgrounds_meet_on_whole_device_pixels() {
        for scale in [1.0, 2.0, 1.5] {
            let line_h = 15.6;
            let rows: Vec<_> = (0..20)
                .map(|i| snap_rect([8.0, 41.3 + i as f32 * line_h], [100.0, line_h], scale))
                .collect();
            for pair in rows.windows(2) {
                let (above, below) = (pair[0], pair[1]);
                // no gap and no overlap: the next row starts where this one ends
                assert_eq!(above.0[1] + above.1[1], below.0[1], "scale {scale}");
            }
            for (pos, size) in rows {
                for v in [pos[1], pos[1] + size[1]] {
                    assert_eq!((v * scale).fract(), 0.0, "scale {scale}: {v}");
                }
            }
        }
    }

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
    fn a_url_soft_wrapped_across_rows_is_a_link_on_each() {
        use rift_pty::alacritty_terminal::term::{Config, test::TermSize};
        let mut t = rift_pty::alacritty_terminal::term::Term::new(
            Config::default(),
            &TermSize::new(12, 4),
            Noop,
        );
        let g = t.grid_mut();
        for (r, text) in ["go https://e", "xample.com/a", "bc done", "x https://b"]
            .into_iter()
            .enumerate()
        {
            for (c, ch) in text.chars().enumerate() {
                g[Line(r as i32)][Column(c)].c = ch;
            }
        }
        g[Line(0)][Column(11)].flags.insert(Flags::WRAPLINE);
        g[Line(1)][Column(11)].flags.insert(Flags::WRAPLINE);
        let url = "https://example.com/abc".to_string();
        assert_eq!(urls_on_row(t.grid(), 0, 0), vec![(3, 9, url.clone())]);
        assert_eq!(urls_on_row(t.grid(), 0, 1), vec![(0, 12, url.clone())]);
        assert_eq!(urls_on_row(t.grid(), 0, 2), vec![(0, 2, url)]);
        // a row that does not wrap stands alone
        assert_eq!(
            urls_on_row(t.grid(), 0, 3),
            vec![(2, 9, "https://b".to_string())]
        );
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
        let m = Matcher::new("\u{e9}y", true, true).unwrap().unwrap();
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

    /// `mask_secrets`: what the gateway sends a phone (screens, output
    /// tails and pages) carries the secrets masked, cell for cell.
    #[test]
    fn the_phone_gets_secrets_masked() {
        let token = format!("ghp_{}", "FAKE0123".repeat(5));
        let bullets = "•".repeat(token.len());
        rift_core::secrets::set_active(Some(std::sync::Arc::new(
            rift_core::secrets::Masker::new(&[]).unwrap(),
        )));
        let (t, _) = fed(80, 4, 100, format!("\x1b[?1049hkey {token} 日").as_bytes());
        let screen = screen_of(&t, &Theme::default(), false).unwrap();
        let (o, scrolled) = fed(80, 6, 100, format!("out {token}\r\nnext\r\n").as_bytes());
        let (tail, _) = output_tail(&o, scrolled, (0, Some(2), false));
        let page = output_range(&o, scrolled, (0, Some(2), false), 0, 10);
        rift_core::secrets::set_active(None);
        let row: String = screen.lines[0].iter().map(|r| r.text.as_str()).collect();
        assert_eq!(row, format!("key {bullets} 日"));
        assert_eq!(tail, format!("out {bullets}\nnext\n"));
        assert_eq!(page.text, tail);
        // masking off: the text as printed
        let (o, scrolled) = fed(80, 6, 100, format!("out {token}\r\n").as_bytes());
        assert!(
            output_tail(&o, scrolled, (0, Some(1), false))
                .0
                .contains(&token)
        );
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
    fn a_running_block_above_its_start_has_no_output() {
        // the program cleared its screen: the cursor sits above the block's start
        let (t, scrolled) = fed(20, 5, 50, b"hi");
        assert_eq!(
            output_tail(&t, scrolled, (10, None, true)),
            (String::new(), 0)
        );
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
    fn a_phones_view_scrolls_within_its_own_list_and_stays_put_on_new_output() {
        let mut v = RemoteView::default();
        // 100 display lines, 20 rows: 80 lines above the first screen
        v.settle(100, 20);
        assert_eq!(
            v,
            RemoteView {
                offset: 0,
                len: 100
            }
        );
        v.scroll(50, 80);
        v.scroll(50, 80);
        // never past the top of the list the phone draws, so the way back
        // down starts at once
        assert_eq!(v.offset, 80);
        v.scroll(-30, 80);
        assert_eq!(v.offset, 50);
        // 7 lines of new output: the view keeps showing what it showed
        v.settle(107, 20);
        assert_eq!(
            v,
            RemoteView {
                offset: 57,
                len: 107
            }
        );
        // following the end: new output scrolls it along
        v.scroll(-50, 87);
        v.scroll(-50, 87);
        assert_eq!(v.offset, 0);
        v.settle(110, 20);
        assert_eq!(
            v,
            RemoteView {
                offset: 0,
                len: 110
            }
        );
        // a shorter list (cleared): clamped to its top
        v.scroll(50, 90);
        v.settle(30, 20);
        assert_eq!(
            v,
            RemoteView {
                offset: 10,
                len: 30
            }
        );
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
            let m = Matcher::new(q, false, false).unwrap().unwrap();
            assert_eq!(m.find_line(&row), vec![(0, 4), (5, 4)], "{q:?}");
        }
        // a regex matches code points of the composed line: a typed
        // (composed) accent finds both spellings
        for q in ["caf\u{e9}", "CAF\u{c9}", "caf[\u{e9}]", "caf\u{e9}+"] {
            let m = Matcher::new(q, false, true).unwrap().unwrap();
            assert_eq!(m.find_line(&row), vec![(0, 4), (5, 4)], "{q:?}");
        }
    }

    /// Header commands are budgeted in drawn cells: counting chars let a
    /// CJK command run through the badge and off the pane.
    #[test]
    fn fit_cells_counts_drawn_cells_and_keeps_clusters_whole() {
        assert_eq!(fit_cells("echo hi", 7), ("echo hi".into(), 7));
        assert_eq!(fit_cells("echo hi", 6), ("echo …".into(), 6));
        assert_eq!(fit_cells("漢字漢字", 8), ("漢字漢字".into(), 8));
        // 5 cells: two wide chars and the ellipsis, never half a char
        assert_eq!(fit_cells("漢字漢字", 6), ("漢字…".into(), 5));
        assert_eq!(fit_cells("漢字漢字", 2), ("…".into(), 1));
        // a combining mark takes no cell and stays on its base
        assert_eq!(fit_cells("cafe\u{301}!", 5), ("cafe\u{301}!".into(), 5));
        assert_eq!(fit_cells("cafe\u{301}!!", 5), ("cafe\u{301}…".into(), 5));
        // a ZWJ family is one two-cell cluster
        let fam = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        assert_eq!(fit_cells(&format!("a{fam}b"), 4).1, 4);
        assert_eq!(fit_cells(&format!("a{fam}bc"), 4), (format!("a{fam}…"), 4));
    }

    /// A regex source is syntax: normalizing it turned `[é]` into a class
    /// of `e` and U+0301, which matched plain `e` and missed `é`.
    #[test]
    fn regex_classes_keep_accented_chars_whole() {
        // as a grid holds them: a combining mark rides on its base cell
        let line = |cells: &[&str]| {
            let mut l = SearchLine::default();
            for (col, cell) in cells.iter().enumerate() {
                for c in cell.chars() {
                    l.push(c, col, col + 1);
                }
            }
            l
        };
        let lines = [
            line(&["e"]),
            line(&["\u{e9}"]),
            line(&["\u{e9}", "\u{e9}"]),
            line(&["e\u{301}"]),
        ];
        let hits = |q: &str| -> Vec<usize> {
            let m = Matcher::new(q, true, true).unwrap().unwrap();
            lines
                .iter()
                .enumerate()
                .filter(|(_, l)| !m.find_line(l).is_empty())
                .map(|(i, _)| i)
                .collect()
        };
        assert_eq!(hits("^[\u{e9}]$"), vec![1, 3]);
        assert_eq!(hits("^\u{e9}+$"), vec![1, 2, 3]);
        assert_eq!(hits("^(e|x)$"), vec![0]);
        assert_eq!(hits(r"^\x{e9}$"), vec![1, 3]);
        // the decomposed cell still highlights as one column
        let m = Matcher::new("\u{e9}", true, true).unwrap().unwrap();
        assert_eq!(m.find_line(&line(&["a", "e\u{301}", "b"])), vec![(1, 1)]);
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
            cwd_remote: false,
            branch: None,
            host: None,
            restored: false,
            replayed: false,
            finished_at: None,
            from_editor: false,
            background: false,
            skip_cols: 0,
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
    fn normal_spacing_puts_a_blank_row_above_each_later_header() {
        // two blocks: prompts on 1 and 4, outputs 2..4 and 5..7
        let blocks = vec![
            block(7, 1, Some(2), Some(4), done(), false),
            block(8, 4, Some(5), Some(7), done(), false),
        ];
        let list = |spaced| {
            build_display_list_filtered(
                &blocks,
                0,
                8,
                0,
                false,
                false,
                None,
                8,
                None,
                &Hidden::default(),
                spaced,
            )
        };
        use DisplayLine::*;
        assert_eq!(
            list(false),
            [
                Grid(0),
                Header(0),
                Grid(2),
                Grid(3),
                Header(1),
                Grid(5),
                Grid(6),
                Grid(7)
            ]
        );
        assert_eq!(
            list(true),
            [
                Grid(0),
                Pad,
                Header(0),
                Grid(2),
                Grid(3),
                Pad,
                Header(1),
                Grid(5),
                Grid(6),
                Grid(7)
            ]
        );
        // a header at the very top gets no blank row above it
        let top = vec![block(7, 0, Some(1), Some(3), done(), false)];
        let d = build_display_list_filtered(
            &top,
            0,
            4,
            0,
            false,
            false,
            None,
            4,
            None,
            &Hidden::default(),
            true,
        );
        assert_eq!(d[0], Header(0));
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
    fn bold_is_bright_maps_the_eight_ansi_colours() {
        use ansi::Color as C;
        assert_eq!(
            bright_for_bold(C::Named(NamedColor::Red)),
            C::Named(NamedColor::BrightRed)
        );
        assert_eq!(
            bright_for_bold(C::Named(NamedColor::Black)),
            C::Named(NamedColor::BrightBlack)
        );
        assert_eq!(bright_for_bold(C::Indexed(7)), C::Indexed(15));
        // already bright, the 256-colour cube, truecolour and the default
        // foreground stay
        for c in [
            C::Named(NamedColor::BrightRed),
            C::Named(NamedColor::Foreground),
            C::Indexed(8),
            C::Indexed(196),
            C::Spec(ansi::Rgb { r: 1, g: 2, b: 3 }),
        ] {
            assert_eq!(bright_for_bold(c), c);
        }
    }

    #[test]
    fn feature_lists_are_interned() {
        let a = intern_features(vec![(*b"ss01", 1), (*b"zero", 1)]);
        let b = intern_features(vec![(*b"ss01", 1), (*b"zero", 1)]);
        assert!(std::ptr::eq(a, b), "one copy per distinct list");
        assert_eq!(a, &[(*b"ss01", 1), (*b"zero", 1)]);
        let c = intern_features(vec![(*b"ss01", 0)]);
        assert!(!std::ptr::eq(a, c));
    }

    #[test]
    fn legacy_computing_blocks() {
        // sextant-1 (U+1FB00): the top-left sixth
        assert_eq!(
            block_element('\u{1FB00}'),
            Some((vec![[0.0, 0.0, 0.5, 1.0 / 3.0]], 1.0))
        );
        // U+1FB14 is sextant-235 (pattern 21, ▌, is skipped before it)
        let (parts, _) = block_element('\u{1FB14}').unwrap();
        assert_eq!(
            parts,
            vec![
                [0.5, 0.0, 1.0, 1.0 / 3.0],
                [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0],
                [0.0, 2.0 / 3.0, 0.5, 1.0]
            ]
        );
        // the last, sextant-23456 (pattern 62)
        assert_eq!(block_element('\u{1FB3B}').unwrap().0.len(), 5);
        // eighth blocks: column 2, row 7, upper quarter, right 7/8
        assert_eq!(
            block_element('\u{1FB70}').unwrap().0,
            vec![[0.125, 0.0, 0.25, 1.0]]
        );
        assert_eq!(
            block_element('\u{1FB7B}').unwrap().0,
            vec![[0.0, 0.75, 1.0, 0.875]]
        );
        assert_eq!(
            block_element('\u{1FB82}').unwrap().0,
            vec![[0.0, 0.0, 1.0, 0.25]]
        );
        assert_eq!(
            block_element('\u{1FB8B}').unwrap().0,
            vec![[0.125, 0.0, 1.0, 1.0]]
        );
        // smooth mosaics (U+1FB3C on) and checkers stay glyphs
        assert!(block_element('\u{1FB3C}').is_none());
        assert!(block_element('\u{1FB95}').is_none());
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
            weights: (400, 700),
            features: &[],
            min_contrast: 1.0,
            bold_is_bright: false,
            cursor_style: CursorStyle::Block,
            editor_owns_cursor: true,
            text_blink_on: true,
            bottom_up: true,
            mouse: None,
            copied: None,
            scale: 2.0,
            spaced_blocks: false,
            block_dividers: false,
            jump_button: false,
            scrollbar: false,
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
            &Hidden::default(),
            false,
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
                &Hidden::default(),
                false,
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

    /// The text of a screen row.
    fn row_text(row: &[rift_remote::protocol::Run]) -> String {
        row.iter().map(|r| r.text.as_str()).collect()
    }

    /// The pane `user@host % ls` ran in: the echo (0), two output lines, then
    /// the live prompt (3) of the next block.
    fn styled_fixture() -> (Vec<Block>, Vec<&'static str>) {
        let mut done_block = block(1, 0, Some(1), Some(3), done(), false);
        done_block.command = "ls".into();
        done_block.cwd = Some("/work/rift".into());
        done_block.branch = Some("main".into());
        (
            vec![
                done_block,
                block(2, 3, None, None, BlockState::Prompt, false),
            ],
            vec!["user@host % ls", "a.txt", "b.txt", "user@host % "],
        )
    }

    fn styled(
        blocks: &[Block],
        lines: &[&str],
        hide_prompt: bool,
        cursor_abs: Option<u64>,
    ) -> (
        Vec<String>,
        Option<usize>,
        Vec<Vec<rift_remote::protocol::Run>>,
    ) {
        let d = build_display_list_filtered(
            blocks,
            0,
            6,
            0,
            false,
            hide_prompt,
            None,
            6,
            Some(4),
            &Hidden::default(),
            false,
        );
        let StyledRows {
            lines: rows,
            cursor,
            ..
        } = styled_rows(
            &d,
            (6, 40),
            blocks,
            &Theme::default(),
            |_| None,
            |abs| {
                vec![run(
                    lines.get(abs as usize).copied().unwrap_or(""),
                    None,
                    None,
                    0,
                )]
            },
            cursor_abs,
            0,
        );
        (rows.iter().map(|r| row_text(r)).collect(), cursor, rows)
    }

    #[test]
    fn a_styled_screen_follows_the_panes_scrolled_view() {
        let display: Vec<DisplayLine> = (0..10).map(DisplayLine::Grid).collect();
        let shown = |d: &[DisplayLine], offset: usize| {
            let StyledRows { lines, cursor, .. } = styled_rows(
                d,
                (4, 20),
                &[],
                &Theme::default(),
                |_| None,
                |abs| vec![run(&format!("g{abs}"), None, None, 0)],
                Some(9),
                offset,
            );
            let text: Vec<String> = lines.iter().map(|r| row_text(r)).collect();
            (text, cursor)
        };
        // at the end: the last rows, the cursor on the last
        assert_eq!(
            shown(&display, 0),
            (
                vec!["g6".into(), "g7".into(), "g8".into(), "g9".into()],
                Some(3)
            )
        );
        // scrolled up 3: the rows above, the cursor out of sight
        assert_eq!(
            shown(&display, 3),
            (
                vec!["g3".into(), "g4".into(), "g5".into(), "g6".into()],
                None
            )
        );
        // never above the first line
        assert_eq!(shown(&display, 100).0, ["g0", "g1", "g2", "g3"]);
        // fewer lines than rows: nothing to scroll, blank rows above
        assert_eq!(shown(&display[..2], 5).0, ["", "", "g0", "g1"]);
    }

    #[test]
    fn a_styled_screen_shows_a_header_not_the_prompt_echo() {
        let (blocks, lines) = styled_fixture();
        let (text, _, rows) = styled(&blocks, &lines, true, None);
        // bottom-anchored like the desktop: blank pads, header, output; the
        // live prompt is hidden (the editor owns it)
        assert_eq!(text.len(), 6);
        assert!(text[..3].iter().all(String::is_empty), "{text:?}");
        assert_eq!(&text[4..], ["a.txt", "b.txt"]);
        let header = &text[3];
        assert!(
            header.starts_with("▾ ls  /work/rift git:(main)"),
            "{header:?}"
        );
        assert!(header.ends_with("✓ 0 1ms"), "{header:?}");
        assert_eq!(header.chars().count(), 40, "{header:?}");
        assert!(!text.iter().any(|t| t.contains("user@host")), "{text:?}");
        // the command is bold in the bright colour; the badge takes the
        // exit colour
        let theme = Theme::default();
        let cmd = &rows[3][1];
        assert_eq!(
            (cmd.text.as_str(), cmd.flags, cmd.fg.as_deref()),
            ("ls", 1, Some(hex8(rgb8(theme.text_bright)).as_str()))
        );
        let badge = rows[3].iter().find(|r| r.text == "✓ 0").unwrap();
        assert_eq!(
            badge.fg.as_deref(),
            Some(hex8(rgb8(theme.success)).as_str())
        );
    }

    #[test]
    fn a_styled_screen_negative_controls() {
        let (blocks, lines) = styled_fixture();
        // the prompt is shown when the editor does not own it
        let (text, _, _) = styled(&blocks, &lines, false, None);
        assert_eq!(text[5], "user@host % ");
        // and with no block anchors the echo is the raw grid line
        let raw: Vec<Block> = Vec::new();
        let (text, _, _) = styled(&raw, &lines, true, None);
        assert!(text.iter().any(|t| t == "user@host % ls"), "{text:?}");
        assert!(!text.iter().any(|t| t.starts_with("▾")), "{text:?}");
    }

    #[test]
    fn a_styled_screen_maps_the_cursor_to_its_display_row() {
        let (blocks, lines) = styled_fixture();
        // a program's cursor on the second output line: grid line 2 is row 5
        let (_, cursor, _) = styled(&blocks, &lines, true, Some(2));
        assert_eq!(cursor, Some(5));
        // on a line the display skips (the hidden prompt): none
        let (_, cursor, _) = styled(&blocks, &lines, true, Some(3));
        assert_eq!(cursor, None);
        let (_, cursor, _) = styled(&blocks, &lines, false, Some(3));
        assert_eq!(cursor, Some(5));
    }

    #[test]
    fn a_styled_header_fits_its_width_and_marks_collapsed_and_failed_blocks() {
        let theme = Theme::default();
        let mut b = block(
            1,
            0,
            Some(1),
            Some(5),
            BlockState::Done {
                exit: Some(2),
                duration: StdDuration::from_millis(1500),
            },
            true,
        );
        b.command = "cargo test --workspace -- --nocapture".into();
        let cols = 30;
        let h = header_runs(&b, &theme, cols);
        let text = row_text(&h);
        assert_eq!(text.chars().count(), cols, "{text:?}");
        assert!(text.starts_with("▸ cargo test"), "{text:?}");
        assert!(
            text.contains('…') && text.ends_with("✕ 2 1.50s"),
            "{text:?}"
        );
        // the hidden-lines note shows where there is room
        b.command = "ls".into();
        let text = row_text(&header_runs(&b, &theme, 60));
        assert!(text.contains("  4 lines hidden"), "{text:?}");
        // a screen too narrow for the badge keeps the command alone
        let text = row_text(&header_runs(&b, &theme, 6));
        assert_eq!(text, "▸ ls");
    }

    #[test]
    fn a_styled_screen_lists_the_rows_with_a_block_rule_and_its_colour() {
        let theme = Theme::default();
        let mut ok = block(1, 0, Some(1), Some(3), done(), false);
        ok.command = "good".into();
        let mut bad = block(
            2,
            3,
            Some(4),
            Some(5),
            BlockState::Done {
                exit: Some(2),
                duration: StdDuration::from_millis(1),
            },
            false,
        );
        bad.command = "bad".into();
        let mut live = block(
            3,
            5,
            Some(6),
            None,
            BlockState::Running {
                started: std::time::Instant::now(),
            },
            false,
        );
        live.command = "live".into();
        // a prompt being typed carries no rule
        let prompt = block(4, 8, None, None, BlockState::Prompt, false);
        let blocks = vec![ok, bad, live, prompt];
        let d = build_display_list_filtered(
            &blocks,
            0,
            9,
            0,
            false,
            false,
            None,
            9,
            Some(8),
            &Hidden::default(),
            false,
        );
        let StyledRows {
            lines: rows, rules, ..
        } = styled_rows(
            &d,
            (12, 40),
            &blocks,
            &theme,
            |abs| block_rule_at(&blocks, &theme, 8, abs),
            |abs| vec![run(&format!("g{abs}"), None, None, 0)],
            None,
            0,
        );
        let colour = |row: usize| {
            rules
                .iter()
                .find(|(r, _)| *r as usize == row)
                .map(|(_, c)| c.clone())
        };
        let find = |needle: &str| {
            rows.iter()
                .position(|r| row_text(r).contains(needle))
                .unwrap_or_else(|| panic!("no row with {needle}"))
        };
        let (success, error, accent) = (hex(theme.success), hex(theme.error), hex(theme.accent));
        // header and output rows of each block, by state
        for (row, want) in [
            ("good", &success),
            ("g1", &success),
            ("g2", &success),
            ("bad", &error),
            ("g4", &error),
            ("live", &accent),
            ("g6", &accent),
        ] {
            assert_eq!(colour(find(row)).as_ref(), Some(want), "{row}");
        }
        // padding above the first block: none
        assert_eq!(colour(0), None);
        // a row is listed once, in order
        let listed: Vec<u16> = rules.iter().map(|(r, _)| *r).collect();
        assert!(listed.windows(2).all(|w| w[0] < w[1]), "{listed:?}");
        // the alternate screen has no rules, but the theme's ground
        let (t, _) = fed(12, 2, 100, b"\x1b[?1049hvim");
        let s = screen_of(&t, &theme, false).unwrap();
        assert_eq!(
            (s.fg, s.bg, s.rules),
            (Some(hex(theme.fg)), Some(hex(theme.bg)), Vec::new())
        );
    }

    #[test]
    fn a_header_badge_is_a_pill_tinted_with_its_colour_over_the_ground() {
        let theme = Theme::default();
        let badge_bg = |state: BlockState| {
            let b = block(1, 0, Some(1), Some(3), state, false);
            let runs = header_runs(&b, &theme, 60);
            // only the badge has a background
            let tinted: Vec<_> = runs.iter().filter(|r| r.bg.is_some()).collect();
            assert_eq!(tinted.len(), 1, "{runs:?}");
            (tinted[0].text.clone(), tinted[0].bg.clone().unwrap())
        };
        let tint = |c: Color| hex(theme.bg.mix(c, 0.13));
        assert_eq!(badge_bg(done()), ("✓ 0".to_string(), tint(theme.success)));
        assert_eq!(
            badge_bg(BlockState::Done {
                exit: Some(3),
                duration: StdDuration::from_millis(1),
            }),
            ("✕ 3".to_string(), tint(theme.error))
        );
        assert_eq!(
            badge_bg(BlockState::Running {
                started: std::time::Instant::now()
            }),
            ("running".to_string(), tint(theme.warning))
        );
    }
}
