//! The terminal's overlay scrollbar with marks (`scrollbar`; Windows
//! Terminal's `showMarksOnScrollbar`). It sits in the pane's right padding,
//! so it never covers text and the grid never moves for it. The thumb is
//! the app-owned viewport (`view_offset` over the display-line list, so
//! collapsed blocks and filters count as the user sees them), never
//! alacritty's `display_offset`. Ticks mark prompts, failed commands,
//! bookmarks and find matches at their place in that list. Dragging the
//! thumb scrolls; a click on the track pages toward the click.

use std::time::{Duration, Instant};

use rift_core::{Color, Theme, config::ScrollbarMode};
use rift_renderer::Layer;

/// Width of the bar (the pane padding is 8 px).
pub const WIDTH: f32 = 6.0;
/// Gap between the bar and the pane's right edge.
pub const INSET: f32 = 1.0;
/// Shortest thumb.
pub const MIN_THUMB: f32 = 16.0;
/// Height of a tick.
pub const TICK_H: f32 = 2.0;
/// `auto`: how long the bar stays after the pane last scrolled.
pub const SHOW_FOR: Duration = Duration::from_millis(1200);

/// What a tick marks, in drawing order (later ones draw over earlier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TickKind {
    Prompt,
    Match,
    Failed,
    Bookmark,
}

impl TickKind {
    fn color(self, theme: &Theme) -> Color {
        match self {
            TickKind::Prompt => theme.text_dim,
            TickKind::Match => theme.search_match,
            TickKind::Failed => theme.error,
            TickKind::Bookmark => theme.accent,
        }
    }
}

/// The pane's display-line list as the bar sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScrollInfo {
    /// Display lines in all.
    pub len: usize,
    /// Lines in view.
    pub rows: usize,
    /// Index of the top line in view.
    pub start: usize,
    /// (display index, kind).
    pub ticks: Vec<(usize, TickKind)>,
}

impl ScrollInfo {
    /// Something to scroll.
    pub fn scrolls(&self) -> bool {
        self.len > self.rows
    }

    /// The thumb inside `track`.
    pub fn thumb(&self, track: [f32; 4]) -> [f32; 4] {
        let [x, y, w, h] = track;
        if !self.scrolls() {
            return track;
        }
        let th = (self.rows as f32 / self.len as f32 * h).clamp(MIN_THUMB.min(h), h);
        let frac = self.start as f32 / (self.len - self.rows) as f32;
        [x, y + frac * (h - th), w, th]
    }

    /// The `view_offset` that puts the thumb's top at `top`.
    pub fn offset_for_top(&self, track: [f32; 4], top: f32) -> usize {
        if !self.scrolls() {
            return 0;
        }
        let th = self.thumb(track)[3];
        let room = (track[3] - th).max(1.0);
        let frac = ((top - track[1]) / room).clamp(0.0, 1.0);
        let max = self.len - self.rows;
        let start = (frac * max as f32).round() as usize;
        max - start.min(max)
    }

    /// The y of display line `index` on `track`.
    pub fn tick_y(&self, track: [f32; 4], index: usize) -> f32 {
        let frac = index as f32 / self.len.max(1) as f32;
        (track[1] + frac * track[3]).min(track[1] + track[3] - TICK_H)
    }

    /// Ticks per kind (the `dump` line).
    pub fn counts(&self) -> [usize; 4] {
        let mut c = [0; 4];
        for (_, k) in &self.ticks {
            c[*k as usize] += 1;
        }
        c
    }
}

/// A pane's bar between frames.
#[derive(Debug, Default)]
pub struct PaneScrollbar {
    /// The last frame's list, `None` on the alternate screen.
    pub info: Option<ScrollInfo>,
    /// Where the bar was laid out last frame (drawn or not).
    pub track: [f32; 4],
    /// It was drawn last frame.
    pub visible: bool,
    /// `view_offset` last frame: a change shows the bar (`auto`).
    pub last_offset: usize,
    /// When the pane last scrolled.
    pub shown_at: Option<Instant>,
    /// The pointer's offset from the thumb's top while dragging it.
    pub drag: Option<f32>,
}

impl PaneScrollbar {
    /// Track the frame: `info` as rendered, `offset` the pane's view offset.
    pub fn update(
        &mut self,
        info: Option<ScrollInfo>,
        track: [f32; 4],
        offset: usize,
        now: Instant,
    ) {
        if offset != self.last_offset {
            self.shown_at = Some(now);
            self.last_offset = offset;
        }
        self.info = info;
        self.track = track;
    }

