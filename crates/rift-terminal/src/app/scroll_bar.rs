//! The overlay scrollbar's input (`views::scrollbar` draws it): a press on
//! the thumb grabs it, a press on the track pages, the pointer moving with
//! the thumb held scrolls, and an `auto` bar goes after its time.

use super::*;

impl App {
    /// A press on `pane`'s bar at y.
    pub(super) fn press_scrollbar(&mut self, pane: PaneId, y: f32) {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return;
        };
        let Some(p) = tab.pane_mut(pane) else {
            return;
        };
        if let Some(offset) = p.scrollbar.press(y, p.view_offset) {
            p.view_offset = offset;
        }
    }

    /// The pointer moved with a thumb held: scroll its pane. False when no
    /// thumb is held.
    pub(super) fn drag_scrollbar(&mut self, y: f32) -> bool {
        let Some(tab) = self.win.tabs.get_mut(self.win.active) else {
            return false;
        };
        let Some(p) = tab.panes.iter_mut().find(|p| p.scrollbar.drag.is_some()) else {
            return false;
        };
        if let Some(offset) = p.scrollbar.drag_to(y) {
            p.view_offset = offset;
        }
        true
    }

    /// Release a held thumb; true when one was held.
    pub(super) fn end_scrollbar_drag(&mut self) -> bool {
        let mut held = false;
        for tab in &mut self.win.tabs {
            for p in &mut tab.panes {
                held |= p.scrollbar.drag.take().is_some();
            }
        }
        held
    }

    /// An `auto` bar shown for a scroll: a repaint when its time is up, and
    /// when the next one's is.
    pub(super) fn scrollbar_deadlines(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let mode = self.config.scrollbar;
        let mut repaint = false;
        let mut next: Option<Instant> = None;
        let Some(tab) = self.win.tabs.get(self.win.active) else {
            return (false, None);
        };
        for p in &tab.panes {
            if let Some(at) = p.scrollbar.hide_at(mode) {
                if now >= at {
                    repaint = true;
                } else {
                    next = Some(next.map_or(at, |n| n.min(at)));
                }
            }
        }
        (repaint, next)
    }
}

/// `pane`'s bar takes clicks in `zone` (its right padding). A split
/// divider's grab reaches a few pixels into the pane, over the bar, and
/// its hit comes first: the bar's own track goes ahead of such a grab, so
/// a press on the bar beside a divider scrolls instead of resizing.
pub(super) fn push_hit(hits: &mut Vec<Hit>, zone: [f32; 4], track: [f32; 4], pane: PaneId) {
    let overlaps = |r: [f32; 4]| {
        r[0] < track[0] + track[2]
            && track[0] < r[0] + r[2]
            && r[1] < track[1] + track[3]
            && track[1] < r[1] + r[3]
    };
    if let Some(at) = hits
        .iter()
        .position(|h| matches!(h.target, ClickTarget::HandleDrag(..)) && overlaps(h.rect))
    {
        hits.insert(
            at,
            Hit {
                rect: track,
                target: ClickTarget::Scrollbar(pane),
            },
        );
    }
    hits.push(Hit {
        rect: zone,
        target: ClickTarget::Scrollbar(pane),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first(hits: &[Hit], x: f32, y: f32) -> Option<&ClickTarget> {
        hits.iter().find(|h| h.contains(x, y)).map(|h| &h.target)
    }

    #[test]
    fn the_bar_beside_a_divider_takes_its_clicks() {
        // the left pane ends at x=590; its divider's grab spans 586..595
        let grab = [586.0, 0.0, 9.0, 400.0];
        let mut hits = vec![Hit {
            rect: grab,
            target: ClickTarget::HandleDrag(Vec::new(), 0),
        }];
        let track = crate::views::scrollbar::track(590.0, 0.0, 400.0);
        let zone = crate::views::scrollbar::zone(track);
        push_hit(&mut hits, zone, track, 7);
        // on the drawn bar (583..589): the bar, also where the grab reaches
        assert!(matches!(
            first(&hits, 584.0, 100.0),
            Some(ClickTarget::Scrollbar(7))
        ));
        assert!(matches!(
            first(&hits, 588.0, 100.0),
            Some(ClickTarget::Scrollbar(7))
        ));
        // past the bar: the divider still
        assert!(matches!(
            first(&hits, 589.5, 100.0),
            Some(ClickTarget::HandleDrag(..))
        ));
        assert!(matches!(
            first(&hits, 593.0, 100.0),
            Some(ClickTarget::HandleDrag(..))
        ));
    }
}
