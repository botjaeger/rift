//! The arranger dialog behind "Edit prompt" and "Edit toolbar" (Warp's
//! drag-to-arrange editors): the items not in use sit in a palette; the ones
//! in use sit in one or more dashed buckets, each with ×. Click a palette
//! item to add it, drag one into a bucket to place it (an accent `|` shows
//! where it lands), drag a bucket item to move it, or out of every bucket
//! to remove it. Tab walks the bucket items, ← → move the focused one
//! (across the buckets in reading order), Delete removes it. Restore
//! default, Cancel (Esc) and Save changes (Enter) close it.

use rift_core::{Color, Theme};
use rift_renderer::glyphon::Weight;
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::icons::{self, Icon};
use crate::views::palette::{lift, sans};

const MODAL_W: f32 = 600.0;
const PAD: f32 = 24.0;
const GAP: f32 = 8.0;
/// Between two buckets side by side.
const BUCKET_GAP: f32 = 10.0;
/// Inside a bucket, around its items.
const BUCKET_PAD: f32 = 12.0;
/// The × after an item in a bucket.
const X_W: f32 = 18.0;
/// Pointer travel that turns a press into a drag.
pub const DRAG_SLOP: f32 = 4.0;

/// An item being dragged: from the palette (`from: None`) or from bucket
/// `.0`'s place `.1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag<T> {
    pub item: T,
    pub from: Option<(usize, usize)>,
    pub press: (f32, f32),
    pub live: bool,
}

/// What the items are and where they are: the draft being edited, what is
/// saved, the defaults Restore puts back, and every item in palette order.
#[derive(Debug, Clone, PartialEq)]
pub struct Arranger<T> {
    pub buckets: Vec<Vec<T>>,
    pub saved: Vec<Vec<T>>,
    pub defaults: Vec<Vec<T>>,
    pub all: Vec<T>,
    pub drag: Option<Drag<T>>,
    /// The bucket item the keyboard works on (bucket, place).
    pub focus: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<T> {
    /// A palette item (click: add it).
    Palette(T),
    /// Bucket `.0`'s item `.1` (drag to move).
    Item(usize, usize),
    Remove(usize, usize),
    Restore,
    Cancel,
    Save,
}

/// What the app does after an event: nothing more, close, or save and close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Cancel,
    Save,
}

/// The keys the dialog answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
    Tab { back: bool },
    Left,
    Right,
    Delete,
}

impl<T: Copy + PartialEq> Arranger<T> {
    pub fn new(buckets: Vec<Vec<T>>, defaults: Vec<Vec<T>>, all: Vec<T>) -> Self {
        Arranger {
            saved: buckets.clone(),
            buckets,
            defaults,
            all,
            drag: None,
            focus: None,
        }
    }

    pub fn changed(&self) -> bool {
        self.buckets != self.saved
    }

    /// The draft is the defaults (Restore default has nothing to do).
    pub fn is_default(&self) -> bool {
        self.buckets == self.defaults
    }

    fn holds(&self, item: T) -> bool {
        self.buckets.iter().any(|b| b.contains(&item))
    }

    /// The items in no bucket, in palette order.
    pub fn palette(&self) -> Vec<T> {
        self.all
            .iter()
            .copied()
            .filter(|i| !self.holds(*i))
            .collect()
    }

    /// Append `item` to bucket `b` (an item already placed stays put).
    pub fn add(&mut self, item: T, b: usize) {
        if !self.holds(item)
            && let Some(bucket) = self.buckets.get_mut(b)
        {
            bucket.push(item);
        }
    }

    pub fn remove(&mut self, b: usize, i: usize) {
        if let Some(bucket) = self.buckets.get_mut(b)
            && i < bucket.len()
        {
            bucket.remove(i);
        }
        // the keyboard stays on the bucket: the next item, else the last
        if let Some((fb, fi)) = self.focus
            && fb == b
        {
            let len = self.buckets[b].len();
            self.focus = if len == 0 {
                None
            } else if fi >= i {
                Some((b, fi.saturating_sub(usize::from(fi > i)).min(len - 1)))
            } else {
                Some((fb, fi))
            };
        }
    }

    /// Put `item` at `at` in bucket `b` (`at` counts the bucket's items
    /// without it); it leaves wherever it was.
    pub fn place(&mut self, item: T, b: usize, at: usize) {
        if b >= self.buckets.len() {
            return;
        }
        for bucket in &mut self.buckets {
            bucket.retain(|x| *x != item);
        }
        let at = at.min(self.buckets[b].len());
        self.buckets[b].insert(at, item);
    }

    pub fn restore_default(&mut self) {
        self.buckets = self.defaults.clone();
        self.focus = None;
    }

