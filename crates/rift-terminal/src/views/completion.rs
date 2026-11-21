use rift_core::completion::{Completion, MenuMatch};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

/// How a row matches the word being completed, and its matched
/// characters (char indices, drawn bold); `None` for a history line.
pub type RowMatch = (Option<MenuMatch>, Vec<usize>);

/// Live completion panel state for the active editor.
#[derive(Default)]
pub struct CompletionUi {
    /// The rows shown.
    pub items: Vec<Completion>,
    /// Per row of `items`: how it matches the word.
    pub marks: Vec<RowMatch>,
    pub selected: usize,
    pub open: bool,
    /// Remainder previewed inline after the cursor.
    pub ghost: String,
    /// The word the candidates replace, as typed.
    pub token: String,
    /// The user asked for the panel: Tab opened it or ↓ moved into it, so
    /// ↑ / ↓ navigate it. A panel opened as you type gives ↑ to the history
    /// menu instead.
    pub asked: bool,
    /// Open as the inline history menu (↑ on the command line's first
    /// line, Warp): the rows are history lines, newest first, and the
    /// selected one previews in the command line. Holds what was typed and
    /// the cursor, put back when the menu closes past the newest line or on
    /// Esc.
    pub history_menu: Option<(String, usize)>,
    /// The word candidates of the last answer, best first (no history
    /// lines): what Tab completes from.
    pub cands: Vec<Completion>,
    /// The menu Tab opened: typing filters its result set (Warp).
    pub tab: Option<TabMenu>,
}

/// A menu Tab opened: where its word starts, the line up to the cursor
/// when it opened (an edit that no longer starts with it closes the menu),
/// and the candidates it filters.
#[derive(Clone, Debug, Default)]
pub struct TabMenu {
    pub start: usize,
    pub typed: String,
    pub all: Vec<Completion>,
}

impl CompletionUi {
    pub fn close(&mut self) {
        self.close_menu();
        self.ghost.clear();
        self.cands.clear();
        self.history_menu = None;
    }

    /// Close the menu, keeping the ghost text and the candidates.
    pub fn close_menu(&mut self) {
        self.open = false;
        self.items.clear();
        self.marks.clear();
        self.selected = 0;
        self.asked = false;
        self.tab = None;
    }

    /// Open as the history menu on `lines` (newest first) for the typed
    /// `typed` with the cursor at `cursor`; the newest is selected.
    /// `false` (and nothing opens) when there are none.
    pub fn open_history_menu(&mut self, lines: Vec<String>, typed: &str, cursor: usize) -> bool {
        self.close();
        if lines.is_empty() {
            return false;
        }
        self.show(
            lines
                .into_iter()
                .map(|insert| {
                    (
                        Completion {
                            insert,
                            description: String::new(),
                            source: "history",
                        },
                        (None, Vec::new()),
                    )
                })
                .collect(),
        );
        self.token = typed.to_string();
        self.history_menu = Some((typed.to_string(), cursor));
        true
    }

    /// The line the history menu previews.
    pub fn history_line(&self) -> Option<&str> {
        self.history_menu.as_ref()?;
        self.items.get(self.selected).map(|c| c.insert.as_str())
    }

