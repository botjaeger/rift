//! Moving tabs (and panes) between windows, as Warp and Windows Terminal
//! do. Every window lives in this one process, so a move hands the tab's
//! state from one `WindowState` to another: its shells, sessions,
//! scrollback, blocks and editor drafts stay alive and keep their pane ids
//! (events route by pane id, so a notification from a moved pane comes
//! forward in its new window).
//!
//! A tab dragged out of its strip (or rail) and released over another
//! window lands there, at the slot under the pointer when it is over that
//! window's strip (else last); released outside every window it opens a
//! window of its own at that point. Released inside its own window it stays
//! where the reorder left it. The tab and terminal context menus move a
//! tab or a pane the same way.
//!
//! A window whose last tab leaves closes without ending a shell. The
//! drop-down hotkey window can give and take tabs; giving away its last
//! one hides it and gives it a fresh tab, as when it is first created, so
//! the next hotkey shows a shell. The last ordinary window's last tab never
//! moves into the drop-down window (that would close the app's last
//! ordinary window, which quits).

use std::borrow::Cow;

use super::*;
use crate::views::context_menu::{MenuAction, MenuItem};

/// Distance between headless windows on their pretend desktop (physical
/// px): wider than any window, so none overlaps another.
pub(super) const HEADLESS_SPACING: f64 = 100_000.0;

/// Where a dragged tab is pulled to, in its own window's coordinates, for
/// "a new window" in a script's `drag_tab`: left of the window, far from
/// every other.
const SCRIPT_OUTSIDE: f32 = -30_000.0;

/// A window's place on the desktop: origin and size in physical px, and
/// its scale factor (logical = physical / scale).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Geom {
    pub origin: (f64, f64),
    pub size: (f64, f64),
    pub scale: f64,
}

impl Geom {
    fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.origin.0
            && p.0 < self.origin.0 + self.size.0
            && p.1 >= self.origin.1
            && p.1 < self.origin.1 + self.size.1
    }

    /// A desktop point in this window's logical coordinates.
    pub fn local(&self, p: (f64, f64)) -> (f32, f32) {
        (
            ((p.0 - self.origin.0) / self.scale) as f32,
            ((p.1 - self.origin.1) / self.scale) as f32,
        )
    }

    /// A point in this window's logical coordinates on the desktop.
    pub fn screen(&self, p: (f32, f32)) -> (f64, f64) {
        (
            self.origin.0 + f64::from(p.0) * self.scale,
            self.origin.1 + f64::from(p.1) * self.scale,
        )
    }
}

/// Another window a tab can be dropped into: its id, place, the tab slots
/// its strip drew last frame (logical px, by tab index) and its tab count.
pub(super) struct Candidate<'a, T> {
    pub id: T,
    pub geom: Geom,
    pub slots: &'a [[f32; 4]],
    pub tabs: usize,
}

/// Where a dragged tab lands on release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Landing<T> {
    /// Inside its own window: it stays (the reorder already happened).
    Own,
    /// Outside every window: a window of its own.
    NewWindow,
    /// Into that window at that index.
    Into(T, usize),
}

/// The point `p` (logical px, one window's coordinates) is across the tab
/// strip drawn as `slots`: within the band the slots span across the strip
/// (the rail's width, or the strip's height).
pub(super) fn in_band(slots: &[[f32; 4]], p: (f32, f32), vertical: bool) -> bool {
    let (lo, hi) = slots.iter().fold((f32::MAX, f32::MIN), |(lo, hi), r| {
        if vertical {
            (lo.min(r[0]), hi.max(r[0] + r[2]))
        } else {
            (lo.min(r[1]), hi.max(r[1] + r[3]))
        }
    });
    let c = if vertical { p.0 } else { p.1 };
    !slots.is_empty() && c >= lo && c < hi
}