    /// Move the item at (`b`, `i`) one place `dir` (−1 or +1) in reading
    /// order: past a bucket's end it enters the next bucket at its start
    /// (and before a bucket's start, the previous one's end). Returns where
    /// it went (unchanged at either end of the whole order).
    pub fn step(&mut self, b: usize, i: usize, dir: i32) -> (usize, usize) {
        let Some(&item) = self.buckets.get(b).and_then(|bk| bk.get(i)) else {
            return (b, i);
        };
        let len = self.buckets[b].len();
        let to = if dir < 0 {
            if i > 0 {
                Some((b, i - 1))
            } else if b > 0 {
                Some((b - 1, self.buckets[b - 1].len()))
            } else {
                None
            }
        } else if i + 1 < len {
            // `place` counts without the item: one past its old place
            Some((b, i + 1))
        } else if b + 1 < self.buckets.len() {
            Some((b + 1, 0))
        } else {
            None
        };
        match to {
            Some((tb, ti)) => {
                self.place(item, tb, ti);
                (tb, ti.min(self.buckets[tb].len() - 1))
            }
            None => (b, i),
        }
    }

    /// The bucket items in reading order, as (bucket, place).
    fn order(&self) -> Vec<(usize, usize)> {
        self.buckets
            .iter()
            .enumerate()
            .flat_map(|(b, bk)| (0..bk.len()).map(move |i| (b, i)))
            .collect()
    }

    /// A key: Esc cancels, Enter saves, Tab walks the bucket items, ← →
    /// move the focused one, Delete removes it.
    pub fn key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Escape => return Outcome::Cancel,
            Key::Enter => return Outcome::Save,
            Key::Tab { back } => {
                let order = self.order();
                if order.is_empty() {
                    self.focus = None;
                } else {
                    let at = self.focus.and_then(|f| order.iter().position(|o| *o == f));
                    let n = order.len();
                    let next = match (at, back) {
                        (None, false) => 0,
                        (None, true) => n - 1,
                        (Some(p), false) => (p + 1) % n,
                        (Some(p), true) => (p + n - 1) % n,
                    };
                    self.focus = Some(order[next]);
                }
            }
            Key::Left | Key::Right => {
                if let Some((b, i)) = self.focus {
                    let dir = if key == Key::Left { -1 } else { 1 };
                    self.focus = Some(self.step(b, i, dir));
                }
            }
            Key::Delete => {
                if let Some((b, i)) = self.focus {
                    self.remove(b, i);
                }
            }
        }
        Outcome::Stay
    }

    /// A press: buttons act at once; an item arms a drag (a release
    /// without travel is a click).
    pub fn press(&mut self, layout: &Layout<T>, mouse: (f32, f32)) -> Outcome {
        let hit = layout
            .hits
            .iter()
            .find(|(r, _)| inside(r, mouse))
            .map(|(_, t)| *t);
        match hit {
            Some(Target::Palette(item)) => {
                self.drag = Some(Drag {
                    item,
                    from: None,
                    press: mouse,
                    live: false,
                })
            }
            Some(Target::Item(b, i)) => {
                if let Some(&item) = self.buckets.get(b).and_then(|bk| bk.get(i)) {
                    self.drag = Some(Drag {
                        item,
                        from: Some((b, i)),
                        press: mouse,
                        live: false,
                    });
                }
            }
            Some(Target::Remove(b, i)) => self.remove(b, i),
            Some(Target::Restore) => self.restore_default(),
            Some(Target::Cancel) => return Outcome::Cancel,
            Some(Target::Save) => return Outcome::Save,
            None => {}
        }
        Outcome::Stay
    }

    /// The pointer moved: a press that travels turns into a drag.
    pub fn moved(&mut self, pos: (f32, f32)) {
        if let Some(d) = &mut self.drag
            && !d.live
            && ((pos.0 - d.press.0).abs() > DRAG_SLOP || (pos.1 - d.press.1).abs() > DRAG_SLOP)
        {
            d.live = true;
        }
    }

    /// A release: a drag drops into the bucket under the pointer (or, from
    /// a bucket, out of every bucket to remove the item); a palette item
    /// clicked is added to bucket `add_to`; a bucket item clicked takes
    /// the keyboard focus.
    pub fn release(&mut self, layout: &Layout<T>, mouse: (f32, f32), add_to: usize) {
        let Some(d) = self.drag.take() else {
            return;
        };
        if !d.live {
            match d.from {
                None => self.add(d.item, add_to),
                Some(from) => self.focus = Some(from),
            }
            return;
        }
        match layout.buckets.iter().position(|r| inside(r, mouse)) {
            Some(b) => {
                let skip = d.from.filter(|f| f.0 == b).map(|f| f.1);
                let slots = layout.slots.get(b).map_or(&[][..], |s| s.as_slice());
                let at = drop_index(slots, skip, mouse);
                self.place(d.item, b, at);
                self.focus = Some((b, at.min(self.buckets[b].len() - 1)));
            }
            None => {
                if let Some((b, i)) = d.from {
                    self.remove(b, i);
                }
            }
        }
    }
}

