//! "Edit prompt" (Warp): pick and order the Shift prompt's context chips.
//! The chips not in the prompt sit in a palette; the prompt's chips sit in a
//! dashed box, each with ×. Click a palette chip to append it, drag one into
//! the box to place it, drag a box chip to reorder it (or out of the box to
//! remove it). Restore default, Cancel and Save changes close it.

use shift_core::{Color, PromptChip, Theme};
use shift_renderer::glyphon::Weight;
use shift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::editor::{CHIP_H, chip_width, draw_chip};
use crate::views::icons::{self, Icon};
use crate::views::palette::{lift, sans};
use crate::views::prompt_chips::sample;

const MODAL_W: f32 = 600.0;
const PAD: f32 = 24.0;
const GAP: f32 = 8.0;
/// The × after a chip in the prompt box.
const X_W: f32 = 18.0;
/// Pointer travel that turns a press into a drag.
pub const DRAG_SLOP: f32 = 4.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub chip: PromptChip,
    /// Index in the draft when dragged from the prompt box.
    pub from: Option<usize>,
    pub press: (f32, f32),
    pub live: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptEditor {
    pub draft: Vec<PromptChip>,
    pub saved: Vec<PromptChip>,
    pub drag: Option<Drag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A palette chip (click: append).
    Palette(PromptChip),
    /// The prompt box's chip `i` (drag to reorder).
    Chip(usize),
    Remove(usize),
    Restore,
    Cancel,
    Save,
}

/// What `render` laid out: the card, the click targets, the prompt box and
/// its chips' rects (drop positions).
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub card: [f32; 4],
    pub hits: Vec<([f32; 4], Target)>,
    pub drop_box: [f32; 4],
    pub slots: Vec<[f32; 4]>,
}

impl PromptEditor {
    pub fn new(chips: &[PromptChip]) -> Self {
        PromptEditor {
            draft: chips.to_vec(),
            saved: chips.to_vec(),
            drag: None,
        }
    }

    pub fn changed(&self) -> bool {
        self.draft != self.saved
    }

    /// The chips not in the prompt, in palette order.
    pub fn palette(&self) -> Vec<PromptChip> {
        PromptChip::ALL
            .into_iter()
            .filter(|c| !self.draft.contains(c))
            .collect()
    }

