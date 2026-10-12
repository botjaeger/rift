//! The file viewer's *Rendered* mode for Markdown: lays the blocks from
//! `rift_core::markdown` out at the viewer's width once (cached until the
//! width or theme changes) and draws only the blocks on screen.

use std::sync::Arc;

use rift_core::markdown::{Align, Block, Span};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{
    Attrs, Buffer, Family, Metrics, Shaping, Style, TextBounds, Weight, Wrap,
};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

/// Side padding of the rendered page.
pub const PAD_X: f32 = 28.0;
/// Space above the first block and below the last.
pub const PAD_Y: f32 = 16.0;
const BODY: (f32, f32) = (13.5, 22.0);
const CODE: (f32, f32) = (12.5, 19.0);
/// Glyph metadata marking inline code and strikethrough runs.
const META_CODE: usize = 1;
const META_STRIKE: usize = 2;

enum Draw {
    Text {
        buf: Arc<Buffer>,
        x: f32,
        y: f32,
        /// Clip width (code blocks scroll nothing: long lines are cut).
        clip_w: Option<f32>,
    },
    Rect {
        r: [f32; 4],
        color: Color,
        radius: f32,
    },
}

struct Laid {
    top: f32,
    h: f32,
    draws: Vec<Draw>,
    /// Code block content past its clip (px of horizontal scroll).
    overflow: f32,
}

/// Blocks laid out at one width and theme.
#[derive(Default)]
pub struct MdLayout {
    key: Option<(u32, [u32; 2])>,
    blocks: Vec<Laid>,
    pub height: f32,
}

impl MdLayout {
    /// The code block at content `y` that is wider than its box: (block
    /// index, how far it scrolls).
    pub fn code_at(&self, y: f32) -> Option<(usize, f32)> {
        self.blocks
            .iter()
            .position(|b| y >= b.top && y < b.top + b.h)
            .map(|i| (i, self.blocks[i].overflow))
            .filter(|(_, o)| *o > 0.0)
    }
}

impl std::fmt::Debug for MdLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MdLayout({} blocks, {}px)",
            self.blocks.len(),
            self.height
        )
    }
}

fn key(width: f32, theme: &Theme) -> (u32, [u32; 2]) {
    let c = |c: Color| {
        (c.r * 255.0) as u32 | ((c.g * 255.0) as u32) << 8 | ((c.b * 255.0) as u32) << 16
    };
    (width.round() as u32, [c(theme.fg), c(theme.accent)])
}

/// Wrapped rich text: the buffer, its height, and the rects of its inline
/// code (background) and strikethrough (line) runs, relative to the buffer.
fn rich(
    engine: &mut TextEngine,
    theme: &Theme,
    spans: &[Span],
    (size, line): (f32, f32),
    width: f32,
    base: Color,
    weight: Weight,
) -> (Buffer, f32, Vec<Draw>) {
    let fs = &mut engine.font_system;
    let sans = Attrs::new()
        .family(Family::Name(FONT_SANS))
        .weight(weight)
        .color(text_color(base));
    let parts: Vec<(&str, Attrs)> = spans
        .iter()
        .map(|s| {
            let st = s.style;
            let mut a = sans.clone();
            if st.code {
                a = a.family(Family::Name(mono_family())).metadata(META_CODE);
            }
            if st.bold {
                a = a
                    .weight(Weight::SEMIBOLD)
                    .color(text_color(theme.text_bright));
            }
            if st.italic {
                a = a.style(Style::Italic);
            }
            if st.link {
                a = a.color(text_color(theme.accent));
            }
            if st.strike {
                a = a.metadata(META_STRIKE).color(text_color(theme.text_dim));
            }
            (s.text.as_str(), a)
        })
        .collect();
    let mut buf = Buffer::new(fs, Metrics::new(size, line));
    buf.set_wrap(fs, Wrap::WordOrGlyph);
    buf.set_size(fs, Some(width.max(1.0)), None);
    buf.set_rich_text(fs, parts, &sans, Shaping::Advanced, None);
    buf.shape_until_scroll(fs, false);
    let mut marks = Vec::new();
    let mut lines = 0;
    for run in buf.layout_runs() {
        lines += 1;
        // merge consecutive glyphs of one kind into one rect
        let mut cur: Option<(usize, f32, f32)> = None;
        let flush = |cur: Option<(usize, f32, f32)>, marks: &mut Vec<Draw>| {
            if let Some((meta, x0, x1)) = cur {
                marks.push(if meta == META_CODE {
                    Draw::Rect {
                        r: [x0 - 3.0, run.line_top + 1.0, x1 - x0 + 6.0, line - 2.0],
                        color: Color::WHITE.with_alpha(0.08),
                        radius: 4.0,
                    }
                } else {
                    Draw::Rect {
                        r: [x0, run.line_top + line / 2.0, x1 - x0, 1.0],
                        color: theme.text_dim,
                        radius: 0.0,
                    }
                });
            }
        };
        for g in run.glyphs {
            let meta = g.metadata;
            match cur {
                Some((m, x0, _)) if m == meta && meta != 0 => cur = Some((m, x0, g.x + g.w)),
                _ => {
                    flush(cur, &mut marks);
                    cur = (meta != 0).then_some((meta, g.x, g.x + g.w));
                }
            }
        }
        flush(cur, &mut marks);
    }
    (buf, lines.max(1) as f32 * line, marks)
}

fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Lay out `blocks` at `width` unless the cache already holds that.
pub fn layout(
    cache: &mut MdLayout,
    engine: &mut TextEngine,
    theme: &Theme,
    blocks: &[Block],
    width: f32,
) {
    let k = key(width, theme);
    if cache.key == Some(k) {
        return;
    }
    cache.key = Some(k);
    cache.blocks.clear();
    let w = (width - 2.0 * PAD_X).max(40.0);
    let mut y = PAD_Y;
    let mut prev_heading = false;
    for (n, block) in blocks.iter().enumerate() {
        let mut draws = Vec::new();
        // code wider than its block: how far it scrolls sideways
        let mut overflow = 0.0;
        // space above this block
        let gap = match block {
            Block::Heading { level, .. } if n > 0 => {
                if *level <= 2 {
                    24.0
                } else {
                    18.0
                }
            }
            Block::Item { .. }
                if matches!(blocks.get(n.wrapping_sub(1)), Some(Block::Item { .. })) =>
            {
                4.0
            }
            _ if n == 0 => 0.0,
            _ if prev_heading => 8.0,
            _ => 12.0,
        };
        y += gap;
        let h = match block {
            Block::Heading { level, spans } => {
                let size = match level {
                    1 => (24.0, 32.0),
                    2 => (19.0, 26.0),
                    3 => (16.0, 23.0),
                    _ => (14.0, 21.0),
                };
                let (buf, h, marks) = rich(
                    engine,
                    theme,
                    spans,
                    size,
                    w,
                    theme.text_bright,
                    Weight::SEMIBOLD,
                );
                draws.extend(marks.into_iter().map(|d| offset(d, PAD_X, 0.0)));
                draws.push(Draw::Text {
                    buf: Arc::new(buf),
                    x: PAD_X,
                    y: 0.0,
                    clip_w: None,
                });
                if *level <= 2 {
                    draws.push(Draw::Rect {
                        r: [PAD_X, h + 7.0, w, 1.0],
                        color: theme.border,
                        radius: 0.0,
                    });
                    h + 8.0
                } else {
                    h
                }
            }
            Block::Paragraph(spans) => {
                text_block(engine, theme, spans, PAD_X, w, theme.fg, &mut draws)
            }
            Block::Item {
                depth,
                marker,
                spans,
            } => {
                let indent = PAD_X + *depth as f32 * 20.0;
                let mut mb = Buffer::new(&mut engine.font_system, Metrics::new(BODY.0, BODY.1));
                mb.set_text(
                    &mut engine.font_system,
                    marker,
                    &Attrs::new()
                        .family(Family::Name(FONT_SANS))
                        .color(text_color(theme.text_dim)),
                    Shaping::Advanced,
                    None,
                );
                let numbered = marker.ends_with('.');
                let mw = engine.measure(marker, BODY.0, FONT_SANS, Weight::NORMAL);
                draws.push(Draw::Text {
                    buf: Arc::new(mb),
                    // numbers right-align in the marker column
                    x: if numbered {
                        indent + 14.0 - mw
                    } else {
                        indent + 2.0
                    },
                    y: 0.0,
                    clip_w: None,
                });
                let tx = indent + 20.0;
                text_block(
                    engine,
                    theme,
                    spans,
                    tx,
                    (w - (tx - PAD_X)).max(40.0),
                    theme.fg,
                    &mut draws,
                )
            }
            Block::Quote(spans) => {
                let h = text_block(
                    engine,
                    theme,
                    spans,
                    PAD_X + 16.0,
                    w - 16.0,
                    theme.text_dim,
                    &mut draws,
                );
                draws.push(Draw::Rect {
                    r: [PAD_X, 0.0, 3.0, h],
                    color: theme.border,
                    radius: 1.5,
                });
                h
            }
            Block::Code(lines) => {
                let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(CODE.0, CODE.1));
                buf.set_wrap(&mut engine.font_system, Wrap::None);
                buf.set_text(
                    &mut engine.font_system,
                    &lines.join("\n"),
                    &Attrs::new()
                        .family(Family::Name(mono_family()))
                        .color(text_color(theme.fg)),
                    Shaping::Advanced,
                    None,
                );
                let h = lines.len().max(1) as f32 * CODE.1 + 24.0;
                draws.push(Draw::Rect {
                    r: [PAD_X, 0.0, w, h],
                    color: Color::WHITE.with_alpha(0.045),
                    radius: 4.0,
                });
                draws.push(Draw::Text {
                    buf: Arc::new(buf),
                    x: PAD_X + 14.0,
                    y: 12.0,
                    clip_w: Some(w - 28.0),
                });
                overflow = (lines
                    .iter()
                    .max_by_key(|l| l.chars().count())
                    .map_or(0.0, |l| {
                        engine.measure(l, CODE.0, mono_family(), Weight::NORMAL)
                    })
                    - (w - 28.0))
                    .max(0.0);
                h
            }
            Block::Rule => {
                draws.push(Draw::Rect {
                    r: [PAD_X, 4.0, w, 1.0],
                    color: theme.border,
                    radius: 0.0,
                });
                9.0
            }
            Block::Table {
                align,
                header,
                rows,
            } => table(engine, theme, align, header, rows, w, &mut draws),
        };
        prev_heading = matches!(block, Block::Heading { .. });
        cache.blocks.push(Laid {
            top: y,
            h,
            draws,
            overflow,
        });
        y += h;
    }
    cache.height = y + PAD_Y;
}