/// Where an item dropped at `p` goes among a bucket's item rects `slots`
/// (the dragged one, `skip`, left out): before the first item past the
/// pointer's x on its row, or before the first item on a lower row.
pub fn drop_index(slots: &[[f32; 4]], skip: Option<usize>, p: (f32, f32)) -> usize {
    let mut at = 0;
    for (i, r) in slots.iter().enumerate() {
        if Some(i) == skip {
            continue;
        }
        let before = p.1 >= r[1] + r[3] || (p.1 >= r[1] && p.0 >= r[0] + r[2] / 2.0);
        if !before {
            break;
        }
        at += 1;
    }
    at
}

fn inside(r: &[f32; 4], p: (f32, f32)) -> bool {
    p.0 >= r[0] && p.0 < r[0] + r[2] && p.1 >= r[1] && p.1 < r[1] + r[3]
}

/// What `render` laid out: the card, the click targets, each bucket's rect
/// and each bucket's item rects (drop positions).
#[derive(Debug, Clone)]
pub struct Layout<T> {
    pub card: [f32; 4],
    pub hits: Vec<([f32; 4], Target<T>)>,
    pub buckets: Vec<[f32; 4]>,
    pub slots: Vec<Vec<[f32; 4]>>,
}

impl<T> Default for Layout<T> {
    fn default() -> Self {
        Layout {
            card: [0.0; 4],
            hits: Vec::new(),
            buckets: Vec::new(),
            slots: Vec::new(),
        }
    }
}

/// How an editor's items look and what its dialog says.
pub struct Look<'a, T> {
    pub title: &'a str,
    /// Over the palette ("Not in the toolbar").
    pub palette_label: &'a str,
    /// The palette when every item is in use.
    pub palette_empty: &'a str,
    /// Over the bucket(s) ("Your toolbar").
    pub buckets_label: &'a str,
    /// Each bucket's own label inside it (none for a single bucket).
    pub bucket_names: &'a [&'a str],
    /// An empty bucket.
    pub bucket_empty: &'a str,
    pub item_h: f32,
    /// The item's frame: drawn by the arranger (true) or by `draw` itself.
    pub framed: bool,
    /// The item's content width (without the ×).
    pub width: &'a dyn Fn(&mut TextEngine, T) -> f32,
    /// Draw the item's content with its top-left at `.3`; `.4` is the room
    /// its frame keeps after it for the × (0 in the palette).
    #[allow(clippy::type_complexity)]
    pub draw: &'a dyn Fn(&mut Layer, &mut TextEngine, &Theme, [f32; 2], T, f32),
}

/// Wrap items of widths `ws` into rows `width` wide from `(x, y)`; returns
/// each one's top-left and the height used.
fn flow(ws: &[f32], x: f32, y: f32, width: f32, h: f32) -> (Vec<[f32; 2]>, f32) {
    let (mut cx, mut cy) = (x, y);
    let mut out = Vec::new();
    for &w in ws {
        if cx > x && cx + w > x + width {
            cx = x;
            cy += h + GAP;
        }
        out.push([cx, cy]);
        cx += w + GAP;
    }
    let used = if ws.is_empty() { 0.0 } else { cy - y + h };
    (out, used)
}

/// A dashed rectangle outline, `t` px thick.
pub(crate) fn dashed(layer: &mut Layer, r: [f32; 4], color: Color, t: f32) {
    let (dash, gap) = (6.0_f32, 4.0_f32);
    let mut x = r[0];
    while x < r[0] + r[2] {
        let w = dash.min(r[0] + r[2] - x);
        layer.rect([x, r[1]], [w, t], color, 0.0);
        layer.rect([x, r[1] + r[3] - t], [w, t], color, 0.0);
        x += dash + gap;
    }
    let mut y = r[1];
    while y < r[1] + r[3] {
        let h = dash.min(r[1] + r[3] - y);
        layer.rect([r[0], y], [t, h], color, 0.0);
        layer.rect([r[0] + r[2] - t, y], [t, h], color, 0.0);
        y += dash + gap;
    }
}

