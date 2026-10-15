use rift_core::highlight::{TokenKind, tokenize};
use rift_core::{Color, Config, CursorStyle, PromptChip, PromptMode, Theme};
use rift_pty::alacritty_terminal::term::width::{Cluster, Placement};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight, Wrap};
use rift_renderer::{Layer, TextEngine, mono_family};

use crate::state::PaneState;
use crate::views::glyphs;
use crate::views::prompt_chips::{self, ChipView};
use crate::vim::VimMode;

/// Editor zone height for a single-line command (13px top pad + 16px
/// context row + 8px gap + input line + 15px bottom pad, design 01).
pub const EDITOR_H: f32 = 72.0;
/// Compact per-pane editor inside splits (design 02: input line only).
pub const EDITOR_COMPACT_H: f32 = 42.0;
/// Input line height; every extra line of a multi-line command adds one.
pub const LINE_H: f32 = 20.0;
pub const FONT_SIZE: f32 = 13.0;
pub const PAD_X: f32 = 16.0;

/// A command line's syntax colors, in the editor and in block headers: the
/// command, its flags, quoted strings, operators and plain arguments.
pub fn syntax_color(theme: &Theme, kind: TokenKind) -> Color {
    match kind {
        TokenKind::Command => theme.syntax.command,
        TokenKind::Flag => theme.syntax.flag,
        TokenKind::Str => theme.syntax.string,
        TokenKind::Op => theme.text_muted,
        TokenKind::Plain => theme.fg,
    }
}

/// Visual rows of the buffer: logical lines (split at `\n`) soft-wrapped
/// every `cols` cells. Byte ranges exclude the newline.
pub fn wrap_rows(buffer: &str, cols: usize) -> Vec<std::ops::Range<usize>> {
    let cols = cols.max(4);
    let mut rows = Vec::new();
    let mut line_start = 0;
    for line in buffer.split('\n') {
        let mut start = line_start;
        let mut used = 0;
        // cells as the terminal counts them: a cluster (combining marks,
        // ZWJ emoji, flags, VS16) never splits across rows
        let mut cluster = Cluster::default();
        for (i, c) in line.char_indices() {
            match cluster.next(c) {
                Some(Placement::New(w)) => {
                    if used + w > cols && used > 0 {
                        rows.push(start..line_start + i);
                        start = line_start + i;
                        used = 0;
                    }
                    used += w;
                }
                Some(Placement::JoinWiden) => used += 1,
                Some(Placement::JoinNarrow) => used = used.saturating_sub(1),
                Some(Placement::Join) | None => {}
            }
        }
        rows.push(start..line_start + line.len());
        line_start += line.len() + 1;
    }
    rows
}

/// Index of the visual row holding byte offset `at` (a soft-wrap boundary
/// belongs to the row it starts).
pub fn row_of(rows: &[std::ops::Range<usize>], at: usize) -> usize {
    rows.iter()
        .rposition(|r| r.start <= at && at <= r.end)
        .unwrap_or(0)
}

/// Input columns available in an editor zone `area_w` wide.
pub fn input_cols(area_w: f32, cell_w: f32, char_w: f32, vim: bool) -> usize {
    let text_left = PAD_X + cell_w * (FONT_SIZE / 13.0) + 11.0;
    let reserve = PAD_X + 10.0 + if vim { 80.0 } else { 0.0 };
    ((area_w - text_left - reserve) / char_w.max(1.0))
        .floor()
        .max(4.0) as usize
}

/// Editor zone height for the given buffer (grows with ⇧⏎ newlines and
/// soft-wrapped rows) and `chip_rows` rows of prompt chips ([`chip_rows`]).
pub fn editor_height(buffer: &str, prompt: PromptMode, cols: usize, chip_rows: usize) -> f32 {
    let base = if prompt == PromptMode::Shell {
        EDITOR_COMPACT_H
    } else {
        EDITOR_H + chip_rows.saturating_sub(1) as f32 * CHIP_PITCH
    };
    base + (wrap_rows(buffer, cols).len().saturating_sub(1)) as f32 * LINE_H
}

/// Offset of the input line from the editor zone's top: below the prompt
/// context rows, or right under the divider in shell-prompt mode.
pub fn input_offset(prompt: PromptMode, chip_rows: usize) -> f32 {
    if prompt == PromptMode::Shell {
        11.0
    } else {
        CHIPS_TOP + 24.0 + chip_rows.saturating_sub(1) as f32 * CHIP_PITCH
    }
}