fn offset(d: Draw, dx: f32, dy: f32) -> Draw {
    match d {
        Draw::Rect { r, color, radius } => Draw::Rect {
            r: [r[0] + dx, r[1] + dy, r[2], r[3]],
            color,
            radius,
        },
        Draw::Text { buf, x, y, clip_w } => Draw::Text {
            buf,
            x: x + dx,
            y: y + dy,
            clip_w,
        },
    }
}

/// Body text at `x`, `w` wide; returns its height.
fn text_block(
    engine: &mut TextEngine,
    theme: &Theme,
    spans: &[Span],
    x: f32,
    w: f32,
    color: Color,
    draws: &mut Vec<Draw>,
) -> f32 {
    let (buf, h, marks) = rich(engine, theme, spans, BODY, w, color, Weight::NORMAL);
    draws.extend(marks.into_iter().map(|d| offset(d, x, 0.0)));
    draws.push(Draw::Text {
        buf: Arc::new(buf),
        x,
        y: 0.0,
        clip_w: None,
    });
    h
}

/// A bordered grid: header row on a faint band, cells wrapped to their
/// column, columns sized to their content and shrunk to fit `w`.
fn table(
    engine: &mut TextEngine,
    theme: &Theme,
    align: &[Align],
    header: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
    w: f32,
    draws: &mut Vec<Draw>,
) -> f32 {
    const CELL_PAD: f32 = 10.0;
    let size = (13.0, 20.0);
    let cols = align.len().max(1);
    let mut natural = vec![0.0f32; cols];
    for (r, row) in std::iter::once(header)
        .chain(rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        for (c, cell) in row.iter().enumerate().take(cols) {
            let weight = if r == 0 {
                Weight::SEMIBOLD
            } else {
                Weight::NORMAL
            };
            let tw = engine.measure(&plain_text(cell), size.0, FONT_SANS, weight);
            natural[c] = natural[c].max(tw + 2.0 * CELL_PAD + 2.0);
        }
    }
    let total: f32 = natural.iter().sum();
    let widths: Vec<f32> = if total <= w {
        natural
    } else {
        // shrink the wide columns first: each keeps at least its share
        let fair = w / cols as f32;
        let small: f32 = natural.iter().filter(|n| **n <= fair).sum();
        let big: f32 = natural.iter().filter(|n| **n > fair).sum();
        let room = (w - small).max(0.0);
        natural
            .iter()
            .map(|n| {
                if *n <= fair {
                    *n
                } else {
                    (n / big.max(1.0) * room).max(48.0)
                }
            })
            .collect()
    };
    let table_w: f32 = widths.iter().sum();
    let mut y = 0.0;
    let mut row_tops = Vec::new();
    for (r, row) in std::iter::once(header)
        .chain(rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        let mut cells = Vec::new();
        let mut row_h: f32 = 0.0;
        let mut x = 0.0;
        for (c, &col_w) in widths.iter().enumerate().take(cols) {
            let empty = Vec::new();
            let cell = row.get(c).unwrap_or(&empty);
            let inner = (col_w - 2.0 * CELL_PAD).max(8.0);
            let (color, weight) = if r == 0 {
                (theme.text_bright, Weight::SEMIBOLD)
            } else {
                (theme.fg, Weight::NORMAL)
            };
            let (buf, h, marks) = rich(engine, theme, cell, size, inner, color, weight);
            // alignment of a single-line cell
            let tw = buf
                .layout_runs()
                .map(|run| run.line_w)
                .fold(0.0f32, f32::max);
            let dx = match align.get(c) {
                Some(Align::Right) => inner - tw,
                Some(Align::Center) => (inner - tw) / 2.0,
                _ => 0.0,
            }
            .max(0.0);
            cells.push((buf, marks, PAD_X + x + CELL_PAD + dx));
            row_h = row_h.max(h);
            x += col_w;
        }
        let row_h = row_h + 2.0 * 7.0;
        if r == 0 {
            draws.push(Draw::Rect {
                r: [PAD_X, y, table_w, row_h],
                color: Color::WHITE.with_alpha(0.045),
                radius: 0.0,
            });
        }
        for (buf, marks, cx) in cells {
            draws.extend(marks.into_iter().map(|d| offset(d, cx, y + 7.0)));
            draws.push(Draw::Text {
                buf: Arc::new(buf),
                x: cx,
                y: y + 7.0,
                clip_w: None,
            });
        }
        row_tops.push(y);
        y += row_h;
    }
    // grid: row rules, column rules, outer frame
    for &ry in row_tops.iter().skip(1) {
        draws.push(Draw::Rect {
            r: [PAD_X, ry, table_w, 1.0],
            color: theme.border,
            radius: 0.0,
        });
    }
    let mut x = PAD_X;
    for cw in &widths[..widths.len().saturating_sub(1)] {
        x += cw;
        draws.push(Draw::Rect {
            r: [x, 0.0, 1.0, y],
            color: theme.border,
            radius: 0.0,
        });
    }
    for r in [
        [PAD_X, 0.0, table_w, 1.0],
        [PAD_X, y - 1.0, table_w, 1.0],
        [PAD_X, 0.0, 1.0, y],
        [PAD_X + table_w - 1.0, 0.0, 1.0, y],
    ] {
        draws.push(Draw::Rect {
            r,
            color: theme.border,
            radius: 0.0,
        });
    }
    y
}