/// A key cap after a button's label (`esc`, `⏎`); returns its width.
fn key_cap(
    layer: &mut Layer,
    engine: &mut TextEngine,
    label: &str,
    x: f32,
    mid: f32,
    fill: Color,
    ink: Color,
) -> f32 {
    let w = engine.measure(label, 10.5, FONT_SANS, Weight::MEDIUM) + 8.0;
    layer.rect([x, mid - 8.0], [w, 16.0], fill, 3.0);
    layer.text(
        sans(engine, label, 10.5, Weight::MEDIUM),
        x + 4.0,
        mid - 7.0,
        ink,
    );
    w
}

const LABEL_H: f32 = 18.0;
const BUCKET_LABEL_H: f32 = 22.0;

/// Draw the dialog centred in a `w`×`h` window over a dim backdrop; the
/// item being dragged goes on `over` (a layer above it: a layer draws its
/// quads before its text, so on the dialog's own layer the items' labels
/// would show through it).
#[allow(clippy::too_many_arguments)]
pub fn render<T: Copy + PartialEq>(
    layer: &mut Layer,
    over: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ed: &Arranger<T>,
    look: &Look<'_, T>,
    [w, h]: [f32; 2],
    mouse: (f32, f32),
) -> Layout<T> {
    let mut out = Layout::default();
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.4), 0.0);
    let modal_w = MODAL_W.min((w - 32.0).max(240.0));
    let inner = modal_w - 2.0 * PAD;
    let ih = look.item_h;
    let frame_pad = if look.framed { 10.0 } else { 0.0 };

    // palette and bucket contents, measured first for the card's height
    let palette = ed.palette();
    let pal_ws: Vec<f32> = palette
        .iter()
        .map(|i| (look.width)(engine, *i) + 2.0 * frame_pad)
        .collect();
    let n = ed.buckets.len().max(1);
    let bucket_w = ((inner - BUCKET_GAP * (n - 1) as f32) / n as f32).floor();
    let item_ws: Vec<Vec<f32>> = ed
        .buckets
        .iter()
        .map(|bk| {
            bk.iter()
                .map(|i| (look.width)(engine, *i) + 2.0 * frame_pad + X_W)
                .collect()
        })
        .collect();
    let (_, pal_h) = flow(&pal_ws, 0.0, 0.0, inner, ih);
    let pal_h = if palette.is_empty() { LABEL_H } else { pal_h };
    let label_h = if look.bucket_names.is_empty() {
        0.0
    } else {
        BUCKET_LABEL_H
    };
    let content_h = item_ws
        .iter()
        .map(|ws| flow(ws, 0.0, 0.0, bucket_w - 2.0 * BUCKET_PAD, ih).1)
        .fold(ih, f32::max);
    let bucket_h =
        (BUCKET_PAD + label_h + content_h + BUCKET_PAD).max(if n > 1 { 96.0 } else { 0.0 });
    let modal_h = PAD
        + 24.0
        + 16.0
        + LABEL_H
        + 6.0
        + pal_h
        + 18.0
        + LABEL_H
        + 6.0
        + bucket_h
        + 24.0
        + 30.0
        + PAD;
    let x = (w - modal_w) / 2.0;
    let y = ((h - modal_h) / 2.0).max(8.0);
    out.card = [x, y, modal_w, modal_h];
    layer.shadow(
        [x, y + 12.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.55),
        8.0,
        24.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );
    let ix = x + PAD;
    let mut cy = y + PAD;

    // ── title, Restore default ──
    layer.text(
        sans(engine, look.title, 16.0, Weight::SEMIBOLD),
        ix,
        cy,
        theme.text_bright,
    );
    let restore = "Restore default";
    let rw = engine.measure(restore, 12.5, FONT_SANS, Weight::NORMAL);
    let rr = [ix + inner - rw - 8.0, cy - 1.0, rw + 8.0, 22.0];
    let can_restore = !ed.is_default();
    if can_restore && inside(&rr, mouse) {
        layer.rect(
            [rr[0], rr[1]],
            [rr[2], rr[3]],
            theme.accent.with_alpha(0.12),
            4.0,
        );
    }
    layer.text(
        sans(engine, restore, 12.5, Weight::NORMAL),
        rr[0] + 4.0,
        cy + 2.0,
        if can_restore {
            theme.accent
        } else {
            theme.text_muted
        },
    );
    if can_restore {
        out.hits.push((rr, Target::Restore));
    }
    cy += 24.0 + 16.0;

    // ── palette ──
    let dragging = ed.drag.as_ref().filter(|d| d.live);
    layer.text(
        sans(engine, look.palette_label, 11.5, Weight::SEMIBOLD),
        ix,
        cy,
        theme.text_dim,
    );
    cy += LABEL_H + 6.0;
    if palette.is_empty() {
        layer.text(
            sans(engine, look.palette_empty, 12.5, Weight::NORMAL),
            ix,
            cy,
            theme.text_muted,
        );
    } else {
        let (spots, _) = flow(&pal_ws, ix, cy, inner, ih);
        for ((item, at), iw) in palette.iter().zip(&spots).zip(&pal_ws) {
            let r = [at[0], at[1], *iw, ih];
            // the item being dragged leaves a faint gap
            if dragging.is_some_and(|d| d.item == *item && d.from.is_none()) {
                layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.04), 5.0);
                continue;
            }
            let hot = inside(&r, mouse) && dragging.is_none();
            draw_item(layer, engine, theme, look, r, *item, 0.0, hot, false);
            out.hits.push((r, Target::Palette(*item)));
        }
    }
    cy += pal_h + 18.0;

    // ── the buckets: dashed, side by side, items each with × ──
    layer.text(
        sans(engine, look.buckets_label, 11.5, Weight::SEMIBOLD),
        ix,
        cy,
        theme.text_dim,
    );
    cy += LABEL_H + 6.0;
    for (b, ws) in item_ws.iter().enumerate() {
        let bx = [
            ix + b as f32 * (bucket_w + BUCKET_GAP),
            cy,
            bucket_w,
            bucket_h,
        ];
        out.buckets.push(bx);
        let over = dragging.is_some() && inside(&bx, mouse);
        if over {
            layer.rect(
                [bx[0], bx[1]],
                [bx[2], bx[3]],
                theme.accent.with_alpha(0.06),
                6.0,
            );
        }
        dashed(
            layer,
            bx,
            if over {
                theme.accent
            } else {
                lift(theme, 0.38)
            },
            1.5,
        );
        let mut top = bx[1] + BUCKET_PAD;
        if let Some(name) = look.bucket_names.get(b) {
            layer.text(
                sans(engine, &name.to_uppercase(), 10.5, Weight::MEDIUM),
                bx[0] + BUCKET_PAD,
                top,
                theme.text_muted,
            );
            top += BUCKET_LABEL_H;
        }
        let (spots, _) = flow(ws, bx[0] + BUCKET_PAD, top, bucket_w - 2.0 * BUCKET_PAD, ih);
        let mut slots = Vec::new();
        for (i, (at, iw)) in spots.iter().zip(ws).enumerate() {
            let item = ed.buckets[b][i];
            let r = [at[0], at[1], *iw, ih];
            slots.push(r);
            if dragging.is_some_and(|d| d.from == Some((b, i))) {
                layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.04), 5.0);
                continue;
            }
            let hot = inside(&[r[0], r[1], r[2] - X_W, r[3]], mouse) && dragging.is_none();
            draw_item(
                layer,
                engine,
                theme,
                look,
                r,
                item,
                X_W,
                hot,
                ed.focus == Some((b, i)),
            );
            let xr = [r[0] + r[2] - X_W - 2.0, r[1], X_W, ih];
            icons::draw(
                layer,
                Icon::Close,
                [xr[0] + X_W / 2.0, xr[1] + ih / 2.0],
                9.0,
                if inside(&xr, mouse) && dragging.is_none() {
                    theme.error
                } else {
                    theme.text_muted
                },
            );
            // the item, then its × (a screen reader's reading order)
            out.hits
                .push(([r[0], r[1], r[2] - X_W - 2.0, r[3]], Target::Item(b, i)));
            out.hits.push((xr, Target::Remove(b, i)));
        }
        if ed.buckets[b].is_empty() {
            layer.text(
                sans(engine, look.bucket_empty, 12.5, Weight::NORMAL),
                bx[0] + BUCKET_PAD,
                top + (ih - 16.0) / 2.0,
                theme.text_muted,
            );
        }
        // where a drop lands
        if over && let Some(d) = dragging {
            let skip = d.from.filter(|f| f.0 == b).map(|f| f.1);
            let at = drop_index(&slots, skip, mouse);
            let others: Vec<[f32; 4]> = slots
                .iter()
                .enumerate()
                .filter(|(i, _)| Some(*i) != skip)
                .map(|(_, r)| *r)
                .collect();
            let (mx, my) = match others.get(at) {
                Some(r) => (r[0] - GAP / 2.0, r[1]),
                None => others.last().map_or((bx[0] + BUCKET_PAD, top), |r| {
                    (r[0] + r[2] + GAP / 2.0, r[1])
                }),
            };
            layer.rect([mx - 1.0, my - 3.0], [2.0, ih + 6.0], theme.accent, 1.0);
        }
        out.slots.push(slots);
    }
    cy += bucket_h + 24.0;

    // ── Cancel esc, Save changes ⏎ ──
    let caps =
        |engine: &mut TextEngine, l: &str| engine.measure(l, 10.5, FONT_SANS, Weight::MEDIUM) + 8.0;
    let save_label = "Save changes";
    let sw = engine.measure(save_label, 12.5, FONT_SANS, Weight::MEDIUM)
        + 24.0
        + 6.0
        + caps(engine, "⏎");
    let save = [ix + inner - sw, cy, sw, 30.0];
    let cw = engine.measure("Cancel", 12.5, FONT_SANS, Weight::MEDIUM)
        + 24.0
        + 6.0
        + caps(engine, "esc");
    let cancel = [save[0] - 8.0 - cw, cy, cw, 30.0];
    layer.bordered(
        [cancel[0], cancel[1]],
        [cancel[2], cancel[3]],
        if inside(&cancel, mouse) {
            lift(theme, 0.1)
        } else {
            lift(theme, 0.06)
        },
        lift(theme, 0.12),
        4.0,
        1.0,
    );
    layer.text(
        sans(engine, "Cancel", 12.5, Weight::MEDIUM),
        cancel[0] + 12.0,
        cy + 7.0,
        theme.text_ui,
    );
    let lw = engine.measure("Cancel", 12.5, FONT_SANS, Weight::MEDIUM);
    key_cap(
        layer,
        engine,
        "esc",
        cancel[0] + 12.0 + lw + 6.0,
        cy + 15.0,
        lift(theme, 0.08),
        theme.text_dim,
    );
    out.hits.push((cancel, Target::Cancel));
    let changed = ed.changed();
    let (fill, ink, cap_fill) = if changed {
        (theme.accent, theme.bg, Color::BLACK.with_alpha(0.18))
    } else {
        (lift(theme, 0.03), theme.text_muted, lift(theme, 0.08))
    };
    if changed {
        layer.rect([save[0], save[1]], [save[2], save[3]], fill, 4.0);
    } else {
        layer.bordered(
            [save[0], save[1]],
            [save[2], save[3]],
            fill,
            lift(theme, 0.08),
            4.0,
            1.0,
        );
    }
    layer.text(
        sans(engine, save_label, 12.5, Weight::MEDIUM),
        save[0] + 12.0,
        cy + 7.0,
        ink,
    );
    let lw = engine.measure(save_label, 12.5, FONT_SANS, Weight::MEDIUM);
    key_cap(
        layer,
        engine,
        "⏎",
        save[0] + 12.0 + lw + 6.0,
        cy + 15.0,
        cap_fill,
        ink,
    );
    if changed {
        out.hits.push((save, Target::Save));
    }

    // the dragged item follows the pointer
    if let Some(d) = dragging {
        let iw = (look.width)(engine, d.item) + 2.0 * frame_pad;
        let r = [mouse.0 - iw / 2.0, mouse.1 - ih / 2.0, iw, ih];
        over.shadow(
            [r[0], r[1] + 4.0],
            [iw, ih],
            Color::BLACK.with_alpha(0.4),
            5.0,
            8.0,
        );
        over.rect([r[0], r[1]], [iw, ih], theme.surface, 5.0);
        draw_item(over, engine, theme, look, r, d.item, 0.0, false, false);
    }
    out
}