    /// Whether the bar shows this frame (`hovered`: the pointer is over its
    /// zone).
    pub fn shows(&self, mode: ScrollbarMode, hovered: bool, now: Instant) -> bool {
        if !self.info.as_ref().is_some_and(|i| i.scrolls()) {
            return false;
        }
        match mode {
            ScrollbarMode::Never => false,
            ScrollbarMode::Always => true,
            ScrollbarMode::Auto => {
                hovered || self.drag.is_some() || self.shown_at.is_some_and(|t| now < t + SHOW_FOR)
            }
        }
    }

    /// When an `auto` bar shown for a scroll goes again.
    pub fn hide_at(&self, mode: ScrollbarMode) -> Option<Instant> {
        (mode == ScrollbarMode::Auto && self.visible && self.drag.is_none())
            .then_some(self.shown_at?)
            .map(|t| t + SHOW_FOR)
    }

    /// A press at y on the bar: grabs the thumb (`true`), or pages toward
    /// the press. Returns the new view offset for a page.
    pub fn press(&mut self, y: f32, offset: usize) -> Option<usize> {
        let info = self.info.as_ref()?;
        let thumb = info.thumb(self.track);
        if y >= thumb[1] && y < thumb[1] + thumb[3] {
            self.drag = Some(y - thumb[1]);
            return None;
        }
        let max = info.len.saturating_sub(info.rows);
        Some(if y < thumb[1] {
            (offset + info.rows).min(max)
        } else {
            offset.saturating_sub(info.rows)
        })
    }

    /// The view offset for a drag to y.
    pub fn drag_to(&self, y: f32) -> Option<usize> {
        let grab = self.drag?;
        Some(self.info.as_ref()?.offset_for_top(self.track, y - grab))
    }
}

/// The bar's place: the pane's right padding beside the rows in view.
pub fn track(pane_right: f32, top: f32, height: f32) -> [f32; 4] {
    [pane_right - INSET - WIDTH, top, WIDTH, height.max(0.0)]
}

/// The zone that shows the bar under the pointer and takes its clicks: the
/// whole right padding beside the track.
pub fn zone(track: [f32; 4]) -> [f32; 4] {
    [track[0] - 1.0, track[1], WIDTH + INSET + 1.0, track[3]]
}

/// Draw the bar: a faint track while it is under the pointer, the thumb,
/// then the ticks over it (one per pixel row and kind).
pub fn draw(layer: &mut Layer, theme: &Theme, track: [f32; 4], info: &ScrollInfo, active: bool) {
    let radius = WIDTH / 2.0;
    if active {
        layer.rect(
            [track[0], track[1]],
            [track[2], track[3]],
            theme.fg.with_alpha(0.06),
            radius,
        );
    }
    let thumb = info.thumb(track);
    let ink = if active {
        theme.text_dim.with_alpha(0.7)
    } else {
        theme.text_muted.with_alpha(0.45)
    };
    layer.rect([thumb[0], thumb[1]], [thumb[2], thumb[3]], ink, radius);
    let mut ticks: Vec<(TickKind, i32)> = info
        .ticks
        .iter()
        .map(|(i, k)| (*k, info.tick_y(track, *i).round() as i32))
        .collect();
    ticks.sort_unstable();
    ticks.dedup();
    for (kind, y) in ticks {
        layer.rect(
            [track[0], y as f32],
            [track[2], TICK_H],
            kind.color(theme),
            0.0,
        );
    }
}

/// The bar's state for `dump`: `none` (alternate screen or nothing to
/// scroll), else `shown|hidden len=… rows=… start=… thumb=y,h
/// ticks=prompt,match,failed,bookmark`.
pub fn dump(bar: &PaneScrollbar) -> String {
    match bar.info.as_ref().filter(|i| i.scrolls()) {
        None => "none".into(),
        Some(i) => {
            let t = i.thumb(bar.track);
            let [p, m, f, b] = i.counts();
            format!(
                "{} len={} rows={} start={} track={:.1},{:.1},{:.1},{:.1} thumb={:.1},{:.1} ticks={p},{m},{f},{b}",
                if bar.visible { "shown" } else { "hidden" },
                i.len,
                i.rows,
                i.start,
                bar.track[0],
                bar.track[1],
                bar.track[2],
                bar.track[3],
                t[1],
                t[3],
            )
        }
    }
}

