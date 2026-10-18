//! Box drawing (U+2500–U+257F) and Powerline separators (U+E0B0–U+E0BF)
//! drawn as geometry instead of font glyphs, like the block elements in
//! `terminal.rs`: the font's glyphs stop short of the line box (the line
//! height is 1.2× the font size), so frames and prompt segments showed gaps
//! between rows. Lines sit on device pixels so neighbouring cells meet.

/// One piece of a cell's ink, in logical px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A filled rect: position and size.
    Rect([f32; 2], [f32; 2]),
    /// A round-capped stroke from one point to another, this wide.
    Line([f32; 2], [f32; 2], f32),
}

/// Arm weights of a line character: up, right, down, left.
const NONE: u8 = 0;
const LIGHT: u8 = 1;
const HEAVY: u8 = 2;
const DOUBLE: u8 = 3;

/// The arms of U+2500–U+257F that are made of straight lines (not dashes,
/// arcs or diagonals), as `[up, right, down, left]`.
fn arms(c: u32) -> Option<[u8; 4]> {
    const L: u8 = LIGHT;
    const H: u8 = HEAVY;
    const D: u8 = DOUBLE;
    const N: u8 = NONE;
    Some(match c {
        0x2500 => [N, L, N, L],
        0x2501 => [N, H, N, H],
        0x2502 => [L, N, L, N],
        0x2503 => [H, N, H, N],
        0x250C => [N, L, L, N],
        0x250D => [N, H, L, N],
        0x250E => [N, L, H, N],
        0x250F => [N, H, H, N],
        0x2510 => [N, N, L, L],
        0x2511 => [N, N, L, H],
        0x2512 => [N, N, H, L],
        0x2513 => [N, N, H, H],
        0x2514 => [L, L, N, N],
        0x2515 => [L, H, N, N],
        0x2516 => [H, L, N, N],
        0x2517 => [H, H, N, N],
        0x2518 => [L, N, N, L],
        0x2519 => [L, N, N, H],
        0x251A => [H, N, N, L],
        0x251B => [H, N, N, H],
        0x251C => [L, L, L, N],
        0x251D => [L, H, L, N],
        0x251E => [H, L, L, N],
        0x251F => [L, L, H, N],
        0x2520 => [H, L, H, N],
        0x2521 => [H, H, L, N],
        0x2522 => [L, H, H, N],
        0x2523 => [H, H, H, N],
        0x2524 => [L, N, L, L],
        0x2525 => [L, N, L, H],
        0x2526 => [H, N, L, L],
        0x2527 => [L, N, H, L],
        0x2528 => [H, N, H, L],
        0x2529 => [H, N, L, H],
        0x252A => [L, N, H, H],
        0x252B => [H, N, H, H],
        0x252C => [N, L, L, L],
        0x252D => [N, L, L, H],
        0x252E => [N, H, L, L],
        0x252F => [N, H, L, H],
        0x2530 => [N, L, H, L],
        0x2531 => [N, L, H, H],
        0x2532 => [N, H, H, L],
        0x2533 => [N, H, H, H],
        0x2534 => [L, L, N, L],
        0x2535 => [L, L, N, H],
        0x2536 => [L, H, N, L],
        0x2537 => [L, H, N, H],
        0x2538 => [H, L, N, L],
        0x2539 => [H, L, N, H],
        0x253A => [H, H, N, L],
        0x253B => [H, H, N, H],
        0x253C => [L, L, L, L],
        0x253D => [L, L, L, H],
        0x253E => [L, H, L, L],
        0x253F => [L, H, L, H],
        0x2540 => [H, L, L, L],
        0x2541 => [L, L, H, L],
        0x2542 => [H, L, H, L],
        0x2543 => [H, L, L, H],
        0x2544 => [H, H, L, L],
        0x2545 => [L, L, H, H],
        0x2546 => [L, H, H, L],
        0x2547 => [H, H, L, H],
        0x2548 => [L, H, H, H],
        0x2549 => [H, L, H, H],
        0x254A => [H, H, H, L],
        0x254B => [H, H, H, H],
        0x2550 => [N, D, N, D],
        0x2551 => [D, N, D, N],
        0x2552 => [N, D, L, N],
        0x2553 => [N, L, D, N],
        0x2554 => [N, D, D, N],
        0x2555 => [N, N, L, D],
        0x2556 => [N, N, D, L],
        0x2557 => [N, N, D, D],
        0x2558 => [L, D, N, N],
        0x2559 => [D, L, N, N],
        0x255A => [D, D, N, N],
        0x255B => [L, N, N, D],
        0x255C => [D, N, N, L],
        0x255D => [D, N, N, D],
        0x255E => [L, D, L, N],
        0x255F => [D, L, D, N],
        0x2560 => [D, D, D, N],
        0x2561 => [L, N, L, D],
        0x2562 => [D, N, D, L],
        0x2563 => [D, N, D, D],
        0x2564 => [N, D, L, D],
        0x2565 => [N, L, D, L],
        0x2566 => [N, D, D, D],
        0x2567 => [L, D, N, D],
        0x2568 => [D, L, N, L],
        0x2569 => [D, D, N, D],
        0x256A => [L, D, L, D],
        0x256B => [D, L, D, L],
        0x256C => [D, D, D, D],
        0x2574 => [N, N, N, L],
        0x2575 => [L, N, N, N],
        0x2576 => [N, L, N, N],
        0x2577 => [N, N, L, N],
        0x2578 => [N, N, N, H],
        0x2579 => [H, N, N, N],
        0x257A => [N, H, N, N],
        0x257B => [N, N, H, N],
        0x257C => [N, H, N, L],
        0x257D => [L, N, H, N],
        0x257E => [N, L, N, H],
        0x257F => [H, N, L, N],
        _ => return None,
    })
}