/// The dialog for screen readers: one button per item and action (Restore
/// default and Save changes listed, disabled, while they do nothing), and
/// the draft read out bucket by bucket. `places` names each bucket ("the
/// prompt", "the left side"); `list` names the draft ("Prompt").
pub fn a11y_overlay<T: Copy + PartialEq>(
    ed: &Arranger<T>,
    layout: &Layout<T>,
    title: &str,
    list: &str,
    places: &[&str],
    name: &dyn Fn(&T) -> String,
) -> crate::a11y::Overlay {
    use crate::a11y::{Control, Overlay};
    use accesskit::Role;
    let item = |b: usize, i: usize| {
        ed.buckets
            .get(b)
            .and_then(|bk| bk.get(i))
            .map_or(String::new(), name)
    };
    let place = |b: usize| places.get(b).copied().unwrap_or("");
    let button = |label: String, rect: Option<[f32; 4]>, disabled: bool| Control {
        label,
        role: Role::Button,
        toggled: None,
        selected: false,
        rect,
        disabled,
    };
    let mut controls: Vec<Control> = layout
        .hits
        .iter()
        .map(|(r, t)| {
            let label = match t {
                Target::Palette(x) => format!("Add {}", name(x)),
                Target::Item(b, i) => format!(
                    "{}, {} in {}{}",
                    item(*b, *i),
                    i + 1,
                    place(*b),
                    if ed.focus == Some((*b, *i)) {
                        ", focused"
                    } else {
                        ""
                    }
                ),
                Target::Remove(b, i) => format!("Remove {}", item(*b, *i)),
                Target::Restore => "Restore default".into(),
                Target::Cancel => "Cancel".into(),
                Target::Save => "Save changes".into(),
            };
            button(label, Some(*r), false)
        })
        .collect();
    if ed.is_default() {
        controls.push(button("Restore default".into(), None, true));
    }
    if !ed.changed() {
        controls.push(button("Save changes".into(), None, true));
    }
    let status = ed
        .buckets
        .iter()
        .enumerate()
        .map(|(b, bk)| {
            let names: Vec<String> = bk.iter().map(name).collect();
            let names = if names.is_empty() {
                "none".to_string()
            } else {
                names.join(", ")
            };
            if ed.buckets.len() > 1 {
                format!("{}: {names}", place(b))
            } else {
                names
            }
        })
        .collect::<Vec<_>>()
        .join("; ");
    Overlay::Dialog {
        label: title.into(),
        input: None,
        status: Some(format!("{list}: {status}")),
        controls,
        rect: Some(layout.card),
    }
}