    /// ↑ in the history menu: an older line (the oldest stays).
    pub fn history_older(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    /// ↓ in the history menu: a newer line; `false` past the newest, where
    /// the menu closes and the typed text comes back.
    pub fn history_newer(&mut self) -> bool {
        match self.selected.checked_sub(1) {
            Some(i) => {
                self.selected = i;
                true
            }
            None => false,
        }
    }

    /// Close the history menu, handing back what was typed and the cursor.
    pub fn take_typed(&mut self) -> Option<(String, usize)> {
        let typed = self.history_menu.take();
        self.close();
        typed
    }

    /// Show `rows` (the first selected); the menu opens when there are any.
    pub fn show(&mut self, rows: Vec<(Completion, RowMatch)>) {
        (self.items, self.marks) = rows.into_iter().unzip();
        self.selected = 0;
        self.open = !self.items.is_empty();
    }

    /// The rows of `all` matching `word` (Warp's order, see
    /// [`rift_core::completion::filter_by_query`]); a hint row (a note,
    /// not a candidate) rides along at the end when any matches.
    pub fn rows_for(all: &[Completion], word: &str) -> Vec<(Completion, RowMatch)> {
        let mut rows: Vec<(Completion, RowMatch)> =
            rift_core::completion::filter_by_query(all, word)
                .into_iter()
                .filter(|(i, _, _)| all[*i].source != "hint")
                .map(|(i, kind, at)| (all[i].clone(), (Some(kind), at)))
                .collect();
        if !rows.is_empty() {
            rows.extend(
                all.iter()
                    .filter(|c| c.source == "hint")
                    .map(|c| (c.clone(), (None, Vec::new()))),
            );
        }
        rows
    }

    /// Typing with the Tab menu open: its result set narrowed to `word`,
    /// the selection kept where it can be. `false` (the menu closes) when
    /// nothing matches.
    pub fn refilter(&mut self, word: &str) -> bool {
        let Some(tab) = self.tab.take() else {
            return false;
        };
        let keep = self.selected;
        let rows = Self::rows_for(&tab.all, word);
        if rows.iter().all(|(c, _)| c.source == "hint") {
            self.close_menu();
            return false;
        }
        self.show(rows);
        self.selected = keep.min(self.items.len().saturating_sub(1));
        self.token = word.to_string();
        self.tab = Some(tab);
        true
    }

    /// The next row, wrapping (Tab, ↓ in a menu Tab opened).
    pub fn select_next(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    /// The previous row, wrapping (⇧Tab).
    pub fn select_prev(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + self.items.len() - 1) % self.items.len();
        }
    }

    /// The rows matching the word as typed, for Tab's common prefix.
    pub fn typed_rows(&self) -> Vec<(&str, MenuMatch)> {
        self.items
            .iter()
            .zip(&self.marks)
            .filter_map(|(c, (k, _))| Some((c.insert.as_str(), (*k)?)))
            .collect()
    }
}

/// The history menu's lines for the typed `typed`: those starting with it,
/// Rift's saved history ranked by directory (`ranked`, best first) first,
/// then the pane's own history, then the shell's (both oldest first, as
/// kept), newest first and each line once; never the typed text itself (a
/// preview that changes nothing), at most [`MAX_HISTORY_MENU`] lines.
pub fn history_matches(
    ranked: &[&str],
    pane: &[String],
    shell: &[String],
    typed: &str,
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ranked
        .iter()
        .copied()
        .chain(pane.iter().rev().map(String::as_str))
        .chain(shell.iter().rev().map(String::as_str))
        .filter(|l| !l.trim().is_empty() && *l != typed && l.starts_with(typed))
        .filter(|l| seen.insert(*l))
        .take(MAX_HISTORY_MENU)
        .map(str::to_string)
        .collect()
}

/// Lines the history menu walks through with ↑ (the shell history alone can
/// hold tens of thousands).
pub const MAX_HISTORY_MENU: usize = 500;

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 28.0;
const FOOTER_H: f32 = 30.0;
const MAX_ROWS: usize = 8;
const PAD_X: f32 = 16.0;
/// The description column starts after at most this many name cells: a
/// longer name (a deep path) is cut rather than push the column away.
const DESC_COL_MAX: usize = 40;

fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut b = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    b.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    b
}

fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// A key cap: the glyph on a hairline-bordered chip; returns its width.
pub fn key_cap(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    key: &str,
    x: f32,
    cy: f32,
) -> f32 {
    let kw = engine.measure(key, 10.5, FONT_SANS, Weight::MEDIUM);
    let w = (kw + 10.0).max(18.0);
    layer.bordered(
        [x, cy - 9.0],
        [w, 18.0],
        lift(theme, 0.05),
        lift(theme, 0.14),
        3.0,
        1.0,
    );
    layer.text(
        sans(engine, key, 10.5, Weight::MEDIUM),
        (x + (w - kw) / 2.0).round(),
        cy - 7.5,
        theme.text_dim,
    );
    w
}

