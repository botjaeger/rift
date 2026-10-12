//! The one icon set every surface draws from: 16-unit stroked outlines
//! (1.5 px at 16 px, round caps, the caller's color), drawn as line and
//! outline quads so they sit on the pixel grid in every font. Coordinates
//! below are the design's SVG paths (`viewBox 0 0 16 16`).

use shift_core::Color;
use shift_renderer::Layer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Terminal,
    Sidebar,
    SplitRight,
    SplitDown,
    Plus,
    Minus,
    Close,
    More,
    Grip,
    Search,
    History,
    Folder,
    File,
    Inbox,
    Settings,
    Branch,
    Copy,
    Check,
    Refresh,
    Revert,
    Open,
    ChevronRight,
    ChevronDown,
    ChevronUp,
    Sync,
    Maximize,
    Restore,
    Keyboard,
    Bookmark,
    Filter,
    Appearance,
    Themes,
    Launch,
    Info,
}

/// Draws `icon` centred on `c` at `size` px (the 16-unit box scaled).
pub fn draw(layer: &mut Layer, icon: Icon, c: [f32; 2], size: f32, color: Color) {
    let s = size / 16.0;
    let o = [c[0] - 8.0 * s, c[1] - 8.0 * s];
    let p = |x: f32, y: f32| [o[0] + x * s, o[1] + y * s];
    let w = (1.5 * s).max(1.2);
    let poly = |layer: &mut Layer, pts: &[(f32, f32)]| {
        for pair in pts.windows(2) {
            layer.line(p(pair[0].0, pair[0].1), p(pair[1].0, pair[1].1), w, color);
        }
    };
    let rect = |layer: &mut Layer, x: f32, y: f32, rw: f32, rh: f32, r: f32| {
        layer.bordered(
            p(x, y),
            [rw * s, rh * s],
            Color::TRANSPARENT,
            color,
            r * s,
            w,
        );
    };
    let ring = |layer: &mut Layer, x: f32, y: f32, r: f32| {
        let d = 2.0 * r * s;
        layer.bordered(
            [p(x, y)[0] - d / 2.0, p(x, y)[1] - d / 2.0],
            [d, d],
            Color::TRANSPARENT,
            color,
            d / 2.0,
            w,
        );
    };
    let dot = |layer: &mut Layer, x: f32, y: f32, d: f32| layer.circle(p(x, y), d * s, color);
    // arc around (cx, cy) from a0 to a1 degrees (y down, so +90 is below)
    let arc = |layer: &mut Layer, cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let steps = (((a1 - a0).abs() / 20.0).ceil() as usize).max(2);
        let pt = |a: f32| {
            let t = a.to_radians();
            (cx + r * t.cos(), cy + r * t.sin())
        };
        let mut prev = pt(a0);
        for i in 1..=steps {
            let next = pt(a0 + (a1 - a0) * i as f32 / steps as f32);
            layer.line(p(prev.0, prev.1), p(next.0, next.1), w, color);
            prev = next;
        }
    };
    match icon {
        Icon::Terminal => {
            poly(layer, &[(4.0, 5.0), (7.0, 8.0), (4.0, 11.0)]);
            poly(layer, &[(8.5, 11.5), (12.5, 11.5)]);
        }
        Icon::Sidebar => {
            rect(layer, 2.0, 3.0, 12.0, 10.0, 1.5);
            poly(layer, &[(6.0, 3.0), (6.0, 13.0)]);
        }
        Icon::SplitRight => {
            rect(layer, 2.0, 3.0, 12.0, 10.0, 1.5);
            poly(layer, &[(8.0, 3.0), (8.0, 13.0)]);
        }
        Icon::SplitDown => {
            rect(layer, 2.0, 3.0, 12.0, 10.0, 1.5);
            poly(layer, &[(2.0, 8.0), (14.0, 8.0)]);
        }
        Icon::Plus => {
            poly(layer, &[(8.0, 3.0), (8.0, 13.0)]);
            poly(layer, &[(3.0, 8.0), (13.0, 8.0)]);
        }
        Icon::Minus => poly(layer, &[(3.0, 8.0), (13.0, 8.0)]),
        Icon::Close => {
            poly(layer, &[(4.0, 4.0), (12.0, 12.0)]);
            poly(layer, &[(12.0, 4.0), (4.0, 12.0)]);
        }
        Icon::More => {
            for y in [3.5, 8.0, 12.5] {
                dot(layer, 8.0, y, 2.4);
            }
        }
        Icon::Grip => {
            for y in [4.0, 8.0, 12.0] {
                dot(layer, 6.0, y, 2.2);
                dot(layer, 10.0, y, 2.2);
            }
        }
        Icon::Search => {
            ring(layer, 7.0, 7.0, 4.5);
            poly(layer, &[(10.5, 10.5), (14.0, 14.0)]);
        }
        Icon::History => {
            arc(layer, 8.0, 8.0, 5.2, 180.0, -140.0);
            poly(layer, &[(2.8, 3.0), (2.8, 6.0), (5.8, 6.0)]);
            poly(layer, &[(8.0, 5.5), (8.0, 8.0), (10.0, 9.5)]);
        }
        Icon::Revert => {
            arc(layer, 8.0, 8.0, 5.0, 180.0, -140.0);
            poly(layer, &[(3.0, 3.0), (3.0, 6.0), (6.0, 6.0)]);
        }
        Icon::Refresh => {
            arc(layer, 8.0, 8.0, 5.0, 0.0, 316.0);
            poly(layer, &[(13.0, 2.5), (13.0, 5.5), (10.0, 5.5)]);
        }
        Icon::Folder => poly(
            layer,
            &[
                (2.0, 4.5),
                (6.0, 4.5),
                (7.5, 6.0),
                (14.0, 6.0),
                (14.0, 12.5),
                (2.0, 12.5),
                (2.0, 4.5),
            ],
        ),
        Icon::File => {
            poly(
                layer,
                &[
                    (4.0, 2.0),
                    (9.0, 2.0),
                    (12.0, 5.0),
                    (12.0, 14.0),
                    (4.0, 14.0),
                    (4.0, 2.0),
                ],
            );
            poly(layer, &[(9.0, 2.0), (9.0, 5.0), (12.0, 5.0)]);
        }
        Icon::Inbox => {
            poly(layer, &[(4.0, 11.0), (4.0, 7.0)]);
            arc(layer, 8.0, 7.0, 4.0, 180.0, 360.0);
            poly(
                layer,
                &[
                    (12.0, 7.0),
                    (12.0, 11.0),
                    (13.0, 12.5),
                    (3.0, 12.5),
                    (4.0, 11.0),
                ],
            );
            poly(layer, &[(6.5, 14.0), (9.5, 14.0)]);
        }
        Icon::Settings => {
            poly(layer, &[(2.5, 5.0), (4.3, 5.0)]);
            poly(layer, &[(7.7, 5.0), (13.5, 5.0)]);
            ring(layer, 6.0, 5.0, 1.7);
            poly(layer, &[(2.5, 11.0), (8.8, 11.0)]);
            poly(layer, &[(12.2, 11.0), (13.5, 11.0)]);
            ring(layer, 10.5, 11.0, 1.7);
        }
        Icon::Branch => {
            ring(layer, 5.0, 4.0, 1.5);
            ring(layer, 5.0, 12.0, 1.5);
            ring(layer, 11.0, 5.5, 1.5);
            poly(layer, &[(5.0, 5.5), (5.0, 10.5)]);
            poly(layer, &[(11.0, 7.0), (10.5, 8.6), (5.0, 10.5)]);
        }
        Icon::Copy => {
            rect(layer, 5.0, 5.0, 8.0, 8.0, 1.5);
            poly(layer, &[(3.0, 11.0), (3.0, 3.0), (11.0, 3.0)]);
        }
        Icon::Check => poly(layer, &[(3.5, 8.5), (6.5, 11.5), (12.5, 4.5)]),
        Icon::Open => {
            poly(layer, &[(5.0, 11.0), (11.0, 5.0)]);
            poly(layer, &[(6.0, 5.0), (11.0, 5.0), (11.0, 10.0)]);
        }
        Icon::ChevronRight => poly(layer, &[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
        Icon::ChevronDown => poly(layer, &[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Icon::ChevronUp => poly(layer, &[(4.0, 10.0), (8.0, 6.0), (12.0, 10.0)]),
        Icon::Sync => {
            poly(layer, &[(3.0, 6.0), (12.0, 6.0), (10.0, 4.0)]);
            poly(layer, &[(13.0, 10.0), (4.0, 10.0), (6.0, 12.0)]);
        }
        Icon::Maximize => {
            poly(layer, &[(3.0, 6.0), (3.0, 3.0), (6.0, 3.0)]);
            poly(layer, &[(13.0, 10.0), (13.0, 13.0), (10.0, 13.0)]);
            poly(layer, &[(3.0, 3.0), (7.0, 7.0)]);
            poly(layer, &[(13.0, 13.0), (9.0, 9.0)]);
        }
        Icon::Restore => {
            poly(layer, &[(3.0, 7.0), (7.0, 7.0), (7.0, 3.0)]);
            poly(layer, &[(13.0, 9.0), (9.0, 9.0), (9.0, 13.0)]);
            poly(layer, &[(7.0, 7.0), (3.0, 3.0)]);
            poly(layer, &[(9.0, 9.0), (13.0, 13.0)]);
        }
        Icon::Keyboard => {
            rect(layer, 1.5, 4.0, 13.0, 8.0, 1.5);
            for x in [4.5, 7.5, 10.5] {
                dot(layer, x, 7.0, 1.4);
            }
            poly(layer, &[(5.0, 9.5), (11.0, 9.5)]);
        }
        Icon::Bookmark => poly(
            layer,
            &[
                (4.5, 2.5),
                (11.5, 2.5),
                (11.5, 13.5),
                (8.0, 11.0),
                (4.5, 13.5),
                (4.5, 2.5),
            ],
        ),
        Icon::Filter => {
            poly(layer, &[(2.5, 4.0), (13.5, 4.0)]);
            poly(layer, &[(4.5, 8.0), (11.5, 8.0)]);
            poly(layer, &[(6.5, 12.0), (9.5, 12.0)]);
        }
        Icon::Appearance => {
            ring(layer, 8.0, 8.0, 5.5);
            // the right half filled: light and dark
            for i in 0..5 {
                let x = 8.6 + i as f32;
                let half = (5.5f32.powi(2) - (x - 8.0).powi(2)).max(0.0).sqrt() - 0.6;
                poly(layer, &[(x, 8.0 - half), (x, 8.0 + half)]);
            }
        }
        Icon::Themes => {
            for (x, y) in [(2.0, 2.0), (9.0, 2.0), (2.0, 9.0), (9.0, 9.0)] {
                rect(layer, x, y, 5.0, 5.0, 1.0);
            }
        }
        Icon::Launch => {
            rect(layer, 2.0, 2.5, 5.0, 11.0, 1.0);
            rect(layer, 9.0, 2.5, 5.0, 5.0, 1.0);
            rect(layer, 9.0, 9.5, 5.0, 4.0, 1.0);
        }
        Icon::Info => {
            ring(layer, 8.0, 8.0, 6.0);
            dot(layer, 8.0, 5.2, 1.6);
            poly(layer, &[(8.0, 7.5), (8.0, 11.5)]);
        }
    }
}