/// One item in rect `r` (`trail` px of it kept for the × after it): its
/// frame when the arranger draws it, the hover lift, the focus ring.
#[allow(clippy::too_many_arguments)]
fn draw_item<T: Copy>(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    look: &Look<'_, T>,
    r: [f32; 4],
    item: T,
    trail: f32,
    hot: bool,
    focused: bool,
) {
    if look.framed {
        layer.bordered(
            [r[0], r[1]],
            [r[2], r[3]],
            if hot {
                lift(theme, 0.1)
            } else {
                lift(theme, 0.06)
            },
            lift(theme, if hot { 0.3 } else { 0.14 }),
            5.0,
            1.0,
        );
        (look.draw)(layer, engine, theme, [r[0] + 10.0, r[1]], item, trail);
    } else {
        if hot {
            layer.rect(
                [r[0] - 2.0, r[1] - 2.0],
                [r[2] + 4.0, r[3] + 4.0],
                lift(theme, 0.07),
                5.0,
            );
        }
        (look.draw)(layer, engine, theme, [r[0], r[1]], item, trail);
    }
    if focused {
        layer.bordered(
            [r[0] - 3.0, r[1] - 3.0],
            [r[2] + 6.0, r[3] + 6.0],
            Color::TRANSPARENT,
            theme.accent,
            6.0,
            1.5,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Arranger<char> {
        Arranger::new(
            vec![vec!['a'], vec!['b', 'c']],
            vec![vec!['a'], vec!['b', 'c']],
            vec!['a', 'b', 'c', 'd', 'e'],
        )
    }

    #[test]
    fn items_are_added_placed_removed_and_restored() {
        let mut ed = two();
        assert!(!ed.changed() && ed.is_default());
        assert_eq!(ed.palette(), ['d', 'e']);
        ed.add('d', 1);
        ed.add('d', 0);
        assert_eq!(ed.buckets, [vec!['a'], vec!['b', 'c', 'd']]);
        // across buckets: it leaves the old one
        ed.place('c', 0, 0);
        assert_eq!(ed.buckets, [vec!['c', 'a'], vec!['b', 'd']]);
        ed.place('a', 1, 99);
        assert_eq!(ed.buckets, [vec!['c'], vec!['b', 'd', 'a']]);
        ed.remove(1, 0);
        assert!(ed.changed() && ed.palette().contains(&'b'));
        ed.restore_default();
        assert!(!ed.changed() && ed.is_default());
        // saved and default can differ: changed is against what is saved
        let mut ed = Arranger::new(
            vec![vec!['e'], vec![]],
            vec![vec!['a'], vec![]],
            vec!['a', 'e'],
        );
        assert!(!ed.changed() && !ed.is_default());
        ed.restore_default();
        assert!(ed.changed() && ed.is_default());
    }

    #[test]
    fn arrows_walk_the_buckets_in_reading_order() {
        let mut ed = two();
        // a: left of b and c; → leaves the first bucket for the second's start
        assert_eq!(ed.step(0, 0, 1), (1, 0));
        assert_eq!(ed.buckets, [vec![], vec!['a', 'b', 'c']]);
        assert_eq!(ed.step(1, 0, 1), (1, 1));
        assert_eq!(ed.buckets, [vec![], vec!['b', 'a', 'c']]);
        assert_eq!(ed.step(1, 1, 1), (1, 2));
        // the end of the whole order holds
        assert_eq!(ed.step(1, 2, 1), (1, 2));
        assert_eq!(ed.buckets, [vec![], vec!['b', 'c', 'a']]);
        // ← from the second bucket's start enters the first's end
        assert_eq!(ed.step(1, 0, -1), (0, 0));
        assert_eq!(ed.buckets, [vec!['b'], vec!['c', 'a']]);
        assert_eq!(ed.step(0, 0, -1), (0, 0));
    }

    #[test]
    fn the_keyboard_tabs_moves_and_deletes() {
        let mut ed = two();
        assert_eq!(ed.key(Key::Right), Outcome::Stay);
        assert_eq!(ed.buckets, two().buckets, "nothing focused moves nothing");
        ed.key(Key::Tab { back: false });
        assert_eq!(ed.focus, Some((0, 0)));
        ed.key(Key::Tab { back: true });
        assert_eq!(ed.focus, Some((1, 1)));
        ed.key(Key::Left);
        assert_eq!(
            (ed.focus, ed.buckets.clone()),
            (Some((1, 0)), vec![vec!['a'], vec!['c', 'b']])
        );
        ed.key(Key::Delete);
        assert_eq!(
            (ed.focus, ed.buckets.clone()),
            (Some((1, 0)), vec![vec!['a'], vec!['b']])
        );
        ed.key(Key::Delete);
        assert_eq!(
            (ed.focus, ed.buckets.clone()),
            (None, vec![vec!['a'], vec![]])
        );
        assert_eq!(ed.key(Key::Escape), Outcome::Cancel);
        assert_eq!(ed.key(Key::Enter), Outcome::Save);
    }

    #[test]
    fn drops_land_in_reading_order() {
        // two rows: a b / c
        let slots = [
            [0.0, 0.0, 40.0, 18.0],
            [50.0, 0.0, 40.0, 18.0],
            [0.0, 26.0, 40.0, 18.0],
        ];
        assert_eq!(drop_index(&slots, None, (5.0, 5.0)), 0);
        assert_eq!(drop_index(&slots, None, (30.0, 5.0)), 1);
        assert_eq!(drop_index(&slots, None, (95.0, 5.0)), 2);
        // a lower row: before its first item past the pointer
        assert_eq!(drop_index(&slots, None, (5.0, 30.0)), 2);
        assert_eq!(drop_index(&slots, None, (60.0, 30.0)), 3);
        // dragging a: it does not count
        assert_eq!(drop_index(&slots, Some(0), (95.0, 5.0)), 1);
    }

    #[test]
    fn a_release_drops_clicks_and_removes() {
        let mut ed = two();
        let layout = Layout {
            card: [0.0, 0.0, 600.0, 400.0],
            hits: vec![
                ([0.0, 0.0, 40.0, 30.0], Target::Palette('d')),
                ([100.0, 100.0, 40.0, 30.0], Target::Item(0, 0)),
                ([300.0, 100.0, 40.0, 30.0], Target::Item(1, 0)),
                ([350.0, 100.0, 40.0, 30.0], Target::Item(1, 1)),
            ],
            buckets: vec![[90.0, 90.0, 180.0, 60.0], [290.0, 90.0, 180.0, 60.0]],
            slots: vec![
                vec![[100.0, 100.0, 40.0, 30.0]],
                vec![[300.0, 100.0, 40.0, 30.0], [350.0, 100.0, 40.0, 30.0]],
            ],
        };
        // a palette click adds to the given bucket
        ed.press(&layout, (10.0, 10.0));
        ed.release(&layout, (10.0, 10.0), 1);
        assert_eq!(ed.buckets, [vec!['a'], vec!['b', 'c', 'd']]);
        // a drag from the first bucket into the second, between b and c
        ed.press(&layout, (110.0, 110.0));
        ed.moved((330.0, 110.0));
        ed.release(&layout, (330.0, 110.0), 1);
        assert_eq!(ed.buckets, [vec![], vec!['b', 'a', 'c', 'd']]);
        assert_eq!(ed.focus, Some((1, 1)));
        // a bucket item dragged out of every bucket leaves
        ed.press(&layout, (310.0, 110.0));
        ed.moved((500.0, 300.0));
        ed.release(&layout, (500.0, 300.0), 1);
        assert_eq!(ed.buckets, [vec![], vec!['a', 'c', 'd']]);
        // a press without travel on a bucket item focuses it, moves nothing
        ed.press(&layout, (355.0, 110.0));
        ed.release(&layout, (355.0, 110.0), 1);
        assert_eq!(
            (ed.focus, ed.buckets.clone()),
            (Some((1, 1)), vec![vec![], vec!['a', 'c', 'd']])
        );
    }
}