/// Pixel geometry of one cell.
struct Cell {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    /// Device pixels per logical px.
    s: f32,
    /// Light and heavy line thickness, whole device pixels.
    light: f32,
    heavy: f32,
}

impl Cell {
    fn new(cell: [f32; 4], font_size: f32, scale: f32) -> Self {
        let s = scale.max(1.0);
        let snap = |v: f32| (v * s).round() / s;
        // one device pixel per 13 px of font at 1×, a logical px at 2×
        let light_px = (font_size / 13.0 * s).round().max(1.0);
        Cell {
            x0: snap(cell[0]),
            y0: snap(cell[1]),
            x1: snap(cell[0] + cell[2]),
            y1: snap(cell[1] + cell[3]),
            s,
            light: light_px / s,
            heavy: light_px * 2.0 / s,
        }
    }

    fn snap(&self, v: f32) -> f32 {
        (v * self.s).round() / self.s
    }

    fn w(&self) -> f32 {
        self.x1 - self.x0
    }

    fn h(&self) -> f32 {
        self.y1 - self.y0
    }

    /// Start of a line `t` thick centred on `c` (on a device pixel).
    fn line_start(&self, c: f32, t: f32) -> f32 {
        self.snap(c - t / 2.0)
    }

    /// Centre of the cell's vertical and horizontal lines (light ones).
    fn centre(&self) -> (f32, f32) {
        let cx = self.line_start((self.x0 + self.x1) / 2.0, self.light) + self.light / 2.0;
        let cy = self.line_start((self.y0 + self.y1) / 2.0, self.light) + self.light / 2.0;
        (cx, cy)
    }

    fn thickness(&self, weight: u8) -> f32 {
        match weight {
            HEAVY => self.heavy,
            NONE => 0.0,
            _ => self.light,
        }
    }
}

/// Rect `[x0, y0, x1, y1]` as a [`Shape::Rect`].
fn rect(r: [f32; 4]) -> Shape {
    Shape::Rect([r[0], r[1]], [r[2] - r[0], r[3] - r[1]])
}