/// Footer of a docked panel: key caps and what they do, from `x`.
pub fn key_hints(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    hints: &[(&[&str], &str)],
    x: f32,
    cy: f32,
    right: f32,
) {
    let mut kx = x;
    for (keys, label) in hints {
        let lw = engine.measure(label, 11.5, FONT_SANS, Weight::NORMAL);
        let keys_w: f32 = keys
            .iter()
            .map(|k| (engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0) + 4.0)
            .sum();
        if kx + keys_w + 2.0 + lw > right {
            break;
        }
        for k in keys.iter() {
            kx += key_cap(layer, engine, theme, k, kx, cy) + 4.0;
        }
        kx += 2.0;
        layer.text(
            sans(engine, label, 11.5, Weight::NORMAL),
            kx,
            cy - 7.8,
            theme.text_muted,
        );
        kx += lw + 16.0;
    }
}

/// The row icon of a candidate's source.
fn kind_icon(item: &Completion) -> Icon {
    match item.source {
        "path" if item.insert.ends_with('/') => Icon::Folder,
        "path" => Icon::File,
        "history" => Icon::History,
        "git" => Icon::Branch,
        "hint" => Icon::Info,
        _ => Icon::Terminal,
    }
}

/// Height of the open panel: the terminal output moves out from under it.
pub fn panel_height(ui: &CompletionUi) -> f32 {
    HEADER_H + ui.items.len().min(MAX_ROWS) as f32 * ROW_H + FOOTER_H
}

/// Which characters of a row's text `name` (`chars` of them) show bold:
/// a history line's typed part (`line_len` bytes), a candidate's matched
/// characters (`mark`), a hint's none.
fn bold_chars(
    item: &Completion,
    name: &str,
    mark: Option<&RowMatch>,
    line_len: usize,
) -> Vec<bool> {
    match item.source {
        "hint" => vec![false; name.chars().count()],
        "history" => name.char_indices().map(|(i, _)| i < line_len).collect(),
        _ => {
            let at = mark.map(|(_, at)| at.as_slice()).unwrap_or_default();
            (0..name.chars().count()).map(|i| at.contains(&i)).collect()
        }
    }
}

