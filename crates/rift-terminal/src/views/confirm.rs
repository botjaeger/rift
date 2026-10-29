use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family};

use crate::views::icons::{self, Icon};

fn text(
    engine: &mut TextEngine,
    value: &str,
    size: f32,
    family: &'static str,
    weight: Weight,
) -> Buffer {
    let mut buffer = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buffer.set_text(
        &mut engine.font_system,
        value,
        &Attrs::new().family(Family::Name(family)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buffer
}

/// Longest prefix of `value` (plus `…`) that fits `max_w`.
fn elide(
    engine: &mut TextEngine,
    value: &str,
    size: f32,
    family: &'static str,
    max_w: f32,
) -> String {
    if engine.measure(value, size, family, Weight::NORMAL) <= max_w {
        return value.to_string();
    }
    let mut out = value.to_string();
    while out.pop().is_some() {
        let candidate = format!("{}…", out.trim_end());
        if engine.measure(&candidate, size, family, Weight::NORMAL) <= max_w {
            return candidate;
        }
    }
    String::new()
}

/// Greedy word wrap of `value` into lines no wider than `max_w`.
fn wrap(engine: &mut TextEngine, value: &str, size: f32, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for word in value.split_whitespace() {
        let next = if cur.is_empty() {
            word.to_string()
        } else {
            format!("{cur} {word}")
        };
        if cur.is_empty() || engine.measure(&next, size, FONT_SANS, Weight::NORMAL) <= max_w {
            cur = next;
        } else {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// What a status toast reports (drives its icon and how long it stays).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToastKind {
    /// It worked (copied, saved).
    Done,
    /// It failed: stays until dismissed.
    Error,
    Info,
    /// A paired phone did something here.
    Phone,
}

/// One toast as drawn.
pub struct ToastView<'a> {
    pub id: u64,
    pub kind: ToastKind,
    pub text: &'a str,
    /// Its button's label (Open config.toml, …).
    pub action: Option<&'a str>,
    /// Shows ✕ (an error stays until dismissed).
    pub sticky: bool,
}

/// Click targets of one toast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToastHit {
    pub id: u64,
    pub rect: [f32; 4],
    pub action: Option<[f32; 4]>,
    pub close: Option<[f32; 4]>,
}

/// A toast's height; they stack 6 px apart.
pub const TOAST_H: f32 = 30.0;

/// Status toasts as chips in the pane they are about (DESIGN.md "Toasts"),
/// right-aligned at `right` and stacked up from `bottom`, the newest
/// lowest; none wider than `max_w`. `items` is newest first.
pub fn toasts(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    items: &[ToastView],
    right: f32,
    bottom: f32,
    max_w: f32,
) -> Vec<ToastHit> {
    const SIZE: f32 = 12.5;
    const DISC: f32 = 18.0;
    let mut hits = Vec::new();
    let mut y = bottom - TOAST_H;
    for t in items {
        let (icon, color) = match t.kind {
            ToastKind::Done => (Icon::Check, theme.success),
            ToastKind::Error => (Icon::Alert, theme.error),
            ToastKind::Info => (Icon::Info, theme.accent),
            ToastKind::Phone => (Icon::Phone, theme.accent),
        };
        let action_w = t.action.map_or(0.0, |a| {
            engine.measure(a, SIZE, FONT_SANS, Weight::MEDIUM) + 10.0
        });
        let close_w = if t.sticky { 26.0 } else { 0.0 };
        // left pad, disc, gap … text … action, ✕ or right pad
        let chrome = 6.0 + DISC + 8.0 + action_w + if t.sticky { close_w + 2.0 } else { 12.0 };
        let text_w = engine.measure(t.text, SIZE, FONT_SANS, Weight::NORMAL);
        // +2: the elision measures a hair wider than the plain text
        let w = (chrome + text_w + 2.0).min(max_w.max(chrome + 40.0)).ceil();
        let x = (right - w).round();
        let r = TOAST_H / 2.0;
        layer.shadow(
            [x, y + 4.0],
            [w, TOAST_H],
            Color::BLACK.with_alpha(0.45),
            r,
            10.0,
        );
        let edge = if t.kind == ToastKind::Error {
            theme.error.with_alpha(0.45)
        } else {
            lift(theme, 0.12)
        };
        layer.bordered([x, y], [w, TOAST_H], theme.surface, edge, r, 1.0);
        let c = [x + 6.0 + DISC / 2.0, y + TOAST_H / 2.0];
        layer.circle(c, DISC, color.with_alpha(0.14));
        icons::draw(layer, icon, c, 11.0, color);
        let tx = x + 6.0 + DISC + 8.0;
        let room = w - chrome;
        let shown = elide(engine, t.text, SIZE, FONT_SANS, room);
        let ty = y + (TOAST_H - SIZE * 1.35) / 2.0;
        layer.text(
            text(engine, &shown, SIZE, FONT_SANS, Weight::NORMAL),
            tx,
            ty,
            theme.text_ui,
        );
        let mut hit = ToastHit {
            id: t.id,
            rect: [x, y, w, TOAST_H],
            action: None,
            close: None,
        };
        let mut ax = tx + room + 4.0;
        if let Some(a) = t.action {
            layer.text(
                text(engine, a, SIZE, FONT_SANS, Weight::MEDIUM),
                ax + 2.0,
                ty,
                theme.accent,
            );
            hit.action = Some([ax - 2.0, y, action_w + 2.0, TOAST_H]);
            ax += action_w;
        }
        if t.sticky {
            let cr = [ax, y + 4.0, 22.0, 22.0];
            super::close_icon(layer, [cr[0] + 11.0, cr[1] + 11.0], 3.5, theme.text_muted);
            hit.close = Some(cr);
        }
        hits.push(hit);
        y -= TOAST_H + 6.0;
    }
    hits
}

fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// Contents of the confirm modal.
pub struct Dialog<'a> {
    pub title: &'a str,
    pub description: String,
    pub note: &'a str,
    /// Up to three are listed (running commands, pasted lines).
    pub lines: &'a [String],
    /// Shown for a blank entry in `lines`.
    pub empty_line: &'a str,
    pub confirm_label: &'a str,
    /// The dismiss button (Esc): "Cancel", or "Later" for an update.
    pub cancel_label: &'a str,
    /// Red confirm button (close/quit) instead of the accent one.
    pub destructive: bool,
    /// `lines` are running processes: each row gets the program's badge
    /// (a coding agent's brand disc, else a terminal mark) and the agent's
    /// name, instead of plain code lines.
    pub processes: bool,
    /// The keyboard stands on the dismiss button (Tab / ←) instead of the
    /// confirm one: it gets the focus ring.
    pub focus_cancel: bool,
    /// A third button at the footer's left (Discard; ⌘⌫).
    pub extra_label: Option<&'a str>,
    /// The confirm button is shown but does nothing (Save while the text
    /// has errors).
    pub confirm_disabled: bool,
}

const ROW_PROCESS: f32 = 36.0;
const ROW_CODE: f32 = 20.0;
const FOOTER_H: f32 = 56.0;
const BUTTON_H: f32 = 30.0;

/// Confirmation modal (close with running processes, multiline paste,
/// revert, remote install, update): title and description, the affected lines in
/// an inset list, then a footer bar with Cancel (esc) and the confirm
/// button (⏎). Returns [cancel, confirm, extra button] hit rects (the extra
/// button's is empty without one).
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    dialog: &Dialog,
    w: f32,
    h: f32,
    scale: f32,
) -> [[f32; 4]; 3] {
    use rift_core::notifications::Agent;
    let commands = dialog.lines;
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.45), 0.0);

    let shown = commands.len().min(3);
    let more = commands.len() > shown;
    let modal_w = 460.0_f32.min((w - 32.0).max(160.0));
    let pad = 22.0;
    let inner_w = modal_w - 2.0 * pad;
    let desc_lines = wrap(engine, &dialog.description, 13.0, inner_w);
    let note_lines = wrap(engine, dialog.note, 12.0, inner_w);
    let row_h = if dialog.processes {
        ROW_PROCESS
    } else {
        ROW_CODE
    };
    let list_h = if shown > 0 {
        let pad_y = if dialog.processes { 0.0 } else { 16.0 };
        pad_y + shown as f32 * row_h + if more { 26.0 } else { 0.0 }
    } else {
        0.0
    };
    let body_h = 24.0
        + desc_lines.len() as f32 * 19.0
        + if note_lines.is_empty() {
            0.0
        } else {
            4.0 + note_lines.len() as f32 * 17.0
        }
        + if list_h > 0.0 { 16.0 + list_h } else { 0.0 };
    let modal_h = pad + body_h + pad + FOOTER_H;
    let x = ((w - modal_w) / 2.0).round();
    let y = ((h - modal_h) / 2.0).max(8.0).round();
    let card = [x, y, modal_w, modal_h];

    layer.shadow(
        [x, y + 16.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.5),
        12.0,
        32.0,
    );
    layer.bordered(
        [x, y],
        [modal_w, modal_h],
        theme.surface,
        lift(theme, 0.1),
        12.0,
        1.0,
    );

    // title, description, note
    let tx = x + pad;
    let title = elide(engine, dialog.title, 16.0, FONT_SANS, inner_w);
    layer.text(
        text(engine, &title, 16.0, FONT_SANS, Weight::SEMIBOLD),
        tx,
        y + pad,
        theme.text_bright,
    );
    let mut ty = y + pad + 26.0;
    for line in &desc_lines {
        layer.text(
            text(engine, line, 13.0, FONT_SANS, Weight::NORMAL),
            tx,
            ty,
            theme.text_dim,
        );
        ty += 19.0;
    }
    if !note_lines.is_empty() {
        ty += 4.0;
        for line in &note_lines {
            layer.text(
                text(engine, line, 12.0, FONT_SANS, Weight::NORMAL),
                tx,
                ty,
                theme.text_muted,
            );
            ty += 17.0;
        }
    }

    // the affected lines, in an inset list
    if shown > 0 {
        ty += 16.0;
        let list = [tx, ty, inner_w, list_h];
        layer.bordered(
            [list[0], list[1]],
            [list[2], list[3]],
            theme.bg.with_alpha(0.6),
            lift(theme, 0.07),
            8.0,
            1.0,
        );
        let mut ry = ty + if dialog.processes { 0.0 } else { 8.0 };
        for (i, command) in commands.iter().take(shown).enumerate() {
            let label = if command.trim().is_empty() {
                dialog.empty_line
            } else {
                command.trim()
            };
            if dialog.processes {
                if i > 0 {
                    layer.hline(tx + 1.0, ry, inner_w - 2.0, 1.0, lift(theme, 0.06));
                }
                let cy = ry + ROW_PROCESS / 2.0;
                let ic = [tx + 12.0 + 10.0, cy];
                let agent = Agent::from_command(label);
                match agent {
                    Some(a) => {
                        super::notifications::agent_disc(layer, engine, a, ic, 20.0, scale, card)
                    }
                    None => {
                        layer.circle(ic, 20.0, lift(theme, 0.08));
                        icons::draw(layer, Icon::Terminal, ic, 12.0, theme.text_dim);
                    }
                }
                let name_w = agent.map_or(0.0, |a| {
                    let nw = engine.measure(a.name(), 12.0, FONT_SANS, Weight::NORMAL);
                    layer.text(
                        text(engine, a.name(), 12.0, FONT_SANS, Weight::NORMAL),
                        tx + inner_w - 12.0 - nw,
                        cy - 8.0,
                        theme.text_muted,
                    );
                    nw + 12.0
                });
                let cmd_x = tx + 42.0;
                let clipped = elide(
                    engine,
                    label,
                    12.5,
                    mono_family(),
                    inner_w - 42.0 - 12.0 - name_w,
                );
                layer.text(
                    text(engine, &clipped, 12.5, mono_family(), Weight::NORMAL),
                    cmd_x,
                    cy - 8.5,
                    theme.text_bright,
                );
            } else {
                let clipped = elide(engine, label, 12.0, mono_family(), inner_w - 24.0);
                layer.text(
                    text(engine, &clipped, 12.0, mono_family(), Weight::NORMAL),
                    tx + 12.0,
                    ry + 2.0,
                    theme.text_ui,
                );
            }
            ry += row_h;
        }
        if more {
            if dialog.processes {
                layer.hline(tx + 1.0, ry, inner_w - 2.0, 1.0, lift(theme, 0.06));
            }
            let more = format!("and {} more…", commands.len() - shown);
            layer.text(
                text(engine, &more, 12.0, FONT_SANS, Weight::NORMAL),
                tx + 12.0,
                ry + 5.0,
                theme.text_muted,
            );
        }
    }

    // footer bar: a hairline, a slightly sunken strip, buttons on the right
    let fy = y + modal_h - FOOTER_H;
    layer.set_clip(Some((
        [x + 1.0, y + 1.0, modal_w - 2.0, modal_h - 2.0],
        11.0,
    )));
    layer.rect([x, fy], [modal_w, FOOTER_H], lift(theme, 0.025), 0.0);
    layer.set_clip(None);
    layer.hline(x + 1.0, fy, modal_w - 2.0, 1.0, lift(theme, 0.08));

    let by = fy + (FOOTER_H - BUTTON_H) / 2.0;
    let label_y = by + (BUTTON_H - 13.0 * 1.35) / 2.0;
    let hint_w =
        |engine: &mut TextEngine, k: &str| engine.measure(k, 11.5, FONT_SANS, Weight::NORMAL);
    let label_w = engine.measure(dialog.confirm_label, 13.0, FONT_SANS, Weight::MEDIUM);
    let confirm_w = 14.0 + label_w + 8.0 + hint_w(engine, "⏎") + 14.0;
    let cancel_lw = engine.measure(dialog.cancel_label, 13.0, FONT_SANS, Weight::MEDIUM);
    let cancel_w = 14.0 + cancel_lw + 8.0 + hint_w(engine, "esc") + 14.0;
    let confirm = [x + modal_w - pad - confirm_w, by, confirm_w, BUTTON_H];
    let cancel = [confirm[0] - 8.0 - cancel_w, by, cancel_w, BUTTON_H];

    layer.bordered(
        [cancel[0], cancel[1]],
        [cancel[2], cancel[3]],
        lift(theme, 0.05),
        lift(theme, 0.12),
        6.0,
        1.0,
    );
    layer.text(
        text(engine, dialog.cancel_label, 13.0, FONT_SANS, Weight::MEDIUM),
        cancel[0] + 14.0,
        label_y,
        theme.text_ui,
    );
    layer.text(
        text(engine, "esc", 11.5, FONT_SANS, Weight::NORMAL),
        cancel[0] + 14.0 + cancel_lw + 8.0,
        label_y + 1.5,
        theme.text_muted,
    );

    // red for discarding work (darkened so white text reads), else accent
    let (fill, ink) = if dialog.confirm_disabled {
        (lift(theme, 0.05), theme.text_muted)
    } else if dialog.destructive {
        (theme.error.mix(Color::BLACK, 0.18), Color::WHITE)
    } else {
        (theme.accent, theme.bg)
    };
    layer.rect(
        [confirm[0], confirm[1]],
        [confirm[2], confirm[3]],
        fill,
        6.0,
    );
    layer.text(
        text(
            engine,
            dialog.confirm_label,
            13.0,
            FONT_SANS,
            Weight::MEDIUM,
        ),
        confirm[0] + 14.0,
        label_y,
        ink,
    );
    layer.text(
        text(engine, "⏎", 11.5, FONT_SANS, Weight::NORMAL),
        confirm[0] + 14.0 + label_w + 8.0,
        label_y + 1.5,
        ink.with_alpha(0.7),
    );

    // keyboard focus (DESIGN.md): a 2 px accent ring, 2 px outside
    let at = if dialog.focus_cancel { cancel } else { confirm };
    layer.bordered(
        [at[0] - 2.0, at[1] - 2.0],
        [at[2] + 4.0, at[3] + 4.0],
        theme.accent.with_alpha(0.0),
        theme.accent,
        6.0,
        2.0,
    );

    // the third button (Discard) at the footer's left
    let mut check = [0.0; 4];
    if let Some(label) = dialog.extra_label {
        let lw = engine.measure(label, 13.0, FONT_SANS, Weight::MEDIUM);
        let hint = if cfg!(target_os = "macos") {
            "⌘⌫"
        } else {
            "Ctrl+⌫"
        };
        let ew = 14.0 + lw + 8.0 + hint_w(engine, hint) + 14.0;
        check = [x + pad, by, ew, BUTTON_H];
        layer.bordered(
            [check[0], check[1]],
            [check[2], check[3]],
            lift(theme, 0.05),
            lift(theme, 0.12),
            6.0,
            1.0,
        );
        layer.text(
            text(engine, label, 13.0, FONT_SANS, Weight::MEDIUM),
            check[0] + 14.0,
            label_y,
            theme.text_ui,
        );
        layer.text(
            text(engine, hint, 11.5, FONT_SANS, Weight::NORMAL),
            check[0] + 14.0 + lw + 8.0,
            label_y + 1.5,
            theme.text_muted,
        );
    }

    [cancel, confirm, check]
}