/// `a` minus `b`: up to four rects covering what of `a` lies outside `b`.
fn subtract(a: [f32; 4], b: [f32; 4]) -> Vec<[f32; 4]> {
    let [ax0, ay0, ax1, ay1] = a;
    let (ix0, iy0, ix1, iy1) = (ax0.max(b[0]), ay0.max(b[1]), ax1.min(b[2]), ay1.min(b[3]));
    if ix0 >= ix1 || iy0 >= iy1 {
        return vec![a];
    }
    [
        [ax0, ay0, ax1, iy0],
        [ax0, iy1, ax1, ay1],
        [ax0, iy0, ix0, iy1],
        [ix1, iy0, ax1, iy1],
    ]
    .into_iter()
    .filter(|r| r[0] < r[2] && r[1] < r[3])
    .collect()
}

/// The arms as rects that do not overlap (translucent ink, such as dim
/// text, would darken where two overlapped).
fn line_rects(c: &Cell, arms: [u8; 4]) -> Vec<[f32; 4]> {
    let [up, right, down, left] = arms;
    let (cx, cy) = c.centre();
    // the gap between a double line's two strokes is one light line
    let o = c.light;
    let lt = c.light;
    let mut out: Vec<[f32; 4]> = Vec::new();
    // how far past the centre an arm reaches toward its opposite edge:
    // `perp` are the arms across it (the minus side first), `through`
    // whether the opposite arm continues the same line
    let reach = |perp: (u8, u8), through: bool| -> f32 {
        let doubles = (perp.0 == DOUBLE) as u8 + (perp.1 == DOUBLE) as u8;
        match doubles {
            // a stem ends on the near stroke of a double line it meets (╤)
            2 if !through => lt / 2.0 - o,
            // a corner reaches the far stroke (╒); a crossing runs over both
            1 | 2 => o + lt / 2.0,
            _ => c.thickness(perp.0).max(c.thickness(perp.1)) / 2.0,
        }
    };
    // a double arm's stroke on side `same` of its line (the other side is
    // `other`): meets a double arm on its side at the inner corner, one on
    // the other side at the outer corner
    let double_reach = |same: u8, other: u8| -> f32 {
        if same == DOUBLE {
            lt / 2.0 - o
        } else if other == DOUBLE {
            o + lt / 2.0
        } else {
            c.thickness(same).max(c.thickness(other)) / 2.0
        }
    };
    // horizontal arms: right then left
    for (weight, toward_right) in [(right, true), (left, false)] {
        if weight == NONE {
            continue;
        }
        let span = |ext: f32| {
            if toward_right {
                (c.snap(cx - ext), c.x1)
            } else {
                (c.x0, c.snap(cx + ext))
            }
        };
        if weight == DOUBLE {
            for (side_y, same, other) in [(cy - o, up, down), (cy + o, down, up)] {
                let (a, b) = span(double_reach(same, other));
                let y = c.line_start(side_y, lt);
                out.push([a, y, b, y + lt]);
            }
        } else {
            let t = c.thickness(weight);
            let through = if toward_right { left } else { right } == weight;
            let (a, b) = span(reach((up, down), through));
            let y = c.line_start(cy, t);
            out.push([a, y, b, y + t]);
        }
    }
    // vertical arms: down then up
    for (weight, toward_down) in [(down, true), (up, false)] {
        if weight == NONE {
            continue;
        }
        let span = |ext: f32| {
            if toward_down {
                (c.snap(cy - ext), c.y1)
            } else {
                (c.y0, c.snap(cy + ext))
            }
        };
        if weight == DOUBLE {
            for (side_x, same, other) in [(cx - o, left, right), (cx + o, right, left)] {
                let (a, b) = span(double_reach(same, other));
                let x = c.line_start(side_x, lt);
                out.push([x, a, x + lt, b]);
            }
        } else {
            let t = c.thickness(weight);
            let through = if toward_down { up } else { down } == weight;
            let (a, b) = span(reach((left, right), through));
            let x = c.line_start(cx, t);
            out.push([x, a, x + t, b]);
        }
    }
    // keep each new rect's part outside the earlier ones
    let mut disjoint: Vec<[f32; 4]> = Vec::new();
    for r in out {
        let mut parts = vec![r];
        for d in &disjoint {
            parts = parts.into_iter().flat_map(|p| subtract(p, *d)).collect();
        }
        disjoint.extend(parts);
    }
    disjoint
}