/// One frame of a pane's bar: track the list (`info`, `None` on the
/// alternate screen), draw the bar when it shows, and return its click
/// zone while there is something to scroll. `place` = (pane's right edge,
/// top of the rows, their height).
#[allow(clippy::too_many_arguments)]
pub fn frame(
    layer: &mut Layer,
    theme: &Theme,
    bar: &mut PaneScrollbar,
    info: Option<ScrollInfo>,
    place: [f32; 3],
    offset: usize,
    mode: ScrollbarMode,
    mouse: (f32, f32),
) -> Option<[f32; 4]> {
    let now = Instant::now();
    let track = track(place[0], place[1], place[2]);
    bar.update(info, track, offset, now);
    // a scroll's time is up: no further repaint is due for it
    if bar.shown_at.is_some_and(|t| now >= t + SHOW_FOR) {
        bar.shown_at = None;
    }
    let zone = zone(track);
    let hovered = mouse.0 >= zone[0]
        && mouse.0 < zone[0] + zone[2]
        && mouse.1 >= zone[1]
        && mouse.1 < zone[1] + zone[3];
    bar.visible = bar.shows(mode, hovered, now);
    let info = bar.info.as_ref().filter(|i| i.scrolls())?;
    if bar.visible {
        draw(layer, theme, track, info, hovered || bar.drag.is_some());
    }
    (mode != ScrollbarMode::Never).then_some(zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(len: usize, rows: usize, start: usize) -> ScrollInfo {
        ScrollInfo {
            len,
            rows,
            start,
            ticks: Vec::new(),
        }
    }

    const TRACK: [f32; 4] = [100.0, 10.0, WIDTH, 200.0];

    #[test]
    fn the_thumb_is_the_view_over_the_list() {
        // top, middle, bottom of 1000 lines with 100 in view
        assert_eq!(info(1000, 100, 0).thumb(TRACK), [100.0, 10.0, WIDTH, 20.0]);
        assert_eq!(info(1000, 100, 450).thumb(TRACK)[1], 10.0 + 90.0);
        assert_eq!(info(1000, 100, 900).thumb(TRACK)[1], 10.0 + 180.0);
        // a long list keeps a thumb you can grab
        assert_eq!(info(1_000_000, 100, 0).thumb(TRACK)[3], MIN_THUMB);
        // nothing to scroll: no bar at all
        assert!(!info(50, 100, 0).scrolls());
    }

    #[test]
    fn dragging_and_paging_scroll_the_view() {
        let mut bar = PaneScrollbar {
            info: Some(info(1000, 100, 900)),
            track: TRACK,
            ..Default::default()
        };
        // a press on the thumb (at the bottom) grabs it
        assert_eq!(bar.press(195.0, 0), None);
        assert_eq!(bar.drag, Some(5.0));
        // dragged to the top: the oldest line; halfway: the middle
        assert_eq!(bar.drag_to(15.0), Some(900));
        assert_eq!(bar.drag_to(10.0 + 90.0 + 5.0), Some(450));
        assert_eq!(bar.drag_to(-500.0), Some(900));
        assert_eq!(bar.drag_to(900.0), Some(0));
        // a press above the thumb pages up, below pages down
        bar.drag = None;
        bar.info = Some(info(1000, 100, 450));
        assert_eq!(bar.press(20.0, 450), Some(550));
        assert_eq!(bar.press(205.0, 450), Some(350));
        assert_eq!(bar.press(205.0, 30), Some(0));
        assert_eq!(bar.press(20.0, 880), Some(900));
    }

    #[test]
    fn auto_shows_while_scrolling_or_hovered_and_hides_after() {
        let now = Instant::now();
        let mut bar = PaneScrollbar::default();
        bar.update(Some(info(1000, 100, 900)), TRACK, 0, now);
        assert!(!bar.shows(ScrollbarMode::Auto, false, now));
        assert!(bar.shows(ScrollbarMode::Auto, true, now));
        assert!(bar.shows(ScrollbarMode::Always, false, now));
        assert!(!bar.shows(ScrollbarMode::Never, true, now));
        // a scroll shows it for SHOW_FOR
        bar.update(Some(info(1000, 100, 897)), TRACK, 3, now);
        assert!(bar.shows(ScrollbarMode::Auto, false, now));
        bar.visible = true;
        assert_eq!(bar.hide_at(ScrollbarMode::Auto), Some(now + SHOW_FOR));
        assert!(!bar.shows(ScrollbarMode::Auto, false, now + SHOW_FOR));
        // nothing to scroll: never shown
        bar.update(Some(info(10, 100, 0)), TRACK, 0, now);
        assert!(!bar.shows(ScrollbarMode::Always, true, now));
    }

    #[test]
    fn ticks_sit_at_their_place_in_the_list() {
        let i = ScrollInfo {
            ticks: vec![
                (0, TickKind::Prompt),
                (500, TickKind::Failed),
                (999, TickKind::Bookmark),
            ],
            ..info(1000, 100, 0)
        };
        assert_eq!(i.tick_y(TRACK, 0), 10.0);
        assert_eq!(i.tick_y(TRACK, 500), 110.0);
        // the last line's tick stays on the track
        assert!(i.tick_y(TRACK, 999) <= 10.0 + 200.0 - TICK_H);
        assert_eq!(i.counts(), [1, 0, 1, 1]);
    }
}