/// Offset of the prompt context (chip) row from the editor zone's top.
pub const CHIPS_TOP: f32 = 13.0;

/// Where the first line of the editor zone sits: the chip row, or the
/// input line in shell-prompt mode (which has no chips). The shell's
/// "Starting …" placeholder goes here, as in Warp.
pub fn first_row_offset(prompt: PromptMode) -> f32 {
    if prompt == PromptMode::Shell {
        input_offset(prompt, 1)
    } else {
        // centred on the chip box by eye: a line's glyphs sit below the
        // middle of its LINE_H box
        CHIPS_TOP + (CHIP_H - LINE_H) / 2.0 - 2.5
    }
}

/// Draw the owned input editor.
/// `area` = [x, y, w, h] of the editor zone.
/// Returns the (x, y) of the input line for anchoring popups, the
/// cursor's top-left (IME candidate window, preedit text), and the context
/// chips' rects (cwd, branch) when drawn.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    config: &Config,
    pane: &PaneState,
    elapsed: Option<std::time::Duration>,
    area: [f32; 4],
    cell_w: f32, // still sizes the ❯ prompt offset below
    blink_on: bool,
    focused: bool,
    ghost: &str,
    shell_env: Option<&rift_core::shell_env::ShellEnv>,
    hint: Option<&str>,
    cols: usize,
) -> ((f32, f32), (f32, f32), DrawnChips) {
    let [x, y, w, _h] = area;
    let mut chips = Vec::new();
    let shell_prompt = config.prompt == PromptMode::Shell;

    // divider (full width, design: border-top on -8px margins)
    layer.hline(x - 8.0, y, w + 16.0, 1.0, theme.border);

    let input_y = if shell_prompt {
        y + input_offset(config.prompt, 1)
    } else {
        // ── prompt context (design 01): ⇕ ~/cwd on ⎇ branch ↑n ──
        let views = pane_chip_views(config, theme, pane, elapsed);
        chips = draw_chips(
            layer,
            engine,
            theme,
            [x + PAD_X, x + w - PAD_X],
            y + CHIPS_TOP,
            &views,
        );
        let rows = chip_rows(engine, &views, w);
        y + input_offset(config.prompt, rows)
    };
    let font_size = FONT_SIZE;
    let line_h = LINE_H;
    let prompt_color = match pane.last_exit {
        Some(n) if n != 0 => theme.error,
        _ => theme.accent,
    };
    // the logo's chevron (exit status colors it); shell-prompt mode keeps
    // the shell's own PS1 on the grid right above, so it is a quiet one
    // the size of a capital letter, centred on the block cursor
    let chevron_h = if shell_prompt { 9.0 } else { 11.0 };
    prompt_chevron(
        layer,
        [x + PAD_X, input_y + CURSOR_TOP + CURSOR_H / 2.0],
        chevron_h,
        if shell_prompt || !focused {
            theme.text_muted
        } else {
            prompt_color
        },
    );

    let text_x = x + PAD_X + cell_w * (font_size / 13.0) + 11.0;
    let buffer = &pane.editor.buffer;
    let vim_normal = pane.editor.vim_mode() == Some(VimMode::Normal);

    // syntax-highlighted spans
    let tokens = tokenize(buffer);
    let color_of = |kind: TokenKind| syntax_color(theme, kind);

    let mut spans: Vec<(&str, Attrs)> = Vec::new();
    let mut pos = 0;
    for (range, kind) in &tokens {
        if range.start > pos {
            spans.push((
                &buffer[pos..range.start],
                mono_attrs().color(rift_renderer::text_color(theme.fg)),
            ));
        }
        spans.push((
            &buffer[range.clone()],
            mono_attrs().color(rift_renderer::text_color(color_of(*kind))),
        ));
        pos = range.end;
    }
    if pos < buffer.len() {
        spans.push((
            &buffer[pos..],
            mono_attrs().color(rift_renderer::text_color(theme.fg)),
        ));
    }

    // soft wrap: break the styled spans at every visual row boundary that
    // is not already a newline
    let rows = wrap_rows(buffer, cols);
    let breaks: Vec<usize> = rows
        .iter()
        .skip(1)
        .map(|r| r.start)
        .filter(|&s| s > 0 && buffer.as_bytes()[s - 1] != b'\n')
        .collect();
    let mut wrapped: Vec<(String, Attrs)> = Vec::with_capacity(spans.len());
    {
        let mut at = 0usize;
        for (text, attrs) in &spans {
            let end = at + text.len();
            // a break exactly at the span start goes before its text
            if breaks.contains(&at) {
                wrapped.push(("\n".to_string(), attrs.clone()));
            }
            let mut from = at;
            for &b in breaks.iter().filter(|&&b| b > at && b < end) {
                wrapped.push((buffer[from..b].to_string(), attrs.clone()));
                wrapped.push(("\n".to_string(), attrs.clone()));
                from = b;
            }
            wrapped.push((buffer[from..end].to_string(), attrs.clone()));
            at = end;
        }
    }

    // selection highlight (under the text), one band per covered row
    if let Some(sel) = pane.editor.selection() {
        for (row, r) in rows.iter().enumerate() {
            let (row_start, row_end) = (r.start, r.end);
            let (a, b) = (sel.start.max(row_start), sel.end.min(row_end));
            if a < b || (sel.start <= row_start && sel.end > row_end) {
                let x0 = engine.measure(
                    &buffer[row_start..a.max(row_start)],
                    font_size,
                    mono_family(),
                    Weight::NORMAL,
                );
                let x1 = engine.measure(
                    &buffer[row_start..b.max(row_start)],
                    font_size,
                    mono_family(),
                    Weight::NORMAL,
                );
                // a selected newline shows as a small tail
                let tail = if sel.end > row_end && buffer.as_bytes().get(row_end) == Some(&b'\n') {
                    6.0
                } else {
                    0.0
                };
                layer.rect(
                    [text_x + x0, input_y + row as f32 * line_h - 1.0],
                    [(x1 - x0 + tail).max(2.0), line_h],
                    theme.selection,
                    2.0,
                );
            }
        }
    }

    // unknown command words: wavy-ish red underline (Warp error highlight)
    if let Some(env) = shell_env
        && config.error_underline
    {
        for range in rift_core::shell_env::first_command_word_ranges(buffer) {
            let word = &buffer[range.clone()];
            // don't flag the word still being typed at the end of the line
            if range.end == buffer.len() && pane.editor.cursor == buffer.len() {
                continue;
            }
            if env.is_known(word) {
                continue;
            }
            let row = row_of(&rows, range.start);
            let row_start = rows[row].start;
            // a word split by the soft wrap is underlined on its first row
            let word_end = range.end.min(rows[row].end);
            let x0 = engine.measure(
                &buffer[row_start..range.start],
                font_size,
                mono_family(),
                Weight::NORMAL,
            );
            let x1 = engine.measure(
                &buffer[row_start..word_end],
                font_size,
                mono_family(),
                Weight::NORMAL,
            );
            let uy = input_y + row as f32 * line_h + font_size + 2.5;
            let mut ux = text_x + x0;
            let mut up = true;
            while ux < text_x + x1 {
                let seg = (text_x + x1 - ux).min(3.0);
                layer.rect(
                    [ux, if up { uy } else { uy + 1.2 }],
                    [seg, 1.3],
                    theme.error,
                    0.0,
                );
                ux += 3.0;
                up = !up;
            }
        }
    }

    if !buffer.is_empty() {
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(font_size, line_h));
        buf.set_wrap(&mut engine.font_system, Wrap::None);
        buf.set_rich_text(
            &mut engine.font_system,
            wrapped.iter().map(|(t, a)| (t.as_str(), a.clone())),
            &mono_attrs(),
            Shaping::Advanced,
            None,
        );
        layer.text(buf, text_x, input_y, theme.fg);
    }

    // cursor position: (visual row, the row's text before the cursor)
    let row = row_of(&rows, pane.editor.cursor);
    let row_start = rows[row].start;
    let row_prefix = &buffer[row_start..pane.editor.cursor];
    // exact shaped widths — cell-width × count drifts on long lines and
    // parks the cursor block on top of the previous glyph
    let prefix_w = if row_prefix.is_empty() {
        0.0
    } else {
        engine.measure(row_prefix, font_size, mono_family(), Weight::NORMAL)
    };
    let cursor_y = input_y + row as f32 * line_h;

    // inline ghost text (Warp): the dimmed completion continues the line
    // right at the cursor (its first glyph shows through the cursor), then
    // a small → chip names the accept key
    let ghost_on =
        !ghost.is_empty() && pane.editor.cursor == buffer.len() && config.autosuggestions;
    if ghost_on {
        let gx = text_x + prefix_w;
        let gw = engine.measure(ghost, font_size, mono_family(), Weight::NORMAL);
        let mut gbuf = Buffer::new(&mut engine.font_system, Metrics::new(font_size, line_h));
        gbuf.set_text(
            &mut engine.font_system,
            ghost,
            &mono_attrs(),
            Shaping::Advanced,
            None,
        );
        layer.text(gbuf, gx, cursor_y, theme.syntax.ghost);
        let key = "→";
        let kw = engine.measure(key, 11.0, rift_renderer::FONT_SANS, Weight::NORMAL);
        let chip = [
            gx + gw + 8.0,
            cursor_y + CURSOR_TOP + (CURSOR_H - 16.0) / 2.0,
            kw + 10.0,
            16.0,
        ];
        layer.bordered(
            [chip[0], chip[1]],
            [chip[2], chip[3]],
            Color::TRANSPARENT,
            theme.text_muted.with_alpha(0.6),
            4.0,
            1.0,
        );
        let mut kb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 16.0));
        kb.set_text(
            &mut engine.font_system,
            key,
            &Attrs::new().family(Family::Name(rift_renderer::FONT_SANS)),
            Shaping::Advanced,
            None,
        );
        layer.text(kb, (chip[0] + 5.0).round(), chip[1] - 0.5, theme.text_dim);
    }

    // correction / hint shown in an empty editor (Warp command corrections)
    if let Some(hint) = hint
        && buffer.is_empty()
    {
        let label = "Did you mean";
        let lw = engine.measure(label, 12.0, rift_renderer::FONT_SANS, Weight::NORMAL);
        let mut lb = Buffer::new(&mut engine.font_system, Metrics::new(12.0, line_h));
        lb.set_text(
            &mut engine.font_system,
            label,
            &Attrs::new().family(Family::Name(rift_renderer::FONT_SANS)),
            Shaping::Advanced,
            None,
        );
        layer.text(lb, text_x + 12.0, input_y + 1.0, theme.text_muted);
        let hx = text_x + 12.0 + lw + 8.0;
        let hw = engine.measure(hint, 13.0, mono_family(), Weight::NORMAL);
        layer.bordered(
            [hx - 6.0, input_y - 1.0],
            [hw + 12.0, 20.0],
            theme.accent.with_alpha(0.10),
            theme.accent.with_alpha(0.35),
            4.0,
            1.0,
        );
        layer.text(
            make_mono(engine, hint, 13.0, Weight::NORMAL),
            hx,
            input_y,
            theme.text_bright,
        );
        let keys = "→ accept";
        let mut kb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, line_h));
        kb.set_text(
            &mut engine.font_system,
            keys,
            &Attrs::new().family(Family::Name(rift_renderer::FONT_SANS)),
            Shaping::Advanced,
            None,
        );
        layer.text(kb, hx + hw + 16.0, input_y + 2.0, theme.text_muted);
    }

    // continuation markers for extra lines (⇧⏎)
    // (explicit newlines only; soft-wrapped rows carry no marker)
    for (i, r) in rows.iter().enumerate().skip(1) {
        if buffer.as_bytes()[r.start - 1] != b'\n' {
            continue;
        }
        layer.text(
            make_mono(engine, "…", font_size, Weight::NORMAL),
            x + PAD_X,
            input_y + i as f32 * line_h,
            theme.text_muted,
        );
    }

    // cursor: config style (block 8×18 / bar / underline), 1.1s steps
    // blink; vim NORMAL always draws a solid block. Translucent when a
    // glyph sits on it so the glyph (drawn above quads) stays readable.
    let over_glyph = pane.editor.cursor < buffer.len() || ghost_on;
    let style = if vim_normal {
        CursorStyle::Block
    } else {
        config.cursor_style
    };
    if blink_on || vim_normal {
        let (rect_pos, rect_size, alpha) = match style {
            CursorStyle::Block => (
                [text_x + prefix_w, cursor_y + CURSOR_TOP],
                [8.0, CURSOR_H],
                if over_glyph { 0.45 } else { 1.0 },
            ),
            CursorStyle::Bar => (
                [text_x + prefix_w - 1.0, cursor_y + CURSOR_TOP],
                [2.0, CURSOR_H],
                1.0,
            ),
            CursorStyle::Underline => (
                [text_x + prefix_w, cursor_y + CURSOR_TOP + CURSOR_H - 2.0],
                [8.0, 2.0],
                1.0,
            ),
        };
        layer.rect(rect_pos, rect_size, theme.cursor.with_alpha(alpha), 1.0);
    }

    // vim mode pill at the right edge of the input line
    if let Some(mode) = pane.editor.vim_mode() {
        let (label, color) = match mode {
            VimMode::Normal => ("NORMAL", theme.warning),
            VimMode::Insert => ("INSERT", theme.text_muted),
        };
        let tw = engine.measure(label, 10.5, rift_renderer::FONT_SANS, Weight::SEMIBOLD);
        let pw = tw + 14.0;
        let px = x + w - PAD_X - pw;
        layer.bordered(
            [px, cursor_y + 1.0],
            [pw, 16.0],
            color.with_alpha(0.12),
            color.with_alpha(0.35),
            4.0,
            1.0,
        );
        let mut b = Buffer::new(&mut engine.font_system, Metrics::new(10.5, 14.0));
        b.set_text(
            &mut engine.font_system,
            label,
            &Attrs::new()
                .family(Family::Name(rift_renderer::FONT_SANS))
                .weight(Weight::SEMIBOLD),
            Shaping::Advanced,
            None,
        );
        layer.text(b, px + 7.0, cursor_y + 2.0, color);
    }

    ((text_x, input_y), (text_x + prefix_w, cursor_y), chips)
}