/// Dashed lines: `n` dashes across the cell, horizontal or vertical.
fn dashes(c: &Cell, n: usize, weight: u8, horizontal: bool) -> Vec<Shape> {
    let (cx, cy) = c.centre();
    let t = c.thickness(weight);
    let (start, len) = if horizontal {
        (c.x0, c.w())
    } else {
        (c.y0, c.h())
    };
    let seg = len / n as f32;
    let px = 1.0 / c.s;
    (0..n)
        .map(|i| {
            // each dash's share of the cell, then a gap of whole device
            // pixels (at least one) split around it: a dash never reaches
            // the cell's far edge, so dashes stay apart across cells too
            let (from, to) = (
                c.snap(start + i as f32 * seg),
                c.snap(start + (i + 1) as f32 * seg),
            );
            let gap = (((to - from) * 0.35 * c.s).round() * px).max(px);
            let before = ((gap * c.s) / 2.0).floor() * px;
            let a = from + before;
            let b = (to - (gap - before)).max(a + px);
            if horizontal {
                let y = c.line_start(cy, t);
                rect([a, y, b, y + t])
            } else {
                let x = c.line_start(cx, t);
                rect([x, a, x + t, b])
            }
        })
        .collect()
}

/// A quarter ellipse as strokes: centre, radii, from angle `a0` to `a1`
/// (radians, y down).
fn arc(c: [f32; 2], r: [f32; 2], a0: f32, a1: f32, width: f32) -> Vec<Shape> {
    const STEPS: usize = 8;
    let at = |i: usize| {
        let a = a0 + (a1 - a0) * i as f32 / STEPS as f32;
        [c[0] + r[0] * a.cos(), c[1] + r[1] * a.sin()]
    };
    (0..STEPS)
        .map(|i| Shape::Line(at(i), at(i + 1), width))
        .collect()
}

/// A filled shape as one rect per device-pixel row: `span(v)` is the
/// covered `[from, to]` fraction of the cell's width at height fraction `v`
/// (the row's middle). The rects' fractional ends antialias the slope.
fn rows(c: &Cell, span: impl Fn(f32) -> Option<(f32, f32)>) -> Vec<Shape> {
    let n = (c.h() * c.s).round().max(1.0) as usize;
    let row_h = c.h() / n as f32;
    (0..n)
        .filter_map(|i| {
            let (a, b) = span((i as f32 + 0.5) / n as f32)?;
            let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
            (b > a).then(|| {
                let y = c.y0 + i as f32 * row_h;
                rect([c.x0 + a * c.w(), y, c.x0 + b * c.w(), y + row_h])
            })
        })
        .collect()
}