/// Draw the laid-out page with its top at `top` (already scrolled), clipped
/// to `bounds`.
pub fn render(
    layer: &mut Layer,
    cache: &MdLayout,
    theme: &Theme,
    x: f32,
    top: f32,
    bounds: TextBounds,
    scroll: &std::collections::HashMap<usize, f32>,
) {
    let (bt, bb) = (bounds.top as f32, bounds.bottom as f32);
    for (i, laid) in cache.blocks.iter().enumerate() {
        let (y0, y1) = (top + laid.top, top + laid.top + laid.h);
        if y1 < bt || y0 > bb {
            continue;
        }
        for d in &laid.draws {
            match d {
                Draw::Rect { r, color, radius } => {
                    let ry = y0 + r[1];
                    let (a, b) = (ry.max(bt), (ry + r[3]).min(bb));
                    if b > a {
                        layer.rect([x + r[0], a], [r[2], b - a], *color, *radius);
                    }
                }
                Draw::Text {
                    buf,
                    x: dx,
                    y: dy,
                    clip_w,
                } => {
                    // code blocks slide under their own clip
                    let sx = scroll.get(&i).copied().unwrap_or(0.0).min(laid.overflow);
                    let b = match clip_w {
                        Some(cw) => TextBounds {
                            left: bounds.left.max((x + dx) as i32),
                            right: bounds.right.min((x + dx + cw) as i32),
                            ..bounds
                        },
                        None => bounds,
                    };
                    layer.text_clipped(buf.clone(), x + dx - sx, y0 + dy, theme.fg, b);
                }
            }
        }
    }
}
