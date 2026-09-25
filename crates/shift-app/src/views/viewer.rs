//! Read-only file viewer: opens from the explorer or the review panel over
//! the active tab's pane area. Line numbers, lightweight highlighting
//! (`shift_core::code_highlight`), only the lines on screen are shaped.
//! Esc or ✕ closes it; the terminal underneath keeps running.

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

#[derive(Default)]
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

    pub fn open(&mut self, path: &Path) {
        *self = ViewerState {
            path: Some(path.to_path_buf()),
            ..ViewerState::default()
        };
        match std::fs::metadata(path) {
            Err(e) => self.error = Some(e.to_string()),
            Ok(m) if m.len() > MAX_BYTES => {
                self.error = Some(format!(
                    "Too large to preview ({:.1} MB)",
                    m.len() as f64 / 1_048_576.0
                ))
            }
            Ok(_) => match std::fs::read(path) {
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

    let body_top = y + HEADER_H + 1.0;
    state.view_h = h - HEADER_H - 1.0;
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
    for i in first..(first + count).min(state.lines.len()) {
        let ly = body_top + 12.0 + i as f32 * LINE - state.scroll;
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
        let mut rich: Vec<(&str, Attrs)> = Vec::new();
        let base = Attrs::new()
            .family(Family::Name(mono))
            .color(text_color(theme.fg));
        let mut at = 0;
        for (r, k) in &spans {
            if r.start > at {
                rich.push((&line[at..r.start], base.clone()));
            }
            rich.push((
                &line[r.clone()],
                base.clone().color(text_color(color_of(*k))),
            ));
            at = r.end;
        }
        if at < line.len() {
            rich.push((&line[at..], base.clone()));
        }
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