/// Each drawn prompt chip and its rect.
pub(crate) type DrawnChips = Vec<(PromptChip, [f32; 4])>;

/// Where each prompt chip goes in a row `[x, right]` wide: (the chip as
/// shown, its x, its row, its width). A chip that does not fit the row
/// wraps to the next one, as in Warp; only a cwd wider than a whole row is
/// cut, keeping its tail (`…/project/dir`).
fn plan_chips(
    engine: &mut TextEngine,
    [x, right]: [f32; 2],
    views: &[ChipView],
) -> Vec<(ChipView, f32, usize, f32)> {
    let empty_w = CHIP_PAD + CHIP_ICON + 5.0 + CHIP_PAD;
    let cwd_room = (right - x - empty_w).max(80.0);
    let mut out: Vec<(ChipView, f32, usize, f32)> = Vec::new();
    let (mut cx, mut row) = (x, 0);
    for v in views {
        let mut shown = v.clone();
        if v.chip == PromptChip::Cwd
            && let Some((t, _)) = shown.texts.first_mut()
        {
            *t = elide_head(t, cwd_room, |s| {
                engine.measure(s, CHIP_SIZE, mono_family(), Weight::NORMAL)
            });
        }
        let w = chip_width(engine, &shown);
        let mut at = if cx == x { cx } else { cx + CHIP_GAP };
        if cx > x && at + w > right {
            row += 1;
            at = x;
        }
        out.push((shown, at, row, w));
        cx = at + w;
    }
    out
}