/// Draw the completion list as a panel docked to the editor, full pane
/// width (Warp's completion pane): a header naming it with its count, the
/// rows (matched characters bold), and a key-hint footer. It sits on the
/// editor zone's top edge (input at the bottom) or under it (input at the
/// top); the output moves out from under it (`terminal::beside_panel`).
/// `zone` = the editor rect, `pane` = the active pane's body.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    ui: &CompletionUi,
    zone: [f32; 4],
    pane: [f32; 4],
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    if !ui.open || ui.items.is_empty() {
        return;
    }
    let shown = ui.items.len().min(MAX_ROWS);
    let panel_h = panel_height(ui);
    let (x, w) = (pane[0], pane[2]);
    // above the editor when it sits at the pane bottom, else below it
    let at_bottom = zone[1] + zone[3] >= pane[1] + pane[3] - 2.0;
    let y = if at_bottom {
        (zone[1] - panel_h).max(pane[1])
    } else {
        zone[1] + zone[3]
    };

    layer.rect([x, y], [w, panel_h], theme.bg, 0.0);
    layer.hline(x, y, w, 1.0, lift(theme, 0.1));
    if !at_bottom {
        layer.hline(x, y + panel_h - 1.0, w, 1.0, lift(theme, 0.1));
    }
    layer.set_clip(Some(([x, y, w, panel_h], 0.0)));

    // header: what the panel lists, with its count
    let label = if ui.history_menu.is_some() {
        "HISTORY"
    } else {
        "COMPLETIONS"
    };
    let count = ui
        .items
        .iter()
        .filter(|c| c.source != "hint")
        .count()
        .to_string();
    let tx = x + 8.0;
    let lw = engine.measure(label, 11.0, FONT_SANS, Weight::SEMIBOLD) + 0.66 * label.len() as f32;
    let cw = engine.measure(&count, 11.0, FONT_SANS, Weight::MEDIUM);
    let tab_w = 10.0 + lw + 5.0 + cw + 10.0;
    layer.rect(
        [tx + 8.0, y + HEADER_H - 2.0],
        [tab_w - 16.0, 2.0],
        theme.accent,
        0.0,
    );
    let mut lb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 14.0));
    lb.set_text(
        &mut engine.font_system,
        label,
        &Attrs::new()
            .family(Family::Name(FONT_SANS))
            .weight(Weight::SEMIBOLD)
            .letter_spacing(0.06),
        Shaping::Advanced,
        None,
    );
    layer.text(
        lb,
        tx + 10.0,
        y + (HEADER_H - 14.0) / 2.0,
        theme.text_bright,
    );
    layer.text(
        sans(engine, &count, 11.0, Weight::MEDIUM),
        tx + 10.0 + lw + 5.0,
        y + (HEADER_H - 14.85) / 2.0,
        theme.text_muted,
    );
    layer.hline(x, y + HEADER_H, w, 1.0, theme.border);

    // rows
    let scroll = ui.selected.saturating_sub(MAX_ROWS - 1);
    let cell = engine
        .measure("0", 12.5, mono_family(), Weight::NORMAL)
        .max(1.0);
    let rows_top = y + HEADER_H + 1.0;
    // the history menu keeps the newest line nearest the command line:
    // above it, the rows run bottom up
    let menu = ui.history_menu.as_ref();
    let line_len = menu.map_or(0, |(typed, _)| typed.len());
    let bottom_up = menu.is_some() && at_bottom;
    // descriptions form a column just past the longest shown name (at most
    // DESC_COL_MAX cells in), so each sits beside its name rather than at
    // the panel's far edge
    let longest = ui
        .items
        .iter()
        .skip(scroll)
        .take(shown)
        .filter(|i| i.source != "hint")
        .map(|i| i.insert.chars().count())
        .max()
        .unwrap_or(0)
        .min(DESC_COL_MAX);
    let desc_col = x + PAD_X + 14.0 + 12.0 + longest as f32 * cell + 28.0;
    for (row, item) in ui.items.iter().skip(scroll).take(shown).enumerate() {
        let idx = row + scroll;
        let slot = if bottom_up { shown - 1 - row } else { row };
        let ry = rows_top + slot as f32 * ROW_H;
        let rect = [x, ry, w, ROW_H];
        let selected = idx == ui.selected;
        let hot = mouse.is_some_and(|(mx, my)| {
            mx >= rect[0] && mx < rect[0] + rect[2] && my >= rect[1] && my < rect[1] + rect[3]
        });
        if selected {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.07), 0.0);
        } else if hot {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.04), 0.0);
        }
        hits.push(Hit {
            rect,
            target: ClickTarget::CompletionRow(idx),
        });
        icons::draw(
            layer,
            kind_icon(item),
            [x + PAD_X + 7.0, ry + ROW_H / 2.0],
            14.0,
            theme.text_dim,
        );

        // the name, then its description (muted) in the column after the
        // names, or ending at the panel's edge when it is too long for
        // that; a hint row is its text alone
        let hint = item.source == "hint";
        let desc_w = if item.description.is_empty() || hint {
            0.0
        } else {
            engine.measure(&item.description, 12.0, FONT_SANS, Weight::NORMAL)
        };
        let name_x = x + PAD_X + 14.0 + 12.0;
        let desc_x = if desc_w > 0.0 {
            desc_col.min(x + w - PAD_X - desc_w)
        } else {
            x + w - PAD_X
        };
        let name_room = (desc_x - 16.0 - name_x).max(cell * 4.0);
        let fits = (name_room / cell).floor().max(1.0) as usize;
        // a history line is drawn like ⌃R draws it: on one line, secrets
        // masked while `mask_secrets` is on (accepting keeps the real one)
        let shown_history;
        let name = if hint {
            &item.description
        } else if item.source == "history" {
            shown_history =
                super::history::masked_line(&item.insert, rift_core::secrets::active().as_deref());
            &shown_history
        } else {
            &item.insert
        };
        let chars = name.chars().count();
        // too long (a deep path): keep the tail, which tells candidates
        // apart (its matched characters are not marked then)
        let runs: Vec<(String, bool)> = if chars > fits {
            let tail: String = name.chars().skip(chars - fits.saturating_sub(1)).collect();
            vec![(format!("…{tail}"), false)]
        } else {
            let bold = bold_chars(item, name, ui.marks.get(idx), line_len);
            let mut runs: Vec<(String, bool)> = Vec::new();
            for (c, b) in name.chars().zip(bold) {
                match runs.last_mut() {
                    Some((text, on)) if *on == b => text.push(c),
                    _ => runs.push((c.to_string(), b)),
                }
            }
            runs
        };
        let plain = Attrs::new()
            .family(Family::Name(mono_family()))
            .color(text_color(if hint {
                theme.text_muted
            } else if selected {
                theme.text_bright
            } else {
                theme.text_ui
            }));
        let strong = Attrs::new()
            .family(Family::Name(mono_family()))
            .weight(Weight::BOLD)
            .color(text_color(theme.accent));
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(12.5, 16.0));
        let rich: Vec<(&str, Attrs)> = runs
            .iter()
            .map(|(t, b)| (t.as_str(), if *b { strong.clone() } else { plain.clone() }))
            .collect();
        buf.set_rich_text(
            &mut engine.font_system,
            rich,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(
            buf,
            name_x,
            ry + (ROW_H - 16.0) / 2.0,
            theme.text_ui,
            TextBounds {
                left: name_x as i32,
                top: ry as i32,
                right: (name_x + name_room) as i32,
                bottom: (ry + ROW_H) as i32,
            },
        );
        if desc_w > 0.0 && desc_x > name_x + 40.0 {
            let dbuf = sans(engine, &item.description, 12.0, Weight::NORMAL);
            layer.text(
                dbuf,
                desc_x,
                ry + (ROW_H - 16.2) / 2.0,
                if selected {
                    theme.text_dim
                } else {
                    theme.text_muted
                },
            );
        }
    }

    // footer: key caps and what they do
    let fy = y + panel_h - FOOTER_H;
    layer.hline(x, fy, w, 1.0, theme.border);
    let mut hints: Vec<(&[&str], &str)> = if menu.is_some() {
        vec![(&["↑", "↓"], "navigate"), (&["⏎"], "run"), (&["⇥"], "edit")]
    } else {
        vec![(&["↑", "↓"], "navigate"), (&["⏎"], "accept")]
    };
    if !ui.ghost.is_empty() {
        hints.push((&["→"], "accept ghost"));
    }
    hints.push((&["esc"], "dismiss"));
    key_hints(
        layer,
        engine,
        theme,
        &hints,
        x + PAD_X,
        fy + FOOTER_H / 2.0,
        x + w - PAD_X,
    );
    layer.set_clip(None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(insert: &str, source: &'static str) -> Completion {
        Completion {
            insert: insert.into(),
            description: String::new(),
            source,
        }
    }

    fn bold_at(item: &Completion, mark: Option<&RowMatch>, line_len: usize) -> Vec<usize> {
        let name = if item.source == "hint" {
            &item.description
        } else {
            &item.insert
        };
        bold_chars(item, name, mark, line_len)
            .into_iter()
            .enumerate()
            .filter_map(|(i, b)| b.then_some(i))
            .collect()
    }

    #[test]
    fn the_matched_characters_show_bold() {
        let row = |cand: &str, word: &str| {
            let item = c(cand, "shell");
            let (k, at) = rift_core::completion::menu_match(cand, word).unwrap();
            bold_at(&item, Some(&(Some(k), at)), 0)
        };
        assert_eq!(row("checkout", "che"), [0, 1, 2]);
        assert_eq!(row("Checkout", "che"), [0, 1, 2]);
        assert_eq!(row("git-checkout", "gco"), [0, 4, 9]);
        // a history line: what was typed
        assert_eq!(bold_at(&c("git che", "history"), None, 3), [0, 1, 2]);
        let hint = Completion {
            description: "note".into(),
            ..c("", "hint")
        };
        assert!(bold_at(&hint, None, 3).is_empty());
    }

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn history_matches_are_prefixed_newest_first_and_deduped() {
        let pane = lines(&["pnpm dev", "ls", "pnpm build", "pnpm dev"]);
        let shell = lines(&["pnpm install", "pnpm build", "  ", "pnpm d"]);
        // the typed text itself is no match: its preview would change nothing
        assert_eq!(
            history_matches(&[], &pane, &shell, "pnpm d"),
            lines(&["pnpm dev"])
        );
        let many: Vec<String> = (0..MAX_HISTORY_MENU + 50)
            .map(|i| format!("c{i}"))
            .collect();
        assert_eq!(
            history_matches(&[], &[], &many, "c").len(),
            MAX_HISTORY_MENU
        );
        // empty: every line, the pane's first
        assert_eq!(
            history_matches(&[], &pane, &shell, ""),
            lines(&["pnpm dev", "pnpm build", "ls", "pnpm d", "pnpm install"])
        );
        assert!(history_matches(&[], &pane, &shell, "git").is_empty());
    }

    #[test]
    fn history_matches_put_the_directory_ranked_lines_first() {
        let pane = lines(&["pnpm dev:web"]);
        let shell = lines(&["pnpm dev"]);
        assert_eq!(
            history_matches(&["pnpm dev", "git log"], &pane, &shell, "pnpm d"),
            lines(&["pnpm dev", "pnpm dev:web"])
        );
    }

    #[test]
    fn history_menu_walks_older_and_newer_and_hands_back_the_typed_text() {
        let mut ui = CompletionUi::default();
        assert!(!ui.open_history_menu(Vec::new(), "x", 1));
        assert!(!ui.open && ui.history_menu.is_none());

        assert!(ui.open_history_menu(lines(&["new", "mid", "old"]), "pn", 1));
        assert!(ui.open && !ui.asked);
        assert_eq!(ui.history_line(), Some("new"));
        ui.history_older();
        ui.history_older();
        assert_eq!(ui.history_line(), Some("old"));
        // the oldest stays
        ui.history_older();
        assert_eq!(ui.history_line(), Some("old"));
        assert!(ui.history_newer());
        assert!(ui.history_newer());
        assert_eq!(ui.history_line(), Some("new"));
        // past the newest: the caller restores
        assert!(!ui.history_newer());
        assert_eq!(ui.take_typed(), Some(("pn".to_string(), 1)));
        assert!(!ui.open && ui.items.is_empty() && ui.history_line().is_none());
        assert_eq!(ui.take_typed(), None);
    }

    fn rows(inserts: &[&str]) -> Vec<Completion> {
        inserts.iter().map(|i| c(i, "shell")).collect()
    }

    #[test]
    fn a_completion_panel_is_no_history_menu() {
        let mut ui = CompletionUi::default();
        ui.show(CompletionUi::rows_for(&rows(&["checkout"]), "ch"));
        assert!(ui.open);
        assert_eq!(ui.history_line(), None);
        // opening the menu replaces the panel; closing forgets both
        ui.asked = true;
        assert!(ui.open_history_menu(lines(&["git checkout"]), "git", 3));
        assert!(!ui.asked && ui.items.len() == 1);
        ui.close();
        assert!(ui.history_menu.is_none());
    }

    #[test]
    fn the_tab_menu_filters_its_results_as_you_type_and_wraps() {
        let mut ui = CompletionUi::default();
        let all = rows(&["checkout", "cherry-pick", "clean", "xcx"]);
        ui.show(CompletionUi::rows_for(&all, "c"));
        ui.tab = Some(TabMenu {
            start: 4,
            typed: "git c".into(),
            all,
        });
        assert_eq!(ui.items.len(), 4);
        // ⇧Tab from the first row wraps to the last, Tab back to the first
        ui.select_prev();
        assert_eq!(ui.selected, 3);
        ui.select_next();
        assert_eq!(ui.selected, 0);
        ui.select_next();
        assert!(ui.refilter("che"));
        let names: Vec<&str> = ui.items.iter().map(|c| c.insert.as_str()).collect();
        assert_eq!(names, ["checkout", "cherry-pick"]);
        // the selection stays where it can
        assert_eq!(ui.selected, 1);
        assert!(ui.refilter("checko"));
        assert_eq!(ui.selected, 0);
        assert_eq!(ui.typed_rows().len(), 1);
        // nothing matches: the menu closes
        assert!(!ui.refilter("zzz"));
        assert!(!ui.open && ui.tab.is_none());
        // a hint alone opens nothing
        let hint = vec![Completion {
            description: "Remote completions need an SSH profile".into(),
            ..c("", "hint")
        }];
        assert!(CompletionUi::rows_for(&hint, "x").is_empty());
        let mut with_hint = rows(&["main"]);
        with_hint.extend(hint);
        let got = CompletionUi::rows_for(&with_hint, "ma");
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].0.source, "hint");
    }
}
