//! Nerd Font glyphs: the brand and tool marks on the prompt chips and the
//! notification badges, from the embedded `Symbols Nerd Font` subset
//! (`assets/fonts/SymbolsNerdFont-subset.ttf`), plus the drawn running
//! spinner. A glyph added here must also be added to that subset, or it
//! draws as a missing-glyph box.

use rift_core::Color;
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SYMBOLS, Layer, TextEngine};

pub const FOLDER: char = '\u{f07b}'; // fa-folder
pub const BRANCH: char = '\u{e725}'; // dev-git_branch
pub const DIFF: char = '\u{f440}'; // oct-diff
pub const USER: char = '\u{f007}'; // fa-user
pub const LAPTOP: char = '\u{f109}'; // fa-laptop
pub const PYTHON: char = '\u{e73c}'; // dev-python
pub const PHP: char = '\u{e73d}'; // dev-php
pub const GITHUB: char = '\u{f09b}'; // fa-github
pub const DOCKER: char = '\u{f21f}'; // fa-docker
pub const CALENDAR: char = '\u{f073}'; // fa-calendar_days
pub const CLOCK: char = '\u{f017}'; // fa-clock_o
pub const HOURGLASS: char = '\u{f252}'; // fa-hourglass_half
pub const BELL: char = '\u{f0f3}'; // fa-bell
pub const BELL_OUTLINE: char = '\u{f0a2}'; // fa-bell_o

/// A glyph of this subset by name (a profile's `icon`).
pub fn by_name(name: &str) -> Option<char> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "folder" => FOLDER,
        "git" | "branch" => BRANCH,
        "python" => PYTHON,
        "php" => PHP,
        "github" => GITHUB,
        "docker" => DOCKER,
        "user" => USER,
        "laptop" | "computer" => LAPTOP,
        _ => return None,
    })
}

pub(crate) fn epoch() -> std::time::Instant {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(std::time::Instant::now)
}

/// Seconds on the clock every chrome animation shares (spinners, the
/// waiting agent's pulse), so they move in step.
pub fn anim_secs() -> f32 {
    epoch().elapsed().as_secs_f32()
}

/// One full turn of the running spinner.
const SPIN_PERIOD: f32 = 0.7;

/// The running-command spinner, `size` px across, centred on `c`: an open
/// arc that turns continuously with every frame (the Nerd Font spinner's six
/// frames stepped at 10 fps and read as lag). Every spinner on screen turns
/// in step.
pub fn draw_spinner(layer: &mut Layer, c: [f32; 2], size: f32, color: Color) {
    use std::f32::consts::TAU;
    let r = size * 0.36;
    let width = (size * 0.13).max(1.5);
    let start = (epoch().elapsed().as_secs_f32() / SPIN_PERIOD).fract() * TAU;
    // a third of a turn, as short round-capped chords
    let (sweep, steps) = (TAU / 3.0, 12);
    let point = |a: f32| [c[0] + r * a.cos(), c[1] + r * a.sin()];
    for i in 0..steps {
        let a0 = start + sweep * i as f32 / steps as f32;
        let a1 = start + sweep * (i + 1) as f32 / steps as f32;
        layer.line(point(a0), point(a1), width, color);
    }
}

/// Draw `glyph` `size` px tall, centred on `c`.
pub fn draw(
    layer: &mut Layer,
    engine: &mut TextEngine,
    glyph: char,
    c: [f32; 2],
    size: f32,
    color: Color,
) {
    let text = glyph.to_string();
    let w = engine.measure(&text, size, FONT_SYMBOLS, Weight::NORMAL);
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size));
    buf.set_text(
        &mut engine.font_system,
        &text,
        &Attrs::new()
            .family(Family::Name(FONT_SYMBOLS))
            .weight(Weight::NORMAL),
        Shaping::Advanced,
        None,
    );
    layer.text(buf, c[0] - w / 2.0, c[1] - size / 2.0, color);
}