/// The prompt chips `pane` shows, its cwd shortened against `$HOME`.
pub(crate) fn pane_chip_views(
    config: &Config,
    theme: &Theme,
    pane: &PaneState,
    elapsed: Option<std::time::Duration>,
) -> Vec<ChipView> {
    let cwd = pane.session.cwd.lock().unwrap().clone();
    let cwd_display = cwd
        .as_deref()
        .map(shorten_home)
        .unwrap_or_else(|| "~".into());
    chip_views(config, theme, pane, elapsed, &cwd_display)
}

/// Rows the prompt chips take in an editor zone `zone_w` wide (1 when
/// there are none).
pub(crate) fn chip_rows(engine: &mut TextEngine, views: &[ChipView], zone_w: f32) -> usize {
    plan_chips(engine, [0.0, zone_w - 2.0 * PAD_X], views)
        .last()
        .map_or(1, |(_, _, row, _)| row + 1)
}

/// Prompt context above the input (design 01/04, 12px mono): the
/// `prompt_chips` that have something to show, each in a hairline chip with
/// its icon, wrapping onto more rows in a narrow pane ([`plan_chips`]).
/// Returns each drawn chip's rect.
pub(crate) fn draw_chips(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    span: [f32; 2],
    y: f32,
    views: &[ChipView],
) -> DrawnChips {
    let mut out = Vec::new();
    for (shown, at, row, w) in plan_chips(engine, span, views) {
        let top = y - 2.0 + row as f32 * CHIP_PITCH;
        draw_chip(layer, engine, theme, [at, top], &shown, 0.0);
        out.push((shown.chip, [at, top, w, CHIP_H]));
    }
    out
}

