use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

/// ⌃⇧F search overlay state.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// (abs line, char col) anchors, oldest first: the newest
    /// [`MATCH_CAP`] matches only.
    pub matches: Vec<(u64, usize)>,
    /// Every match in the pane, including older ones past the cap.
    pub total: usize,
    pub current: usize,
    /// "Aa" toggle: match case exactly (⌥C).
    pub case_sensitive: bool,
    /// ".*" toggle: the query is a regular expression (⌥R).
    pub regex: bool,
    /// Why the regex does not compile (shown instead of the counter).
    pub error: Option<String>,
    /// ⌘A selected the whole query: typing replaces it, Backspace clears it.
    pub all_selected: bool,
    /// Find limited to one block (it was selected when find opened): its
    /// id and command. Deselecting it, or the chip's ×, widens to the pane.
    pub scope: Option<(rift_core::BlockId, String)>,
    /// Matches in output lines a block filter hides (not navigable): said
    /// in the counter, so a find that "finds nothing" is not silent.
    pub hidden: usize,
}

impl SearchState {
    pub fn close(&mut self) {
        self.open = false;
        self.all_selected = false;
        self.query.clear();
        self.matches.clear();
        self.total = 0;
        self.current = 0;
        self.error = None;
        self.scope = None;
        self.hidden = 0;
    }

    /// Store a fresh result; selects the newest match.
    pub fn set_matches(&mut self, found: crate::views::terminal::Found) {
        self.total = found.total;
        self.matches = found.anchors;
        self.current = self.matches.len().saturating_sub(1);
    }