/// Powerline separators U+E0B0–U+E0BF.
fn powerline(c: &Cell, cp: u32) -> Option<Vec<Shape>> {
    use std::f32::consts::{FRAC_PI_2, PI};
    let (x0, y0, x1, y1) = (c.x0, c.y0, c.x1, c.y1);
    let my = (y0 + y1) / 2.0;
    let lt = c.light;
    let half_ellipse = |v: f32| (1.0 - (2.0 * v - 1.0).powi(2)).max(0.0).sqrt();
    Some(match cp {
        // solid arrows: right-pointing, left-pointing
        0xE0B0 => rows(c, |v| Some((0.0, 1.0 - (2.0 * v - 1.0).abs()))),
        0xE0B2 => rows(c, |v| Some(((2.0 * v - 1.0).abs(), 1.0))),
        // thin arrows
        0xE0B1 => vec![
            Shape::Line([x0, y0], [x1, my], lt),
            Shape::Line([x1, my], [x0, y1], lt),
        ],
        0xE0B3 => vec![
            Shape::Line([x1, y0], [x0, my], lt),
            Shape::Line([x0, my], [x1, y1], lt),
        ],
        // half circles: solid and thin, right then left
        0xE0B4 => rows(c, |v| Some((0.0, half_ellipse(v)))),
        0xE0B6 => rows(c, |v| Some((1.0 - half_ellipse(v), 1.0))),
        0xE0B5 => {
            let r = [c.w() - lt / 2.0, c.h() / 2.0 - lt / 2.0];
            let mut v = arc([x0, my], r, -FRAC_PI_2, 0.0, lt);
            v.extend(arc([x0, my], r, 0.0, FRAC_PI_2, lt));
            v
        }
        0xE0B7 => {
            let r = [c.w() - lt / 2.0, c.h() / 2.0 - lt / 2.0];
            let mut v = arc([x1, my], r, FRAC_PI_2, PI, lt);
            v.extend(arc([x1, my], r, PI, PI + FRAC_PI_2, lt));
            v
        }
        // triangles: lower left, lower right, upper left, upper right
        0xE0B8 => rows(c, |v| Some((0.0, v))),
        0xE0BA => rows(c, |v| Some((1.0 - v, 1.0))),
        0xE0BC => rows(c, |v| Some((0.0, 1.0 - v))),
        0xE0BE => rows(c, |v| Some((v, 1.0))),
        // their thin diagonals
        0xE0B9 | 0xE0BF => vec![Shape::Line([x0, y0], [x1, y1], lt)],
        0xE0BB | 0xE0BD => vec![Shape::Line([x0, y1], [x1, y0], lt)],
        _ => return None,
    })
}

/// The shapes that draw `ch` in the cell `[x, y, w, h]` (logical px), or
/// None when `ch` is not drawn natively. `font_size` sets the line
/// thickness; `scale` is device pixels per logical px. With `seal` (opaque
/// ink), a rect that meets another one or the cell's right or bottom edge
/// reaches one device pixel past it: the quads' antialiased corners left
/// faint dots where two met (translucent ink would darken there instead).
pub fn shapes(
    ch: char,
    cell: [f32; 4],
    font_size: f32,
    scale: f32,
    seal: bool,
) -> Option<Vec<Shape>> {
    let cp = ch as u32;
    if !(0x2500..=0x257F).contains(&cp) && !(0xE0B0..=0xE0BF).contains(&cp) {
        return None;
    }
    let c = Cell::new(cell, font_size, scale);
    let mut v = exact(&c, cp)?;
    if seal {
        seal_seams(&mut v, &c);
    }
    Some(v)
}

/// Extends each rect of `shapes` by a device pixel along its length (a
/// horizontal one sideways, a vertical one up or down) into another rect it
/// meets there, and past the cell's right or bottom edge into the next cell
/// (see [`shapes`]); never across, which would thicken the line.
fn seal_seams(shapes: &mut [Shape], c: &Cell) {
    let px = 1.0 / c.s;
    let near = |a: f32, b: f32| (a - b).abs() < px / 4.0;
    let rects: Vec<[f32; 4]> = shapes
        .iter()
        .filter_map(|s| match *s {
            Shape::Rect(p, z) => Some([p[0], p[1], p[0] + z[0], p[1] + z[1]]),
            Shape::Line(..) => None,
        })
        .collect();
    for s in shapes.iter_mut() {
        let Shape::Rect(p, z) = s else { continue };
        let [x0, y0, x1, y1] = [p[0], p[1], p[0] + z[0], p[1] + z[1]];
        // another rect against this one's left, right, top or bottom edge
        let at_x = |x: f32, side: usize| {
            rects
                .iter()
                .any(|o| near(o[side], x) && o[1] < y1 && o[3] > y0)
        };
        let at_y = |y: f32, side: usize| {
            rects
                .iter()
                .any(|o| near(o[side], y) && o[0] < x1 && o[2] > x0)
        };
        if x1 - x0 >= y1 - y0 {
            if at_x(x0, 2) {
                p[0] -= px;
                z[0] += px;
            }
            if near(x1, c.x1) || at_x(x1, 0) {
                z[0] += px;
            }
        } else {
            if at_y(y0, 3) {
                p[1] -= px;
                z[1] += px;
            }
            if near(y1, c.y1) || at_y(y1, 1) {
                z[1] += px;
            }
        }
    }
}