/// A chip's width: pad, icon, gap, its texts 6 apart, pad.
pub(crate) fn chip_width(engine: &mut TextEngine, v: &ChipView) -> f32 {
    let texts: f32 = v
        .texts
        .iter()
        .map(|(t, _)| engine.measure(t, CHIP_SIZE, mono_family(), Weight::NORMAL))
        .sum::<f32>()
        + 6.0 * v.texts.len().saturating_sub(1) as f32;
    CHIP_PAD + CHIP_ICON + 5.0 + texts + CHIP_PAD
}

/// One chip `CHIP_H` tall with its top-left at `at`; its border reaches
/// `trail` px further right (room for a × after it).
pub(crate) fn draw_chip(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    [at, top]: [f32; 2],
    v: &ChipView,
    trail: f32,
) {
    let w = chip_width(engine, v) + trail;
    layer.bordered(
        [at, top],
        [w, CHIP_H],
        theme.fg.with_alpha(0.04),
        theme.border,
        4.0,
        1.0,
    );
    glyphs::draw(
        layer,
        engine,
        v.glyph,
        [at + CHIP_PAD + CHIP_ICON / 2.0, top + CHIP_H / 2.0],
        CHIP_ICON,
        v.icon_color,
    );
    let mut tx = at + CHIP_PAD + CHIP_ICON + 5.0;
    for (t, c) in &v.texts {
        // the line box centred in the chip
        let ty = top + (CHIP_H - CHIP_SIZE * 1.4) / 2.0;
        layer.text(make_mono(engine, t, CHIP_SIZE, Weight::NORMAL), tx, ty, *c);
        tx += engine.measure(t, CHIP_SIZE, mono_family(), Weight::NORMAL) + 6.0;
    }
}

