use runway_core::{Color, Theme};
use runway_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use runway_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::views::tabbar::TAB_BAR_H;

/// ⌃⇧F search overlay state.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// (abs line, char col) anchors, oldest first.
    pub matches: Vec<(u64, usize)>,
    pub current: usize,
    /// "Aa" toggle: match case exactly (⌥C).
    pub case_sensitive: bool,
    /// ".*" toggle: the query is a regular expression (⌥R).
    pub regex: bool,
    /// Why the regex does not compile (shown instead of the counter).
    pub error: Option<String>,
}

impl SearchState {
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.matches.clear();
        self.current = 0;
        self.error = None;
    }

    /// Compile the query with the current toggles (records a regex error).
    pub fn matcher(&mut self) -> Option<crate::views::terminal::Matcher> {
        match crate::views::terminal::Matcher::new(&self.query, self.case_sensitive, self.regex) {
            Ok(m) => {
                self.error = None;
                m
            }
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    pub fn next(&mut self) {
        if !self.matches.is_empty() {
            self.current = (self.current + 1) % self.matches.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.matches.is_empty() {
            self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        }
    }

    pub fn current_anchor(&self) -> Option<(u64, usize)> {
        self.matches.get(self.current).copied()
    }
}

/// Counter text for find mode (`3/12`, `0 results`, placeholder).
pub fn counter(state: &SearchState) -> String {
    if state.error.is_some() {
        return "invalid regex".to_string();
    }
    if state.matches.is_empty() {
        if state.query.is_empty() {
            "search".to_string()
        } else {
            "0 results".to_string()
        }
    } else {
        format!("{}/{}", state.current + 1, state.matches.len())
    }
}

/// Floating search bar, top-right (spec: search overlay, match colors §1).
/// Also hosts the block-filter input (`label` = "filter", accent ring).
/// `toggles` = (case-sensitive, regex) for find mode; returns the two
/// toggle chip rects (empty in filter mode).
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    query: &str,
    counter: &str,
    label: Option<&str>,
    w: f32,
    blink_on: bool,
    toggles: Option<(bool, bool)>,
) -> Vec<[f32; 4]> {
    let bar_w = if toggles.is_some() { 400.0 } else { 340.0 };
    let bar_h = 44.0;
    let x = w - bar_w - 16.0;
    let y = TAB_BAR_H + 10.0;

    layer.shadow(
        [x, y + 6.0],
        [bar_w, bar_h],
        Color::hex("#000000").unwrap().with_alpha(0.5),
        10.0,
        12.0,
    );
    let edge = if label.is_some() {
        theme.warning.with_alpha(0.55)
    } else {
        Color::rgba(1.0, 1.0, 1.0, 0.11)
    };
    layer.bordered([x, y], [bar_w, bar_h], theme.surface, edge, 10.0, 1.0);

    // magnifier, or the mode label for the block filter
    let mut qx = x + 40.0;
    if let Some(label) = label {
        let lw = engine.measure(label, 11.0, FONT_SANS, Weight::SEMIBOLD);
        layer.rect(
            [x + 10.0, y + 13.0],
            [lw + 12.0, 18.0],
            theme.warning.with_alpha(0.15),
            5.0,
        );
        let mut lb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 15.0));
        lb.set_text(
            &mut engine.font_system,
            label,
            &Attrs::new()
                .family(Family::Name(FONT_SANS))
                .weight(Weight::SEMIBOLD),
            Shaping::Advanced,
            None,
        );
        layer.text(lb, x + 16.0, y + 15.0, theme.warning);
        qx = x + 10.0 + lw + 12.0 + 10.0;
    } else {
        layer.bordered(
            [x + 14.0, y + 15.0],
            [11.0, 11.0],
            Color::TRANSPARENT,
            theme.text_dim,
            5.5,
            1.6,
        );
        layer.rect([x + 23.5, y + 24.5], [5.0, 1.7], theme.text_dim, 1.0);
    }

    // query + caret
    if !query.is_empty() {
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(13.5, 18.0));
        buf.set_text(
            &mut engine.font_system,
            query,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text(buf, qx, y + 13.0, theme.text_bright);
    }
    if blink_on {
        let qw = engine.measure(query, 13.5, mono_family(), Weight::NORMAL);
        layer.rect([qx + qw + 1.0, y + 12.0], [2.0, 18.0], theme.accent, 0.0);
    }

    // toggle chips: Aa (match case) and .* (regex), right edge
    let mut rects = Vec::new();
    let mut right = x + bar_w - 12.0;
    if let Some((case, re)) = toggles {
        for (text, on) in [(".*", re), ("Aa", case)] {
            let cw = 26.0;
            let cx = right - cw;
            let r = [cx, y + 11.0, cw, 22.0];
            if on {
                layer.rect(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    theme.accent.with_alpha(0.22),
                    5.0,
                );
            }
            let tw = engine.measure(text, 12.0, mono_family(), Weight::NORMAL);
            let mut b = Buffer::new(&mut engine.font_system, Metrics::new(12.0, 16.0));
            b.set_text(
                &mut engine.font_system,
                text,
                &Attrs::new().family(Family::Name(mono_family())),
                Shaping::Advanced,
                None,
            );
            layer.text(
                b,
                cx + (cw - tw) / 2.0,
                y + 14.0,
                if on { theme.accent } else { theme.text_muted },
            );
            rects.push(r);
            right = cx - 4.0;
        }
        rects.reverse(); // [case, regex]
        right -= 6.0;
    } else {
        right -= 4.0;
    }

    // counter
    let cw = engine.measure(counter, 11.5, FONT_SANS, Weight::NORMAL);
    let mut cbuf = Buffer::new(&mut engine.font_system, Metrics::new(11.5, 15.0));
    cbuf.set_text(
        &mut engine.font_system,
        counter,
        &Attrs::new().family(Family::Name(FONT_SANS)),
        Shaping::Advanced,
        None,
    );
    let error = counter == "invalid regex";
    layer.text(
        cbuf,
        right - cw,
        y + 15.0,
        if error { theme.error } else { theme.text_muted },
    );
    rects
}