    /// (1-based position among all matches, total). The kept window is
    /// the newest matches, so the oldest kept one is `total - kept + 1`.
    pub fn position(&self) -> Option<(usize, usize)> {
        if self.matches.is_empty() {
            return None;
        }
        let total = self.total.max(self.matches.len());
        Some((total - self.matches.len() + self.current + 1, total))
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

/// Most anchors kept per search (the newest ones; older matches are
/// counted but not navigable).
pub const MATCH_CAP: usize = 500;

/// Counter text for find mode (`3/12`, `0 results`, placeholder). The
/// number is the match's position among ALL matches, so with 600 matches
/// and 500 kept, navigation runs 101/600 ..= 600/600.
pub fn counter(state: &SearchState) -> String {
    if state.error.is_some() {
        return "invalid regex".to_string();
    }
    let shown = match state.position() {
        Some((n, total)) => format!("{n}/{total}"),
        None if state.query.is_empty() => return "search".to_string(),
        None => "0 results".to_string(),
    };
    match state.hidden {
        0 => shown,
        n => format!("{shown} · {n} hidden by the filter"),
    }
}

/// Left shift of the query text so its end (the caret) stays inside a
/// viewport `avail` px wide; `text_w` includes the caret.
pub fn query_scroll(text_w: f32, avail: f32) -> f32 {
    (text_w - avail.max(0.0)).max(0.0)
}

/// Floating search bar, top-right (spec: search overlay, match colors §1).
/// Also hosts the block-filter input (`label` = "filter", accent ring).
/// `toggles` = the chips at its right edge, left to right, as (text, on):
/// find's Aa and .*, the filter's Aa .* ! ±N. `scope` = find limited to a
/// block: an "in block: COMMAND ×" chip after the magnifier. Returns
/// the toggle chip rects in `toggles` order, then the scope chip's.
/// `selected`: the whole query is selected (⌘A).
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    query: &str,
    counter: &str,
    label: Option<&str>,
    w: f32,
    // where the panes start (the tab strip's height, 0 while `tab_bar` hides it)
    top: f32,
    blink_on: bool,
    toggles: &[(&str, bool)],
    selected: bool,
    scope: Option<&str>,
) -> Vec<[f32; 4]> {
    let scope = scope.map(|cmd| {
        let cmd: String = if cmd.chars().count() > 22 {
            cmd.chars().take(21).chain(['…']).collect()
        } else {
            cmd.to_string()
        };
        format!("in block: {cmd}")
    });
    let scope_w = scope.as_deref().map_or(0.0, |s| {
        engine.measure(s, 11.0, FONT_SANS, Weight::MEDIUM) + 34.0
    });
    let bar_w = 340.0 + 30.0 * toggles.len() as f32 + scope_w;
    let bar_h = 44.0;
    let x = w - bar_w - 16.0;
    let y = top + 10.0;

    layer.shadow(
        [x, y + 6.0],
        [bar_w, bar_h],
        Color::hex("#000000").unwrap().with_alpha(0.5),
        8.0,
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
            4.0,
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
            4.0,
            1.6,
        );
        layer.rect([x + 23.5, y + 24.5], [5.0, 1.7], theme.text_dim, 1.0);
    }
    // find limited to the selected block: its chip; a click widens
    let mut scope_rect = None;
    if let Some(text) = &scope {
        let r = [qx - 4.0, y + 11.0, scope_w - 8.0, 22.0];
        layer.rect(
            [r[0], r[1]],
            [r[2], r[3]],
            theme.accent.with_alpha(0.15),
            4.0,
        );
        let mut b = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 15.0));
        b.set_text(
            &mut engine.font_system,
            text,
            &Attrs::new()
                .family(Family::Name(FONT_SANS))
                .weight(Weight::MEDIUM),
            Shaping::Advanced,
            None,
        );
        layer.text(b, r[0] + 8.0, y + 15.0, theme.accent);
        super::close_icon(layer, [r[0] + r[2] - 11.0, y + 22.0], 3.0, theme.accent);
        scope_rect = Some(r);
        qx += scope_w;
    }

    // toggle chips (Aa match case, .* regex, ! invert, ±N context), right edge
    let mut rects = Vec::new();
    let mut right = x + bar_w - 12.0;
    if !toggles.is_empty() {
        for (text, on) in toggles.iter().rev().copied() {
            let cw = if text.chars().count() > 2 { 32.0 } else { 26.0 };
            let cx = right - cw;
            let r = [cx, y + 11.0, cw, 22.0];
            if on {
                layer.rect(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    theme.accent.with_alpha(0.22),
                    4.0,
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
        rects.reverse(); // left to right, as given
        right -= 6.0;
    } else {
        right -= 4.0;
    }
    rects.extend(scope_rect);

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

    // query + caret, clipped to the space left of the counter; long
    // queries scroll left so the caret (always at the end) stays visible
    let q_right = right - cw - 8.0;
    let avail = (q_right - qx).max(0.0);
    let qw = engine.measure(query, 13.5, mono_family(), Weight::NORMAL);
    let scroll = query_scroll(qw + 3.0, avail);
    if selected && !query.is_empty() {
        let left = (qx - scroll).max(qx);
        let width = (qx - scroll + qw).min(q_right) - left;
        if width > 0.0 {
            layer.rect([left, y + 12.0], [width, 18.0], theme.selection, 2.0);
        }
    }
    if !query.is_empty() {
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(13.5, 18.0));
        buf.set_text(
            &mut engine.font_system,
            query,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(
            buf,
            qx - scroll,
            y + 13.0,
            theme.text_bright,
            TextBounds {
                left: qx.floor() as i32,
                top: y as i32,
                right: q_right.ceil() as i32,
                bottom: (y + bar_h) as i32,
            },
        );
    }
    if blink_on && avail > 0.0 {
        layer.rect(
            [qx + qw - scroll + 1.0, y + 12.0],
            [2.0, 18.0],
            theme.accent,
            0.0,
        );
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::terminal::Found;

    #[test]
    fn query_scroll_keeps_caret_visible() {
        assert_eq!(query_scroll(50.0, 200.0), 0.0);
        assert_eq!(query_scroll(200.0, 200.0), 0.0);
        assert_eq!(query_scroll(260.0, 200.0), 60.0);
        // no room at all: the whole text scrolls out
        assert_eq!(query_scroll(40.0, -5.0), 40.0);
        // caret end after the shift sits at the viewport's right edge
        let (w, avail) = (731.5, 120.0);
        assert_eq!(w - query_scroll(w, avail), avail);
    }

    #[test]
    fn counter_reports_position_among_all_matches() {
        let mut s = SearchState {
            query: "hit".into(),
            ..Default::default()
        };
        assert_eq!(counter(&s), "0 results");
        s.set_matches(Found {
            anchors: (100..600).map(|i| (i, 0)).collect(),
            total: 600,
        });
        assert_eq!(counter(&s), "600/600");
        s.current = 0;
        assert_eq!(counter(&s), "101/600");
        s.set_matches(Found {
            anchors: vec![(1, 0), (2, 0)],
            total: 2,
        });
        assert_eq!(counter(&s), "2/2");
        // matches a block filter hides are said, never silently missing
        s.hidden = 3;
        assert_eq!(counter(&s), "2/2 · 3 hidden by the filter");
        s.set_matches(Found::default());
        assert_eq!(counter(&s), "0 results · 3 hidden by the filter");
        s.close();
        assert_eq!(counter(&s), "search");
    }
}