    pub fn add(&mut self, chip: PromptChip) {
        if !self.draft.contains(&chip) {
            self.draft.push(chip);
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.draft.len() {
            self.draft.remove(i);
        }
    }

    /// Put `chip` at `at` (an index into the draft without it).
    pub fn place(&mut self, chip: PromptChip, at: usize) {
        self.draft.retain(|c| *c != chip);
        let at = at.min(self.draft.len());
        self.draft.insert(at, chip);
    }

    pub fn restore_default(&mut self) {
        self.draft = PromptChip::defaults();
    }
}

/// Where a chip dropped at `p` goes among the box's chips `slots` (the
/// dragged one, `skip`, left out): before the first chip it comes before in
/// reading order.
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

/// A dashed rectangle outline (Warp's drop box).
fn dashed(layer: &mut Layer, r: [f32; 4], color: Color) {
    let (dash, gap) = (6.0_f32, 4.0_f32);
    let mut x = r[0];
    while x < r[0] + r[2] {
        let w = dash.min(r[0] + r[2] - x);
        layer.rect([x, r[1]], [w, 1.0], color, 0.0);
        layer.rect([x, r[1] + r[3] - 1.0], [w, 1.0], color, 0.0);
        x += dash + gap;
    }
    let mut y = r[1];
    while y < r[1] + r[3] {
        let h = dash.min(r[1] + r[3] - y);
        layer.rect([r[0], y], [1.0, h], color, 0.0);
        layer.rect([r[0] + r[2] - 1.0, y], [1.0, h], color, 0.0);
        y += dash + gap;
    }
}

/// Wrap chips of widths `ws` into rows `width` wide from `(x, y)`; returns
/// each one's top-left and the height used.
fn flow(ws: &[f32], x: f32, y: f32, width: f32) -> (Vec<[f32; 2]>, f32) {
    let (mut cx, mut cy) = (x, y);
    let mut out = Vec::new();
    for &w in ws {
        if cx > x && cx + w > x + width {
            cx = x;
            cy += CHIP_H + GAP;
        }
        out.push([cx, cy]);
        cx += w + GAP;
    }
    let h = if ws.is_empty() { 0.0 } else { cy - y + CHIP_H };
    (out, h)
}

/// Draw the dialog centred in a `w`×`h` window over a dim backdrop.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ed: &PromptEditor,
    [w, h]: [f32; 2],
    mouse: (f32, f32),
) -> Layout {
    let mut out = Layout::default();
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.4), 0.0);
    let modal_w = MODAL_W.min((w - 32.0).max(240.0));
    let inner = modal_w - 2.0 * PAD;

    // palette and box contents, measured first for the card's height
    let palette = ed.palette();
    let pal_views: Vec<_> = palette.iter().map(|c| sample(*c, theme)).collect();
    let pal_ws: Vec<f32> = pal_views.iter().map(|v| chip_width(engine, v)).collect();
    let box_views: Vec<_> = ed.draft.iter().map(|c| sample(*c, theme)).collect();
    let box_ws: Vec<f32> = box_views
        .iter()
        .map(|v| chip_width(engine, v) + X_W)
        .collect();
    let (_, pal_h) = flow(&pal_ws, 0.0, 0.0, inner);
    let (_, box_h) = flow(&box_ws, 0.0, 0.0, inner - 24.0);
    let box_h = box_h.max(CHIP_H) + 24.0;
    let pal_h = if palette.is_empty() { 18.0 } else { pal_h };
    let modal_h = PAD + 24.0 + 16.0 + 22.0 + 10.0 + pal_h + 16.0 + box_h + 24.0 + 32.0 + PAD;
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
    layer.text(
        sans(engine, "Edit prompt", 16.0, Weight::SEMIBOLD),
        ix,
        cy,
        theme.text_bright,
    );
    cy += 24.0 + 16.0;

    // ── section: title, Restore default ──
    layer.text(
        sans(engine, "Shift prompt", 13.0, Weight::MEDIUM),
        ix,
        cy,
        theme.text_ui,
    );
    let restore = "Restore default";
    let rw = engine.measure(restore, 12.5, FONT_SANS, Weight::NORMAL);
    let rr = [ix + inner - rw - 8.0, cy - 3.0, rw + 8.0, 22.0];
    if inside(&rr, mouse) {
        layer.rect([rr[0], rr[1]], [rr[2], rr[3]], lift(theme, 0.06), 4.0);
    }
    layer.text(
        sans(engine, restore, 12.5, Weight::NORMAL),
        rr[0] + 4.0,
        cy,
        theme.accent,
    );
    out.hits.push((rr, Target::Restore));
    cy += 22.0 + 10.0;

    // ── palette ──
    let dragging = ed.drag.as_ref().filter(|d| d.live);
    if palette.is_empty() {
        layer.text(
            sans(
                engine,
                "Every chip is in your prompt.",
                12.5,
                Weight::NORMAL,
            ),
            ix,
            cy,
            theme.text_muted,
        );
    } else {
        let (spots, _) = flow(&pal_ws, ix, cy, inner);
        for ((v, at), w) in pal_views.iter().zip(&spots).zip(&pal_ws) {
            let r = [at[0], at[1], *w, CHIP_H];
            // the chip being dragged leaves a faint gap
            if dragging.is_some_and(|d| d.chip == v.chip && d.from.is_none()) {
                layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.04), 4.0);
                continue;
            }
            if inside(&r, mouse) && dragging.is_none() {
                layer.rect(
                    [r[0] - 2.0, r[1] - 2.0],
                    [r[2] + 4.0, r[3] + 4.0],
                    lift(theme, 0.07),
                    5.0,
                );
            }
            draw_chip(layer, engine, theme, *at, v, 0.0);
            out.hits.push((r, Target::Palette(v.chip)));
        }
    }
    cy += pal_h + 16.0;

    // ── the prompt: a dashed box of chips, each with × ──
    let bx = [ix, cy, inner, box_h];
    out.drop_box = bx;
    let over_box = dragging.is_some() && inside(&bx, mouse);
    layer.rect([bx[0], bx[1]], [bx[2], bx[3]], theme.bg_chrome, 4.0);
    dashed(
        layer,
        bx,
        if over_box {
            theme.accent
        } else {
            lift(theme, 0.25)
        },
    );
    let (spots, _) = flow(&box_ws, bx[0] + 12.0, bx[1] + 12.0, inner - 24.0);
    for (i, ((v, at), w)) in box_views.iter().zip(&spots).zip(&box_ws).enumerate() {
        let r = [at[0], at[1], *w, CHIP_H];
        out.slots.push(r);
        if dragging.is_some_and(|d| d.from == Some(i)) {
            layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.04), 4.0);
            continue;
        }
        draw_chip(layer, engine, theme, *at, v, X_W - 4.0);
        let xr = [r[0] + r[2] - X_W, r[1], X_W, CHIP_H];
        icons::draw(
            layer,
            Icon::Close,
            [xr[0] + X_W / 2.0 - 2.0, xr[1] + CHIP_H / 2.0],
            9.0,
            if inside(&xr, mouse) {
                theme.error
            } else {
                theme.text_muted
            },
        );
        out.hits.push((xr, Target::Remove(i)));
        out.hits
            .push(([r[0], r[1], r[2] - X_W, r[3]], Target::Chip(i)));
    }
    if ed.draft.is_empty() {
        layer.text(
            sans(
                engine,
                "Drag chips here, or click one above",
                12.5,
                Weight::NORMAL,
            ),
            bx[0] + 12.0,
            bx[1] + 12.0,
            theme.text_muted,
        );
    }
    // where a drop lands
    if over_box && let Some(d) = dragging {
        let at = drop_index(&out.slots, d.from, mouse);
        let others: Vec<[f32; 4]> = out
            .slots
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != d.from)
            .map(|(_, r)| *r)
            .collect();
        let (mx, my) = match others.get(at) {
            Some(r) => (r[0] - GAP / 2.0, r[1]),
            None => others.last().map_or((bx[0] + 12.0, bx[1] + 12.0), |r| {
                (r[0] + r[2] + GAP / 2.0, r[1])
            }),
        };
        layer.rect([mx - 1.0, my], [2.0, CHIP_H], theme.accent, 1.0);
    }
    cy += box_h + 24.0;

    // ── Cancel, Save changes ──
    let save_label = "Save changes";
    let sw = engine.measure(save_label, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
    let save = [ix + inner - sw, cy, sw, 30.0];
    let cw = engine.measure("Cancel", 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
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
    out.hits.push((cancel, Target::Cancel));
    let (fill, ink) = if ed.changed() {
        (theme.accent, theme.bg)
    } else {
        (lift(theme, 0.08), theme.text_muted)
    };
    layer.rect([save[0], save[1]], [save[2], save[3]], fill, 4.0);
    layer.text(
        sans(engine, save_label, 12.5, Weight::MEDIUM),
        save[0] + 12.0,
        cy + 7.0,
        ink,
    );
    if ed.changed() {
        out.hits.push((save, Target::Save));
    }

    // the dragged chip follows the pointer
    if let Some(d) = dragging {
        let v = sample(d.chip, theme);
        let cw = chip_width(engine, &v);
        let at = [mouse.0 - cw / 2.0, mouse.1 - CHIP_H / 2.0];
        layer.shadow(
            [at[0], at[1] + 4.0],
            [cw, CHIP_H],
            Color::BLACK.with_alpha(0.4),
            4.0,
            8.0,
        );
        layer.rect([at[0], at[1]], [cw, CHIP_H], theme.surface, 4.0);
        draw_chip(layer, engine, theme, at, &v, 0.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_are_added_placed_and_removed() {
        let mut ed = PromptEditor::new(&PromptChip::defaults());
        assert!(!ed.changed());
        assert!(!ed.palette().contains(&PromptChip::Cwd));
        ed.add(PromptChip::Time24);
        ed.add(PromptChip::Time24);
        assert_eq!(
            ed.draft,
            [
                PromptChip::Cwd,
                PromptChip::GitBranch,
                PromptChip::GitDiff,
                PromptChip::Time24
            ]
        );
        ed.place(PromptChip::Time24, 0);
        ed.place(PromptChip::Cwd, 99);
        assert_eq!(
            ed.draft,
            [
                PromptChip::Time24,
                PromptChip::GitBranch,
                PromptChip::GitDiff,
                PromptChip::Cwd
            ]
        );
        ed.remove(1);
        assert!(ed.changed() && ed.palette().contains(&PromptChip::GitBranch));
        ed.restore_default();
        assert!(!ed.changed());
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
        assert_eq!(drop_index(&slots, None, (5.0, 30.0)), 2);
        assert_eq!(drop_index(&slots, None, (60.0, 30.0)), 3);
        // dragging a: it does not count
        assert_eq!(drop_index(&slots, Some(0), (95.0, 5.0)), 1);
    }
}