/// The configured prompt chips for `pane` (shown cwd `cwd`; `elapsed`: time
/// since its tab's first command).
pub(crate) fn chip_views(
    config: &Config,
    theme: &Theme,
    pane: &PaneState,
    elapsed: Option<std::time::Duration>,
    cwd: &str,
) -> Vec<ChipView> {
    let vars = pane.session.user_vars.lock().unwrap().clone();
    let (user, host) = prompt_chips::user_and_host();
    let chips = &config.prompt_chips;
    let bin = |name: &str| {
        vars.get(name)
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
    };
    let python = chips
        .contains(&PromptChip::Python)
        .then(|| {
            let cwd = pane.session.cwd.lock().unwrap().clone()?;
            rift_core::runtimes::python_version(&bin("RiftPythonBin")?, cwd.as_ref())
        })
        .flatten();
    let php = chips
        .contains(&PromptChip::Php)
        .then(|| rift_core::runtimes::php_version(&bin("RiftPhpBin")?))
        .flatten();
    prompt_chips::views(
        &config.prompt_chips,
        theme,
        &prompt_chips::Inputs {
            cwd,
            git: pane.git.as_ref(),
            user,
            host,
            vars: &vars,
            python,
            php,
            pr: pane.pr.as_ref(),
            docker: pane.docker.as_ref(),
            now: prompt_chips::LocalTime::now(),
            elapsed,
        },
    )
}

/// Context chips: text size, height, inner padding, icon size, gap.
const CHIP_SIZE: f32 = 12.0;
pub(crate) const CHIP_H: f32 = 18.0;
const CHIP_PAD: f32 = 5.0;
const CHIP_ICON: f32 = 12.0;
const CHIP_GAP: f32 = 6.0;
/// From one chip row's top to the next one's.
const CHIP_PITCH: f32 = CHIP_H + 6.0;

/// Block cursor height, and its top below the input line's top: it spans
/// the 13 px text's ascenders to descenders in the 20 px line. The prompt
/// chevron and the ghost text's → chip centre on it.
const CURSOR_H: f32 = 17.0;
const CURSOR_TOP: f32 = 2.0;