/// [`shapes`] before sealing.
fn exact(c: &Cell, cp: u32) -> Option<Vec<Shape>> {
    if let Some(a) = arms(cp) {
        return Some(line_rects(c, a).into_iter().map(rect).collect());
    }
    if cp >= 0xE0B0 {
        return powerline(c, cp);
    }
    let (cx, cy) = c.centre();
    let lt = c.light;
    Some(match cp {
        // triple, quadruple and double dashes: light/heavy, horizontal/vertical
        0x2504 => dashes(c, 3, LIGHT, true),
        0x2505 => dashes(c, 3, HEAVY, true),
        0x2506 => dashes(c, 3, LIGHT, false),
        0x2507 => dashes(c, 3, HEAVY, false),
        0x2508 => dashes(c, 4, LIGHT, true),
        0x2509 => dashes(c, 4, HEAVY, true),
        0x250A => dashes(c, 4, LIGHT, false),
        0x250B => dashes(c, 4, HEAVY, false),
        0x254C => dashes(c, 2, LIGHT, true),
        0x254D => dashes(c, 2, HEAVY, true),
        0x254E => dashes(c, 2, LIGHT, false),
        0x254F => dashes(c, 2, HEAVY, false),
        // rounded corners: a quarter circle meeting the edges' centres,
        // straight on along the longer (vertical) side
        0x256D..=0x2570 => {
            use std::f32::consts::{FRAC_PI_2, PI};
            let r = (c.x1 - cx).min(cx - c.x0).min((c.y1 - c.y0) / 2.0);
            // which way the arms go: down or up, right or left
            let (down, right) = match cp {
                0x256D => (true, true),
                0x256E => (true, false),
                0x256F => (false, false),
                _ => (false, true),
            };
            let ox = if right { cx + r } else { cx - r };
            let oy = if down { cy + r } else { cy - r };
            // the arc from the vertical arm's end to the horizontal one's
            let (a0, a1) = match (down, right) {
                (true, true) => (PI, PI + FRAC_PI_2),
                (true, false) => (-FRAC_PI_2, 0.0),
                (false, false) => (0.0, FRAC_PI_2),
                (false, true) => (FRAC_PI_2, PI),
            };
            let mut v = arc([ox, oy], [r, r], a0, a1, lt);
            let x = c.line_start(cx, lt);
            let (va, vb) = if down { (oy, c.y1) } else { (c.y0, oy) };
            if vb > va {
                v.push(rect([x, va, x + lt, vb]));
            }
            let y = c.line_start(cy, lt);
            let (ha, hb) = if right { (ox, c.x1) } else { (c.x0, ox) };
            if hb > ha {
                v.push(rect([ha, y, hb, y + lt]));
            }
            v
        }
        // diagonals
        0x2571 => vec![Shape::Line([c.x0, c.y1], [c.x1, c.y0], lt)],
        0x2572 => vec![Shape::Line([c.x0, c.y0], [c.x1, c.y1], lt)],
        0x2573 => vec![
            Shape::Line([c.x0, c.y1], [c.x1, c.y0], lt),
            Shape::Line([c.x0, c.y0], [c.x1, c.y1], lt),
        ],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CELL: [f32; 4] = [10.0, 20.0, 8.0, 16.0];

    fn rects(ch: char) -> Vec<[f32; 4]> {
        shapes(ch, CELL, 13.0, 1.0, false)
            .unwrap()
            .into_iter()
            .map(|s| match s {
                Shape::Rect(p, z) => [p[0], p[1], p[0] + z[0], p[1] + z[1]],
                Shape::Line(..) => panic!("{ch}: a stroke"),
            })
            .collect()
    }

    fn area(r: &[[f32; 4]]) -> f32 {
        r.iter().map(|r| (r[2] - r[0]) * (r[3] - r[1])).sum()
    }

    fn overlap(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3])
    }

    fn covers(r: &[[f32; 4]], x: f32, y: f32) -> bool {
        r.iter()
            .any(|r| x >= r[0] && x < r[2] && y >= r[1] && y < r[3])
    }

    #[test]
    fn only_box_drawing_and_powerline() {
        for ch in [
            'a', '█', '▀', '中', '\u{24ff}', '\u{2580}', '\u{e0af}', '\u{e0c0}',
        ] {
            assert!(shapes(ch, CELL, 13.0, 1.0, false).is_none(), "{ch}");
        }
        // every character of the two ranges draws something
        for cp in (0x2500..=0x257F).chain(0xE0B0..=0xE0BF) {
            let ch = char::from_u32(cp).unwrap();
            assert!(
                shapes(ch, CELL, 13.0, 2.0, false).is_some_and(|s| !s.is_empty()),
                "U+{cp:04X}"
            );
        }
    }

    #[test]
    fn lines_span_the_whole_cell_on_device_pixels() {
        // ─ reaches both edges at the line height's middle, 1px thick
        let h = rects('─');
        assert_eq!(area(&h), 8.0);
        assert!(h.iter().all(|r| r[1] == 28.0 && r[3] == 29.0), "{h:?}");
        assert_eq!(h.iter().map(|r| r[0]).fold(f32::MAX, f32::min), 10.0);
        assert_eq!(h.iter().map(|r| r[2]).fold(f32::MIN, f32::max), 18.0);
        // │ runs the full line height (no gap between rows)
        let v = rects('│');
        assert_eq!(v.iter().map(|r| r[1]).fold(f32::MAX, f32::min), 20.0);
        assert_eq!(v.iter().map(|r| r[3]).fold(f32::MIN, f32::max), 36.0);
        // heavy is twice as thick
        let heavy = rects('━');
        assert_eq!(heavy[0][3] - heavy[0][1], 2.0);
        // at 2× the light line is one logical px, still on device pixels
        for s in shapes('│', [10.3, 20.1, 7.8, 15.6], 13.0, 2.0, false).unwrap() {
            let Shape::Rect(p, z) = s else { panic!() };
            for v in [p[0], p[1], z[0], z[1]] {
                assert_eq!((v * 2.0).fract(), 0.0, "{v}");
            }
        }
    }

    #[test]
    fn junctions_do_not_overlap() {
        // ┼ ╋ ┿ ╬ ╪ ╫ ┌ ╔: disjoint rects (translucent ink stays even)
        for ch in ['┼', '╋', '┿', '╬', '╪', '╫', '┌', '╔', '╠', '╤'] {
            let r = rects(ch);
            for (i, a) in r.iter().enumerate() {
                for b in &r[i + 1..] {
                    assert!(!overlap(*a, *b), "{ch}: {a:?} {b:?}");
                }
            }
        }
        // ┼: a full row and a full column, sharing one centre pixel
        assert_eq!(area(&rects('┼')), 8.0 + 16.0 - 1.0);
    }

    #[test]
    fn double_corners_and_tees() {
        let (cx, cy) = (14.5, 28.5);
        // ╔: the outer strokes meet at the outer corner, the inner ones at
        // the inner corner; nothing up or left of them
        let r = rects('╔');
        assert!(covers(&r, cx - 1.0, cy - 1.0), "outer corner");
        assert!(covers(&r, cx + 1.0, cy + 1.0), "inner corner");
        assert!(!covers(&r, cx - 2.0, cy), "nothing left of the corner");
        assert!(!covers(&r, cx, cy - 2.0), "nothing above it");
        assert!(!covers(&r, cx, cy), "the gap between the strokes");
        // ╦: the top stroke runs through, the lower one breaks for the stem
        let t = rects('╦');
        assert!(covers(&t, cx, cy - 1.0));
        assert!(!covers(&t, cx, cy + 1.0));
        // ╤: the single stem starts at the lower stroke, not across both
        let s = rects('╤');
        assert!(covers(&s, cx, cy + 2.0));
        assert!(!covers(&s, cx, cy));
        // ╒: its stem reaches the upper stroke (a corner)
        assert!(covers(&rects('╒'), cx, cy - 1.0));
    }

    #[test]
    fn opaque_ink_seals_the_joins() {
        let sealed = |ch: char| -> Vec<[f32; 4]> {
            shapes(ch, CELL, 13.0, 1.0, true)
                .unwrap()
                .into_iter()
                .map(|s| match s {
                    Shape::Rect(p, z) => [p[0], p[1], p[0] + z[0], p[1] + z[1]],
                    Shape::Line(..) => panic!(),
                })
                .collect()
        };
        // ─: the left half reaches over the join, the right half past the
        // cell's edge into the next cell; neither gets thicker
        let h = sealed('─');
        assert!(h.iter().all(|r| r[3] - r[1] == 1.0), "{h:?}");
        assert_eq!(h.iter().map(|r| r[2]).fold(f32::MIN, f32::max), 19.0);
        assert!(h.iter().any(|r| r[0] == 10.0 && r[2] > 15.0), "{h:?}");
        // │ reaches one pixel into the row below
        let v = sealed('│');
        assert_eq!(v.iter().map(|r| r[3]).fold(f32::MIN, f32::max), 37.0);
        // ╴ ends inside the cell: nothing to seal
        assert_eq!(sealed('╴'), rects('╴'));
        // ┐: the arm over the stem reaches no thicker (no pad across it)
        let corner = sealed('┐');
        let flat = rects('┐');
        for (s, f) in corner.iter().zip(&flat) {
            let (sw, sh, fw, fh) = (s[2] - s[0], s[3] - s[1], f[2] - f[0], f[3] - f[1]);
            assert!(sw == fw || sh == fh, "{s:?} {f:?}");
            assert!(if fw >= fh { sh == fh } else { sw == fw }, "{s:?} {f:?}");
        }
        // dashes keep their gaps
        assert_eq!(sealed('┄'), rects('┄'));
    }

    #[test]
    fn powerline_arrows_fill_the_line_height() {
        let s = shapes('\u{e0b0}', CELL, 13.0, 1.0, false).unwrap();
        // one rect per pixel row, from the top of the line box to its bottom
        assert_eq!(s.len(), 16);
        let rows: Vec<[f32; 4]> = s
            .iter()
            .map(|s| match s {
                Shape::Rect(p, z) => [p[0], p[1], p[0] + z[0], p[1] + z[1]],
                Shape::Line(..) => panic!(),
            })
            .collect();
        assert_eq!(rows[0][1], 20.0);
        assert_eq!(rows[15][3], 36.0);
        // starts at the left edge; widest in the middle, a point at the ends
        assert!(rows.iter().all(|r| r[0] == 10.0));
        assert!(rows[7][2] > 17.0 && rows[0][2] < 11.0);
        // the left-pointing one mirrors it
        let l = shapes('\u{e0b2}', CELL, 13.0, 1.0, false).unwrap();
        let Shape::Rect(p, z) = l[0] else { panic!() };
        assert_eq!(p[0] + z[0], 18.0);
        // the thin ones are strokes from corner to the middle of the far edge
        assert_eq!(
            shapes('\u{e0b1}', CELL, 13.0, 1.0, false).unwrap()[0],
            Shape::Line([10.0, 20.0], [18.0, 28.0], 1.0)
        );
    }
}