/// The index a tab dropped at `p` takes among `slots`: past every slot
/// whose middle it is beyond, along the strip.
pub(super) fn insert_index(slots: &[[f32; 4]], p: (f32, f32), vertical: bool) -> usize {
    slots
        .iter()
        .filter(|r| {
            if vertical {
                r[1] + r[3] / 2.0 < p.1
            } else {
                r[0] + r[2] / 2.0 < p.0
            }
        })
        .count()
}

/// Where a tab released at desktop point `p` lands: its own window wins
/// (it is under the pointer, in front), then the first of `others` (front
/// to back) holding the point; else outside them all.
pub(super) fn landing<T: Copy>(
    p: (f64, f64),
    own: Geom,
    others: &[Candidate<'_, T>],
    vertical: bool,
) -> Landing<T> {
    if own.contains(p) {
        return Landing::Own;
    }
    let Some(c) = others.iter().find(|c| c.geom.contains(p)) else {
        return Landing::NewWindow;
    };
    let local = c.geom.local(p);
    let at = if in_band(c.slots, local, vertical) {
        insert_index(c.slots, local, vertical).min(c.tabs)
    } else {
        c.tabs
    };
    Landing::Into(c.id, at)
}

/// A tab taken out of a window, with what that window kept about its panes
/// that must follow it (`detach_tab`, `attach_tab`).
struct MovedTab {
    tab: TabState,
    toasts: Vec<Toast>,
    a11y_done: Vec<(PaneId, rift_core::BlockId)>,
}

/// Where a moved tab goes.
pub(super) enum MoveTo {
    /// A new window with this frame (none: the default place).
    NewWindow(Option<rift_core::session_state::SavedWindow>),
    /// That window, at that index (none: last).
    Window(WindowId, Option<usize>),
}

impl App {
    /// The tab slots `w` drew last frame, by tab index (shell tabs only).
    fn tab_slots(w: &WindowState) -> Vec<[f32; 4]> {
        let mut slots: Vec<(usize, [f32; 4])> = w
            .hits
            .iter()
            .filter_map(|h| match h.target {
                ClickTarget::TabSelect(j) if j < w.tabs.len() => Some((j, h.rect)),
                _ => None,
            })
            .collect();
        slots.sort_by_key(|(j, _)| *j);
        slots.dedup_by_key(|(j, _)| *j);
        slots.into_iter().map(|(_, r)| r).collect()
    }

    /// Where window `w` is on the desktop; none where the platform does not
    /// say (Wayland). Headless windows sit on a pretend desktop.
    fn window_geom(&self, w: &WindowState) -> Option<Geom> {
        let win = w.window.as_ref()?;
        let size = win.inner_size();
        let origin = if self.headless {
            w.headless_origin
        } else {
            let p = win.inner_position().ok()?;
            (f64::from(p.x), f64::from(p.y))
        };
        Some(Geom {
            origin,
            size: (f64::from(size.width), f64::from(size.height)),
            scale: win.scale_factor(),
        })
    }

    fn vertical_tabs(&self) -> bool {
        self.config.tab_layout == rift_core::TabLayout::Vertical
    }

    /// The pointer is over the served window's tab strip (or rail): a
    /// dragged tab reorders only there.
    pub(super) fn pointer_in_tab_bar(&self) -> bool {
        let (mx, my) = self.win.mouse;
        let (lw, lh) = self.logical_size();
        mx >= 0.0
            && my >= 0.0
            && mx < lw
            && my < lh
            && in_band(&Self::tab_slots(&self.win), (mx, my), self.vertical_tabs())
    }

    /// The other windows a tab can be dropped into, front to back (the
    /// most recently focused first); a hidden drop-down window, or a hidden
    /// or minimized window of a run with a display, is not one.
    fn drop_candidates(&self) -> Vec<&WindowState> {
        let me = self.win_id();
        let others: Vec<&WindowState> = self
            .parked
            .iter()
            .map(|w| &**w)
            .filter(|w| w.window.is_some())
            .filter(|w| w.dropdown.as_ref().is_none_or(|d| d.shown))
            .filter(|w| {
                self.headless
                    || w.window.as_ref().is_some_and(|x| {
                        x.is_visible() != Some(false) && x.is_minimized() != Some(true)
                    })
            })
            .collect();
        let id = |w: &WindowState| w.window.as_ref().map(|x| x.id());
        let mut front: Vec<&WindowState> = self
            .focus_history
            .iter()
            .rev()
            .filter(|f| Some(**f) != me)
            .filter_map(|f| others.iter().find(|w| id(w) == Some(*f)).copied())
            .collect();
        for w in others {
            if !front.iter().any(|o| std::ptr::eq(*o, w)) {
                front.push(w);
            }
        }
        front
    }

    /// Where a tab released at `p` (the served window's logical
    /// coordinates) lands.
    fn landing_at(&self, p: (f32, f32)) -> Landing<WindowId> {
        let Some(own) = self.window_geom(&self.win) else {
            // no desktop coordinates: only "in this window or not"
            let (lw, lh) = self.logical_size();
            let inside = p.0 >= 0.0 && p.1 >= 0.0 && p.0 < lw && p.1 < lh;
            return if inside {
                Landing::Own
            } else {
                Landing::NewWindow
            };
        };
        let windows = self.drop_candidates();
        let slots: Vec<Vec<[f32; 4]>> = windows.iter().map(|w| Self::tab_slots(w)).collect();
        let others: Vec<Candidate<'_, WindowId>> = windows
            .iter()
            .zip(&slots)
            .filter_map(|(w, s)| {
                Some(Candidate {
                    id: w.window.as_ref()?.id(),
                    geom: self.window_geom(w)?,
                    slots: s,
                    tabs: w.tabs.len(),
                })
            })
            .collect();
        landing(own.screen(p), own, &others, self.vertical_tabs())
    }

    /// A live tab drag ended: move the tab when it was released outside its
    /// own window. True when it moved (the served window changed).
    pub(super) fn release_tab_drag(&mut self, d: TabDrag, event_loop: &ActiveEventLoop) -> bool {
        let p = self.win.mouse;
        match self.landing_at(p) {
            Landing::Own => false,
            Landing::Into(id, at) => {
                self.move_tab(d.index, MoveTo::Window(id, Some(at)), event_loop)
            }
            Landing::NewWindow => {
                // the new window opens with the tab under the pointer
                let frame = self.window_geom(&self.win).map(|g| {
                    let (x, y) = g.screen(p);
                    let (lw, lh) = self.logical_size();
                    let grab = if self.vertical_tabs() { 16.0 } else { d.grab };
                    rift_core::session_state::SavedWindow {
                        x: x / g.scale - f64::from(grab) - 8.0,
                        y: y / g.scale - 20.0,
                        width: f64::from(lw),
                        height: f64::from(lh),
                        maximized: false,
                        fullscreen: false,
                    }
                });
                self.move_tab(d.index, MoveTo::NewWindow(frame), event_loop)
            }
        }
    }

    /// Tab `tab` of the served window may move to a new window
    /// (`to_dropdown` false) or into the drop-down window: a window does
    /// not move its only tab into a window of its own (that is the window),
    /// and the last ordinary window's last tab stays out of the drop-down
    /// window (its closing would quit).
    fn can_move_tab(&self, tab: usize, new_window: bool, to_dropdown: bool) -> bool {
        let n = self.win.tabs.len();
        let ordinary = self.win.dropdown.is_none();
        if tab >= n {
            return false;
        }
        if new_window {
            return n > 1 || !ordinary;
        }
        !(n == 1 && ordinary && to_dropdown && self.ordinary_window_count() <= 1)
    }

    /// Move tab `tab` of the served window to `to`: its shells keep running
    /// and it takes focus there. The source window closes when it was its
    /// last tab (the drop-down window hides with a fresh tab instead). True
    /// when it moved; the destination is served afterwards.
    pub(super) fn move_tab(
        &mut self,
        tab: usize,
        to: MoveTo,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        let Some(src) = self.win_id() else {
            return false;
        };
        let allowed = match &to {
            MoveTo::NewWindow(_) => self.can_move_tab(tab, true, false),
            MoveTo::Window(id, _) if *id == src => false,
            MoveTo::Window(id, _) => {
                let dest = self
                    .parked
                    .iter()
                    .find(|w| w.window.as_ref().is_some_and(|x| x.id() == *id));
                dest.is_some_and(|w| self.can_move_tab(tab, false, w.dropdown.is_some()))
            }
        };
        if !allowed {
            return false;
        }
        // one session write when it is done, not one per step
        let persist = std::mem::replace(&mut self.persist_session, false);
        // focus goes with the tab: this window's focused pane is told so
        self.win.focused = false;
        self.sync_focus();
        let Some(moved) = self.detach_tab(tab) else {
            self.persist_session = persist;
            return false;
        };
        match to {
            MoveTo::Window(id, at) => {
                self.enter_window(id);
                let at = at.unwrap_or(self.win.tabs.len());
                self.attach_tab(moved, at);
            }
            MoveTo::NewWindow(frame) => {
                self.open_window(event_loop, frame);
                self.attach_tab(moved, 0);
            }
        }
        let dest = self.win_id();
        // the source window, emptied: it goes (no shell of it is left to
        // end); the drop-down window stays, hidden, with a fresh tab
        if self.enter_window(src) && self.win.tabs.is_empty() {
            if self.win.dropdown.is_some() {
                let cwd = self.config.start_dir(None);
                self.spawn_profile_tab(None, cwd);
                self.hide_hotkey_window(false);
            } else {
                self.close_window();
            }
        }
        if let Some(id) = dest {
            self.enter_window(id);
            // a hidden drop-down window drops down to show what came in
            self.reveal_hotkey_window();
            self.note_window_focus(id);
            if let Some(w) = self.win.window.as_ref() {
                if !self.headless {
                    w.set_minimized(false);
                    w.focus_window();
                }
                w.request_redraw();
            }
        }
        self.persist_session = persist;
        self.save_session();
        true
    }

    /// Take tab `i` out of the served window, its shells untouched. What
    /// this window keeps about its panes (focus report, a pending press, a
    /// rename) stays behind; the file viewer belongs to the window and
    /// stays too. Selections are cleared. The panes' toasts and announced
    /// blocks go with the tab.
    fn detach_tab(&mut self, i: usize) -> Option<MovedTab> {
        if i >= self.win.tabs.len() {
            return None;
        }
        let mut tab = self.win.tabs.remove(i);
        if tab.root.contains(VIEWER_PANE) {
            tab.root.remove(VIEWER_PANE);
            if tab.zoomed == Some(VIEWER_PANE) {
                tab.zoomed = None;
            }
        }
        let ids: Vec<PaneId> = tab.panes.iter().map(|p| p.session.id).collect();
        let gone = |p: PaneId| ids.contains(&p);
        let w = &mut self.win;
        w.focus_pane = w.focus_pane.filter(|p| !gone(*p));
        w.status_pane = w.status_pane.filter(|p| !gone(*p));
        w.copied = w.copied.filter(|(p, ..)| !gone(*p));
        w.editor_drag = w.editor_drag.filter(|p| !gone(*p));
        w.mouse_report = w.mouse_report.filter(|(p, ..)| !gone(*p));
        w.mouse_hover_cell = w.mouse_hover_cell.filter(|(p, _)| !gone(*p));
        if w.pipe_target.as_ref().is_some_and(|(p, ..)| gone(*p)) {
            w.pipe_target = None;
        }
        if w.link_press.as_ref().is_some_and(|(p, _)| gone(*p)) {
            w.link_press = None;
        }
        let (toasts, kept): (Vec<Toast>, Vec<Toast>) = std::mem::take(&mut w.toasts)
            .into_iter()
            .partition(|t| t.pane.is_some_and(gone));
        w.toasts = kept;
        let a11y_done: Vec<_> = ids
            .iter()
            .filter_map(|p| w.a11y_done.remove(p).map(|b| (*p, b)))
            .collect();
        w.rename = None;
        w.pane_drag = None;
        w.tab_drag = None;
        w.pending_sel = None;
        if !w.tabs.is_empty() {
            if w.active > i {
                w.active -= 1;
            } else if w.active >= w.tabs.len() {
                w.active = w.tabs.len() - 1;
            }
        }
        for p in &mut tab.panes {
            p.session.term.lock().selection = None;
            p.has_selection = false;
        }
        self.sync_viewer_pane();
        Some(MovedTab {
            tab,
            toasts,
            a11y_done,
        })
    }

    /// Put `moved` into the served window at index `at` (clamped) and show
    /// it.
    fn attach_tab(&mut self, moved: MovedTab, at: usize) {
        let MovedTab {
            tab,
            toasts,
            a11y_done,
        } = moved;
        // newest first, as `toast_as` keeps them
        self.win.toasts.extend(toasts);
        self.win.toasts.sort_by_key(|t| std::cmp::Reverse(t.at));
        self.win.toasts.truncate(TOASTS_MAX);
        self.win.a11y_done.extend(a11y_done);
        let at = at.min(self.win.tabs.len());
        self.win.tabs.insert(at, tab);
        if self.win.active >= at && self.win.tabs.len() > 1 {
            self.win.active += 1;
        }
        self.win.rename = None;
        self.win.viewer_focus = false;
        self.switch_tab(at);
    }

    /// Terminal menu: the pane leaves for a window of its own (a split pane
    /// first becomes a tab of its own, then that tab moves).
    pub(super) fn move_pane_to_new_window(&mut self, pane: PaneId, event_loop: &ActiveEventLoop) {
        if pane == VIEWER_PANE {
            return;
        }
        let find = |app: &App| app.win.tabs.iter().position(|t| t.pane(pane).is_some());
        let Some(mut ti) = find(self) else {
            return;
        };
        if self.win.tabs[ti].panes.len() > 1 {
            self.drop_pane(pane, ti, PaneDrop::NewTab);
            let Some(t) = find(self) else {
                return;
            };
            ti = t;
        }
        let frame = self.cascade_frame();
        self.move_tab(ti, MoveTo::NewWindow(frame), event_loop);
    }

    /// The pane menu can move `pane` to a new window: it is split, or not
    /// this window's only tab.
    pub(super) fn can_move_pane(&self, pane: PaneId) -> bool {
        let Some(ti) = self.win.tabs.iter().position(|t| t.pane(pane).is_some()) else {
            return false;
        };
        self.win.tabs[ti].panes.len() > 1 || self.can_move_tab(ti, true, false)
    }

    /// The tab menu's move rows for tab `tab`: "Move to new window", then
    /// one row per other window it can go to, named by its number (as the
    /// switcher counts) and its active tab's title.
    pub(super) fn move_tab_items(&self, tab: usize) -> Vec<MenuItem> {
        let mut items = vec![
            MenuItem::new("Move to new window", MenuAction::MoveTabToNewWindow(tab))
                .enabled(self.can_move_tab(tab, true, false))
                .sep(),
        ];
        let me = self.win_id();
        for (k, w) in self.windows_in_order().into_iter().enumerate() {
            if w.window.as_ref().map(|x| x.id()) == me
                || !self.can_move_tab(tab, false, w.dropdown.is_some())
            {
                continue;
            }
            let title = w
                .tabs
                .get(w.active)
                .map(|t| t.display_title())
                .unwrap_or_default();
            // the menu is a fixed width: the drop-down window (one at most)
            // goes by its kind alone
            let label = if w.dropdown.is_some() {
                "Move to hotkey window".to_string()
            } else {
                format!("Move to window {}: {}", k + 1, short_title(&title))
            };
            items.push(MenuItem::new(
                Cow::Owned(label),
                MenuAction::MoveTabToWindow(tab, k),
            ));
        }
        items
    }

    /// Script `drag_tab`: press tab `tab` of window `from` (opening order),
    /// pull it to tab slot `index` of window `to` (none: out of every
    /// window), and release, through the same press, motion and release
    /// handlers as the mouse. Headless windows sit apart on a pretend
    /// desktop, so the release point is never covered by another window.
    pub(super) fn script_drag_tab(
        &mut self,
        from: usize,
        tab: usize,
        to: Option<usize>,
        index: Option<usize>,
        event_loop: &ActiveEventLoop,
    ) {
        if !self.enter_window_index(from) {
            log::warn!("script: no window {from} ({} open)", self.window_count());
            return;
        }
        if let Some(id) = self.win_id() {
            self.note_window_focus(id);
        }
        let Some(window) = self.win.window.clone() else {
            return;
        };
        let vertical = self.vertical_tabs();
        let Some(r) = Self::tab_slots(&self.win).get(tab).copied() else {
            log::warn!("script: drag_tab: window {from} drew no tab {tab}");
            return;
        };
        let press = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let release = match to {
            None => (SCRIPT_OUTSIDE, press.1),
            Some(k) => {
                let target = self
                    .window_order
                    .get(k)
                    .and_then(|id| {
                        self.parked
                            .iter()
                            .find(|w| w.window.as_ref().is_some_and(|x| x.id() == *id))
                    })
                    .map(|w| (Self::tab_slots(w), self.window_geom(w), w.window.as_ref()));
                let Some((slots, Some(geom), Some(win))) = target else {
                    log::warn!("script: drag_tab: no other window {k}");
                    return;
                };
                let local = match index.map(|i| (i, slots.get(i))) {
                    // just inside the slot's leading edge: it lands before it
                    Some((_, Some(s))) if vertical => (s[0] + s[2] / 2.0, s[1] + 1.0),
                    Some((_, Some(s))) => (s[0] + 1.0, s[1] + s[3] / 2.0),
                    // past the last slot's middle: it lands last
                    _ => match slots.last() {
                        Some(s) if vertical => (s[0] + s[2] / 2.0, s[1] + s[3] - 1.0),
                        Some(s) => (s[0] + s[2] - 1.0, s[1] + s[3] / 2.0),
                        None => {
                            let size = win.inner_size().to_logical::<f32>(geom.scale);
                            (size.width / 2.0, size.height / 2.0)
                        }
                    },
                };
                match self.window_geom(&self.win) {
                    Some(own) => own.local(geom.screen(local)),
                    None => (SCRIPT_OUTSIDE, press.1),
                }
            }
        };
        self.on_cursor_moved(press, &window);
        self.on_left(ElementState::Pressed, event_loop, &window);
        self.on_cursor_moved(release, &window);
        self.on_left(ElementState::Released, event_loop, &window);
    }
}

/// A window's active tab title, short enough for a menu row after "Move to
/// window N: " (the menu is `MENU_W` wide).
fn short_title(title: &str) -> Cow<'_, str> {
    const MAX: usize = 12;
    if title.chars().count() <= MAX {
        return Cow::Borrowed(title);
    }
    let cut: String = title.chars().take(MAX - 1).collect();
    Cow::Owned(format!("{}…", cut.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: Geom = Geom {
        origin: (0.0, 0.0),
        size: (1000.0, 800.0),
        scale: 1.0,
    };

    /// Three tabs 100 px wide from x 80, y 4..36 (a horizontal strip).
    fn strip() -> Vec<[f32; 4]> {
        (0..3)
            .map(|i| [80.0 + 100.0 * i as f32, 4.0, 100.0, 32.0])
            .collect()
    }

    /// Three rail rows 40 px tall from y 50, x 0..200 (vertical tabs).
    fn rail() -> Vec<[f32; 4]> {
        (0..3)
            .map(|i| [0.0, 50.0 + 40.0 * i as f32, 200.0, 40.0])
            .collect()
    }

    #[test]
    fn insert_index_counts_slots_whose_middle_is_passed() {
        let s = strip();
        assert_eq!(insert_index(&s, (81.0, 10.0), false), 0);
        assert_eq!(insert_index(&s, (129.0, 10.0), false), 0);
        assert_eq!(insert_index(&s, (131.0, 10.0), false), 1);
        assert_eq!(insert_index(&s, (181.0, 10.0), false), 1);
        assert_eq!(insert_index(&s, (379.0, 10.0), false), 3);
        let r = rail();
        assert_eq!(insert_index(&r, (100.0, 51.0), true), 0);
        assert_eq!(insert_index(&r, (100.0, 91.0), true), 1);
        assert_eq!(insert_index(&r, (100.0, 169.0), true), 3);
    }

    #[test]
    fn band_is_across_the_strip_only() {
        let s = strip();
        assert!(in_band(&s, (900.0, 10.0), false));
        assert!(!in_band(&s, (100.0, 40.0), false));
        let r = rail();
        assert!(in_band(&r, (10.0, 700.0), true));
        assert!(!in_band(&r, (201.0, 60.0), true));
        assert!(!in_band(&[], (0.0, 0.0), false));
    }

    #[test]
    fn landing_prefers_own_window_then_front_to_back() {
        let s = strip();
        let behind = Geom {
            origin: (500.0, 0.0),
            ..ONE
        };
        let front = Geom {
            origin: (1200.0, 0.0),
            ..ONE
        };
        let others = [
            Candidate {
                id: 1,
                geom: front,
                slots: &s,
                tabs: 3,
            },
            Candidate {
                id: 2,
                geom: behind,
                slots: &s,
                tabs: 3,
            },
        ];
        // inside its own window (even where another window lies behind)
        assert_eq!(landing((600.0, 10.0), ONE, &others, false), Landing::Own);
        // over the front window's strip, just inside slot 1: before it
        assert_eq!(
            landing((1200.0 + 181.0, 10.0), ONE, &others, false),
            Landing::Into(1, 1)
        );
        // over the window behind, where the front one does not reach
        assert_eq!(
            landing((1100.0, 10.0), ONE, &others, false),
            Landing::Into(2, 3)
        );
        // over a window but off its strip: last
        assert_eq!(
            landing((1300.0, 400.0), ONE, &others, false),
            Landing::Into(1, 3)
        );
        // outside every window
        assert_eq!(
            landing((-50.0, 10.0), ONE, &others, false),
            Landing::NewWindow
        );
        assert_eq!(
            landing((5000.0, 10.0), ONE, &others, false),
            Landing::NewWindow
        );
    }

    #[test]
    fn landing_converts_between_scale_factors() {
        let r = rail();
        // a Retina window to the right of a 1x one: rail rows in logical px
        let retina = Geom {
            origin: (1000.0, 0.0),
            size: (2000.0, 1600.0),
            scale: 2.0,
        };
        let others = [Candidate {
            id: 7,
            geom: retina,
            slots: &r,
            tabs: 3,
        }];
        // logical y 91 there is physical 182: past row 0's middle
        assert_eq!(
            landing((1020.0, 182.0), ONE, &others, true),
            Landing::Into(7, 1)
        );
        // the round trip a script's `drag_tab` makes
        let p = retina.screen((10.0, 91.0));
        assert_eq!(retina.local(p), (10.0, 91.0));
    }

    #[test]
    fn short_titles_stay_and_long_ones_end_in_an_ellipsis() {
        assert_eq!(short_title("zsh"), "zsh");
        assert_eq!(short_title("twelve-chars"), "twelve-chars");
        let s = short_title("a-very-long-directory-name-indeed");
        assert_eq!(s, "a-very-long…");
    }
}