/// The logo's chevron (`>` of the app icon: 170 wide, 304 tall with its
/// round caps on the 1024 grid), `h` px tall overall, its left cap at
/// `at[0]`, vertically centred on `at[1]`. The stroke stays text-weight:
/// the logo's 96 units would be bold at prompt size.
fn prompt_chevron(layer: &mut Layer, at: [f32; 2], h: f32, color: Color) {
    let k = h / (304.0 + 96.0);
    let stroke = (96.0 * k).min(2.2);
    let (w, hh) = (170.0 * k, 152.0 * k);
    let left = at[0] + stroke / 2.0;
    let tip = [left + w, at[1]];
    layer.line([left, at[1] - hh], tip, stroke, color);
    layer.line([left, at[1] + hh], tip, stroke, color);
}

/// `text` cut from the front to fit `max_w` (`…` + its tail): for paths,
/// whose end says where you are.
pub(crate) fn elide_head(text: &str, max_w: f32, mut measure: impl FnMut(&str) -> f32) -> String {
    if measure(text) <= max_w {
        return text.to_string();
    }
    let mut start = 0;
    for (i, _) in text.char_indices().skip(1) {
        start = i;
        let candidate = format!("…{}", &text[start..]);
        if measure(&candidate) <= max_w {
            return candidate;
        }
    }
    format!("…{}", &text[start..])
}

fn mono_attrs() -> Attrs<'static> {
    Attrs::new().family(Family::Name(mono_family()))
}

fn make_mono(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.4));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new()
            .family(Family::Name(mono_family()))
            .weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

pub(crate) fn shorten_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && let Some(rest) = path.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elide_head_keeps_the_path_tail() {
        let w = |t: &str| t.chars().count() as f32;
        assert_eq!(elide_head("~/zzz/rift", 20.0, w), "~/zzz/rift");
        assert_eq!(elide_head("/private/tmp/a/b/c", 7.0, w), "…/a/b/c");
        assert_eq!(elide_head("abc", 0.0, w), "…c");
    }

    #[test]
    fn height_grows_per_line() {
        assert_eq!(editor_height("ls", PromptMode::Rift, 80, 1), EDITOR_H);
        assert_eq!(
            editor_height("a\nb\nc", PromptMode::Rift, 80, 1),
            EDITOR_H + 2.0 * LINE_H
        );
        // shell-prompt mode drops the context row
        assert_eq!(
            editor_height("ls", PromptMode::Shell, 80, 1),
            EDITOR_COMPACT_H
        );
        // a long line soft-wraps into extra rows
        assert_eq!(
            editor_height(&"x".repeat(25), PromptMode::Rift, 10, 1),
            EDITOR_H + 2.0 * LINE_H
        );
        // prompt chips wrapped onto a second row push the input down a row
        assert_eq!(
            editor_height("ls", PromptMode::Rift, 80, 2),
            EDITOR_H + CHIP_PITCH
        );
        assert_eq!(
            input_offset(PromptMode::Rift, 2),
            input_offset(PromptMode::Rift, 1) + CHIP_PITCH
        );
    }

    #[test]
    fn wrap_rows_split_lines_and_widths() {
        assert_eq!(wrap_rows("", 10), vec![0..0]);
        assert_eq!(wrap_rows("abcdefghij", 4), vec![0..4, 4..8, 8..10]);
        assert_eq!(wrap_rows("ab\ncd", 4), vec![0..2, 3..5]);
        // wide characters count two cells
        let s = "가나다";
        assert_eq!(wrap_rows(s, 4), vec![0..6, 6..9]);
        // a combining mark adds no cell; a ZWJ family is one wide cluster
        // and never splits across rows
        assert_eq!(wrap_rows("abce\u{301}", 4), vec![0..6]);
        let family = "ab\u{1f468}\u{200d}\u{1f469}";
        assert_eq!(wrap_rows(family, 4), vec![0..family.len()]);
        // the cursor on a wrap boundary sits at the start of the next row
        let rows = wrap_rows("abcdefgh", 4);
        assert_eq!(row_of(&rows, 4), 1);
        assert_eq!(row_of(&rows, 8), 1);
        assert_eq!(row_of(&rows, 0), 0);
        // end of a logical line stays on that line
        let rows = wrap_rows("ab\ncd", 4);
        assert_eq!(row_of(&rows, 2), 0);
        assert_eq!(row_of(&rows, 3), 1);
    }
}
