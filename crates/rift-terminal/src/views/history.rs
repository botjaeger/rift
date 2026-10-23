//! Command history pane (⌃R): docked above the command line across the
//! pane, like the completion panel. The command line is the search field;
//! rows show whether the command succeeded, the command with the match
//! highlighted, where it ran and how long ago.

use std::time::SystemTime;

use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::state::{ClickTarget, Hit};
use crate::views::completion::key_hints;
use crate::views::icons::{self, Icon};

/// Which commands the pane lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    /// Commands run in the active pane.
    Pane,
    /// Every shell of this window plus the imported shell history.
    #[default]
    All,
    /// Commands that exited non-zero.
    Failed,
    /// Past runs by their output too, across every window's panes and the
    /// saved transcripts (`crate::results`).
    Results,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Pane, Scope::All, Scope::Failed, Scope::Results];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Pane => "This pane",
            Scope::All => "All shells",
            Scope::Failed => "Failed",
            Scope::Results => "Results",
        }
    }

    /// ⌃R while open: the next scope, wrapping.
    pub fn next(self) -> Scope {
        let i = Scope::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Scope::ALL[(i + 1) % Scope::ALL.len()]
    }
}

/// One remembered command.
#[derive(Clone, Debug)]
pub struct Entry {
    pub command: String,
    /// Where it ran (blocks of this window); `None` for imported history.
    pub cwd: Option<String>,
    /// Exit status; `None` when unknown (imported, or no status reported).
    pub exit: Option<i32>,
    pub finished: Option<SystemTime>,
    /// Ran in the active pane.
    pub here: bool,
}

#[derive(Default)]
pub struct HistoryPane {
    pub open: bool,
    pub scope: Scope,
    /// Newest first, one row per distinct command.
    pub entries: Vec<Entry>,
    /// Indices into `entries` matching the scope and query.
    pub results: Vec<usize>,
    pub selected: usize,
    /// The query the results were filtered with (the command line).
    pub query: String,
    /// The pane whose command line it searches; it closes when another
    /// pane takes focus.
    pub pane: Option<u64>,
    /// Results scope: the runs found for the latest answered query.
    pub found: Vec<rift_core::result_search::Found>,
    /// The latest search asked for (answers for older ones are dropped).
    pub results_seq: u64,
    /// The query that search was for.
    pub results_query: Option<String>,
    /// A search is running for the current query.
    pub searching: bool,
    /// The last answer stopped at a bound (older runs not searched).
    pub truncated: bool,
    /// Saved transcripts were searched too (`save_transcripts`).
    pub saved: bool,
}

impl HistoryPane {
    pub fn open(&mut self, entries: Vec<Entry>, query: &str) {
        self.entries = entries;
        self.open = true;
        self.selected = 0;
        self.filter(query);
    }

    pub fn close(&mut self) {
        *self = HistoryPane {
            scope: self.scope,
            // a late answer for this pane's search stays stale
            results_seq: self.results_seq,
            ..HistoryPane::default()
        };
    }

    /// The Results scope wants a search for the current query (none asked
    /// for it yet).
    pub fn wants_search(&self) -> bool {
        self.open
            && self.scope == Scope::Results
            && self.results_query.as_deref() != Some(self.query.as_str())
    }

    /// The worker's answer for search `seq`; `false` when it is stale.
    pub fn take_results(&mut self, seq: u64, outcome: rift_core::result_search::Outcome) -> bool {
        if seq != self.results_seq || !self.open {
            return false;
        }
        self.found = outcome.found;
        self.truncated = outcome.truncated;
        self.searching = false;
        self.selected = self.selected.min(self.found.len().saturating_sub(1));
        true
    }

    /// Rows listed in the current scope.
    pub fn len(&self) -> usize {
        if self.scope == Scope::Results {
            self.found.len()
        } else {
            self.results.len()
        }
    }

    /// The selected row's command.
    pub fn current_command(&self) -> Option<&str> {
        if self.scope == Scope::Results {
            self.found.get(self.selected).map(|f| f.command.as_str())
        } else {
            self.current().map(|e| e.command.as_str())
        }
    }

    /// The selected found run (Results scope).
    pub fn current_found(&self) -> Option<&rift_core::result_search::Found> {
        (self.scope == Scope::Results)
            .then(|| self.found.get(self.selected))
            .flatten()
    }

    pub fn set_scope(&mut self, scope: Scope) {
        self.scope = scope;
        let q = std::mem::take(&mut self.query);
        self.filter(&q);
    }

    /// Narrow to the scope and to commands containing `query` (any case).
    pub fn filter(&mut self, query: &str) {
        self.query = query.to_string();
        let q = query.trim().to_lowercase();
        self.results = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match self.scope {
                Scope::Pane => e.here,
                Scope::All => true,
                Scope::Failed => e.exit.is_some_and(|c| c != 0),
                // searched by the worker (`found`), not from these entries
                Scope::Results => false,
            })
            .filter(|(_, e)| q.is_empty() || e.command.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.len().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&Entry> {
        self.results.get(self.selected).map(|&i| &self.entries[i])
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
        }
    }

    /// ↓: the next row; `false` past the last one, where the pane closes.
    pub fn step_down(&mut self) -> bool {
        if self.selected + 1 >= self.len() {
            return false;
        }
        self.selected += 1;
        true
    }
}

/// Keep the first occurrence of each command, in order; it counts as run in
/// the active pane (`here`) when any occurrence was, and shows the latest
/// run (exit, time, directory), so Failed sees a newer failure ranked below
/// an older success.
pub fn dedupe(entries: Vec<Entry>) -> Vec<Entry> {
    let mut at = std::collections::HashMap::new();
    let mut out: Vec<Entry> = Vec::new();
    for e in entries {
        if e.command.trim().is_empty() {
            continue;
        }
        match at.get(&e.command) {
            Some(&i) => {
                let kept: &mut Entry = &mut out[i];
                kept.here |= e.here;
                if e.finished > kept.finished {
                    kept.exit = e.exit;
                    kept.finished = e.finished;
                    kept.cwd = e.cwd;
                }
            }
            None => {
                at.insert(e.command.clone(), out.len());
                out.push(e);
            }
        }
    }
    out
}

/// `Just now`, `4 min ago`, `2 h ago`, `yesterday`, `5 days ago`.
pub fn age(t: SystemTime, now: SystemTime) -> String {
    let s = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match s {
        s if s < 60 => "Just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s if s < 2 * 86_400 => "yesterday".into(),
        s => format!("{} days ago", s / 86_400),
    }
}

fn saved_age(finished: Option<u64>, now: SystemTime) -> String {
    finished
        .and_then(|t| SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::from_secs(t)))
        .map(|t| age(t, now))
        .unwrap_or_else(|| "date unknown".into())
}

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 28.0;
const FOOTER_H: f32 = 30.0;
const MAX_ROWS: usize = 8;
const PAD_X: f32 = 16.0;
/// Results scope: the filter line under the header, the rows shown and
/// the output preview under them.
const FILTER_H: f32 = 26.0;
const RESULT_ROWS: usize = 6;
const PREVIEW_HEAD_H: f32 = 24.0;
const PREVIEW_LINE_H: f32 = 16.0;
const PREVIEW_SHOWN: usize = 8;

/// `local · main · ~/api` for a found run: what is known of where it ran.
pub fn found_context(f: &rift_core::result_search::Found) -> String {
    let mut parts = vec![f.host.label()];
    if let Some(b) = &f.branch {
        parts.push(b.clone());
    }
    parts.push(f.cwd.clone().unwrap_or_else(|| "directory unknown".into()));
    parts.join(" · ")
}

/// The preview's heading: which output line matched, how much output
/// there is, where it was read from.
pub fn preview_heading(f: &rift_core::result_search::Found) -> String {
    use rift_core::result_search::{Output, Source};
    let mut s = match (f.output_state, f.preview.is_empty()) {
        (Output::Unavailable, _) => "Output no longer in scrollback".to_string(),
        (_, true) => "No output".to_string(),
        (Output::Partial, false) => "Output (only part of it is kept)".to_string(),
        (Output::Full, false) => "Output".to_string(),
    };
    if let Some(n) = f.hit_line {
        s.push_str(&format!(" · match on line {n}"));
    } else if f.in_command.is_some() {
        s.push_str(" · match in the command only");
    }
    match f.source {
        Source::Pane => {}
        Source::Restored => s.push_str(" · restored from a previous session"),
        Source::Saved => s.push_str(" · saved transcript"),
    }
    s
}

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

/// Height of the open pane: the terminal output moves out from under it.
pub fn panel_height(hp: &HistoryPane) -> f32 {
    if hp.scope == Scope::Results {
        return HEADER_H
            + FILTER_H
            + hp.found.len().clamp(1, RESULT_ROWS) as f32 * ROW_H
            + PREVIEW_HEAD_H
            + PREVIEW_SHOWN as f32 * PREVIEW_LINE_H
            + 8.0
            + FOOTER_H;
    }
    HEADER_H + hp.results.len().clamp(1, MAX_ROWS) as f32 * ROW_H + FOOTER_H
}

/// A row's command as drawn: on one line, secrets masked while
/// `mask_secrets` is on (the entry keeps the real one for Edit and Rerun).
fn shown_command(command: &str) -> String {
    masked_line(command, rift_core::secrets::active().as_deref())
}

pub(crate) fn masked_line(command: &str, masker: Option<&rift_core::secrets::Masker>) -> String {
    match masker {
        Some(m) => m.mask(command).replace('\n', " ⏎ "),
        None => command.replace('\n', " ⏎ "),
    }
}

/// Draw the pane docked to the editor `zone` across `pane` (above the
/// editor at the pane bottom, else below it).
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    hp: &HistoryPane,
    zone: [f32; 4],
    pane: [f32; 4],
    mouse: Option<(f32, f32)>,
    hits: &mut Vec<Hit>,
) {
    if !hp.open {
        return;
    }
    let results = hp.scope == Scope::Results;
    let shown = if results {
        hp.found.len().clamp(1, RESULT_ROWS)
    } else {
        hp.results.len().clamp(1, MAX_ROWS)
    };
    let panel_h = panel_height(hp);
    let (x, w) = (pane[0], pane[2]);
    let at_bottom = zone[1] + zone[3] >= pane[1] + pane[3] - 2.0;
    let y = if at_bottom {
        (zone[1] - panel_h).max(pane[1])
    } else {
        zone[1] + zone[3]
    };
    let hot = |r: [f32; 4]| {
        mouse.is_some_and(|(mx, my)| {
            mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
        })
    };

    layer.rect([x, y], [w, panel_h], theme.bg, 0.0);
    layer.hline(x, y, w, 1.0, lift(theme, 0.1));
    if !at_bottom {
        layer.hline(x, y + panel_h - 1.0, w, 1.0, lift(theme, 0.1));
    }
    layer.set_clip(Some(([x, y, w, panel_h], 0.0)));

    // header: icon, HISTORY, scope chips, count, ×
    let cy = y + HEADER_H / 2.0;
    icons::draw(
        layer,
        Icon::History,
        [x + PAD_X + 7.0, cy],
        14.0,
        theme.text_dim,
    );
    let mut tb = Buffer::new(&mut engine.font_system, Metrics::new(11.0, 14.0));
    tb.set_text(
        &mut engine.font_system,
        "HISTORY",
        &Attrs::new()
            .family(Family::Name(FONT_SANS))
            .weight(Weight::SEMIBOLD)
            .letter_spacing(0.06),
        Shaping::Advanced,
        None,
    );
    layer.text(tb, x + PAD_X + 22.0, cy - 7.0, theme.text_dim);
    let mut cx = x + PAD_X + 22.0 + 62.0;
    for scope in Scope::ALL {
        let label = scope.label();
        let lw = engine.measure(label, 11.5, FONT_SANS, Weight::MEDIUM);
        let r = [cx, cy - 11.0, lw + 18.0, 22.0];
        let on = hp.scope == scope;
        let (fill, edge) = if on {
            (theme.accent.with_alpha(0.16), theme.accent.with_alpha(0.5))
        } else if hot(r) {
            (lift(theme, 0.05), lift(theme, 0.12))
        } else {
            (Color::TRANSPARENT, lift(theme, 0.1))
        };
        layer.bordered([r[0], r[1]], [r[2], r[3]], fill, edge, 4.0, 1.0);
        layer.text(
            sans(engine, label, 11.5, Weight::MEDIUM),
            r[0] + 9.0,
            cy - 7.8,
            if on { theme.text_ui } else { theme.text_dim },
        );
        hits.push(Hit {
            rect: r,
            target: ClickTarget::HistoryScope(scope),
        });
        cx += r[2] + 4.0;
    }
    let close = [x + w - 8.0 - 24.0, cy - 12.0, 24.0, 24.0];
    if hot(close) {
        layer.rect([close[0], close[1]], [24.0, 24.0], lift(theme, 0.08), 4.0);
    }
    icons::draw(
        layer,
        Icon::Close,
        [close[0] + 12.0, cy],
        11.0,
        theme.text_dim,
    );
    hits.push(Hit {
        rect: close,
        target: ClickTarget::HistoryClose,
    });
    let count = if results {
        let n = hp.found.len();
        format!(
            "{n}{} run{}",
            if hp.truncated { "+" } else { "" },
            if n == 1 { "" } else { "s" }
        )
    } else {
        format!("{} of {}", hp.results.len(), hp.entries.len())
    };
    let cw = engine.measure(&count, 11.5, FONT_SANS, Weight::NORMAL);
    if close[0] - 10.0 - cw > cx + 8.0 {
        layer.text(
            sans(engine, &count, 11.5, Weight::NORMAL),
            close[0] - 10.0 - cw,
            cy - 7.8,
            theme.text_muted,
        );
    }
    layer.hline(x, y + HEADER_H, w, 1.0, theme.border);
    if results {
        render_results(
            layer,
            engine,
            theme,
            hp,
            [x, y + HEADER_H + 1.0, w],
            hot,
            hits,
        );
        footer(layer, engine, theme, x, y + panel_h - FOOTER_H, w, true);
        layer.set_clip(None);
        return;
    }

    // rows: status dot, command, directory, age
    let rows_top = y + HEADER_H + 1.0;
    let now = SystemTime::now();
    let age_w = 90.0;
    let cwd_w = (w * 0.22).clamp(80.0, 240.0);
    let scroll = hp.selected.saturating_sub(MAX_ROWS - 1);
    if hp.results.is_empty() {
        let msg = if hp.query.trim().is_empty() {
            "No commands yet"
        } else {
            "No matching commands"
        };
        layer.text(
            sans(engine, msg, 12.5, Weight::NORMAL),
            x + PAD_X,
            rows_top + (ROW_H - 16.9) / 2.0,
            theme.text_muted,
        );
    }
    let q = hp.query.trim().to_lowercase();
    for (row, &idx) in hp.results.iter().skip(scroll).take(shown).enumerate() {
        let e = &hp.entries[idx];
        let i = row + scroll;
        let ry = rows_top + row as f32 * ROW_H;
        let rect = [x, ry, w, ROW_H];
        let selected = i == hp.selected;
        if selected {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.07), 0.0);
            layer.rect([x, ry], [2.0, ROW_H], theme.accent, 0.0);
        } else if hot(rect) {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.04), 0.0);
        }
        hits.push(Hit {
            rect,
            target: ClickTarget::HistoryRow(i),
        });
        let rcy = ry + ROW_H / 2.0;
        let dot = match e.exit {
            Some(0) => theme.success,
            Some(_) => theme.error,
            None => theme.text_muted.with_alpha(0.6),
        };
        layer.circle([x + PAD_X + 4.0, rcy], 8.0, dot);

        // the command, the match in the accent
        let cmd_x = x + PAD_X + 20.0;
        let right = x + w - PAD_X - age_w - cwd_w - 16.0;
        let line = shown_command(&e.command);
        let lower = line.to_lowercase();
        // highlight only where lowercasing kept the byte offsets
        let (a, b) = match lower
            .find(&q)
            .filter(|_| !q.is_empty() && lower.len() == line.len())
        {
            Some(s) if line.is_char_boundary(s) && line.is_char_boundary(s + q.len()) => {
                (s, s + q.len())
            }
            _ => (0, 0),
        };
        let (pre, mid, post) = (&line[..a], &line[a..b], &line[b..]);
        let plain = Attrs::new()
            .family(Family::Name(mono_family()))
            .color(text_color(if selected {
                theme.text_bright
            } else {
                theme.text_ui
            }));
        let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(12.5, 16.0));
        buf.set_rich_text(
            &mut engine.font_system,
            vec![
                (pre, plain.clone()),
                (
                    mid,
                    Attrs::new()
                        .family(Family::Name(mono_family()))
                        .weight(Weight::BOLD)
                        .color(text_color(theme.accent)),
                ),
                (post, plain.clone()),
            ],
            &plain,
            Shaping::Advanced,
            None,
        );
        layer.text_clipped(
            buf,
            cmd_x,
            rcy - 8.0,
            theme.text_ui,
            TextBounds {
                left: cmd_x as i32,
                top: ry as i32,
                right: right.max(cmd_x) as i32,
                bottom: (ry + ROW_H) as i32,
            },
        );
        if let Some(cwd) = &e.cwd {
            let cwd = super::palette::elide_to(engine, cwd, 12.0, mono_family(), cwd_w);
            let mut cb = Buffer::new(&mut engine.font_system, Metrics::new(12.0, 16.0));
            cb.set_text(
                &mut engine.font_system,
                &cwd,
                &Attrs::new().family(Family::Name(mono_family())),
                Shaping::Advanced,
                None,
            );
            layer.text(cb, right + 16.0, rcy - 8.0, theme.text_muted);
        }
        let when = match (e.finished, e.cwd.is_none()) {
            (Some(t), _) => age(t, now),
            (None, true) => "imported".into(),
            (None, false) => String::new(),
        };
        if !when.is_empty() {
            let aw = engine.measure(&when, 12.0, FONT_SANS, Weight::NORMAL);
            layer.text(
                sans(engine, &when, 12.0, Weight::NORMAL),
                x + w - PAD_X - aw,
                rcy - 8.1,
                theme.text_muted,
            );
        }
    }

    footer(layer, engine, theme, x, y + panel_h - FOOTER_H, w, false);
    layer.set_clip(None);
}

fn footer(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    x: f32,
    fy: f32,
    w: f32,
    results: bool,
) {
    layer.hline(x, fy, w, 1.0, theme.border);
    key_hints(
        layer,
        engine,
        theme,
        &[
            (&["↑", "↓"], "navigate"),
            (&["⏎"], if results { "rerun here" } else { "run" }),
            (&["⇥"], "edit first"),
            (&["⌘", "⏎"], "run in new split"),
            (&["⌃", "R"], "scope"),
            (&["esc"], "close"),
        ],
        x + PAD_X,
        fy + FOOTER_H / 2.0,
        x + w - PAD_X,
    );
}

/// A mono line with `hl` (a byte range) in the accent.
fn mono_marked(
    engine: &mut TextEngine,
    theme: &Theme,
    line: &str,
    hl: Option<std::ops::Range<usize>>,
    size: f32,
    color: Color,
) -> Buffer {
    let (a, b) = match hl {
        Some(r)
            if r.end <= line.len()
                && line.is_char_boundary(r.start)
                && line.is_char_boundary(r.end) =>
        {
            (r.start, r.end)
        }
        _ => (0, 0),
    };
    let plain = Attrs::new()
        .family(Family::Name(mono_family()))
        .color(text_color(color));
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.28));
    buf.set_rich_text(
        &mut engine.font_system,
        vec![
            (&line[..a], plain.clone()),
            (
                &line[a..b],
                Attrs::new()
                    .family(Family::Name(mono_family()))
                    .weight(Weight::BOLD)
                    .color(text_color(theme.accent)),
            ),
            (&line[b..], plain.clone()),
        ],
        &plain,
        Shaping::Advanced,
        None,
    );
    buf
}

/// The Results scope under the header: the filter line, the found runs
/// (status, command, where it ran, when) and the selected run's output
/// with the match marked. Previewing runs nothing.
#[allow(clippy::too_many_arguments)]
fn render_results(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    hp: &HistoryPane,
    [x, top, w]: [f32; 3],
    hot: impl Fn([f32; 4]) -> bool,
    hits: &mut Vec<Hit>,
) {
    // filters in force, else how to set them; the search's state at right
    let fcy = top + FILTER_H / 2.0;
    let (_, filters) = rift_core::result_search::parse_query(&hp.query);
    let labels = filters.labels();
    let mut fx = x + PAD_X;
    if labels.is_empty() {
        layer.text(
            sans(
                engine,
                "Filter: is:failed  is:ok  since:today  since:7d  dir:…  host:…  branch:…",
                11.5,
                Weight::NORMAL,
            ),
            fx,
            fcy - 7.8,
            theme.text_muted,
        );
    }
    for label in &labels {
        let lw = engine.measure(label, 11.5, FONT_SANS, Weight::MEDIUM);
        layer.bordered(
            [fx, fcy - 10.0],
            [lw + 16.0, 20.0],
            theme.accent.with_alpha(0.12),
            theme.accent.with_alpha(0.4),
            4.0,
            1.0,
        );
        layer.text(
            sans(engine, label, 11.5, Weight::MEDIUM),
            fx + 8.0,
            fcy - 7.8,
            theme.text_ui,
        );
        fx += lw + 22.0;
    }
    let state = if hp.searching {
        "Searching…"
    } else if hp.truncated {
        "Stopped at the search limit: older runs not searched"
    } else if !hp.saved {
        "Open panes only (saved transcripts are off)"
    } else {
        "Open panes and saved transcripts"
    };
    let sw = engine.measure(state, 11.5, FONT_SANS, Weight::NORMAL);
    if x + w - PAD_X - sw > fx + 12.0 {
        layer.text(
            sans(engine, state, 11.5, Weight::NORMAL),
            x + w - PAD_X - sw,
            fcy - 7.8,
            theme.text_muted,
        );
    }
    layer.hline(x, top + FILTER_H, w, 1.0, lift(theme, 0.06));

    let rows_top = top + FILTER_H + 1.0;
    let shown = hp.found.len().clamp(1, RESULT_ROWS);
    let now = SystemTime::now();
    let age_w = 90.0;
    let ctx_w = (w * 0.34).clamp(110.0, 380.0);
    let scroll = hp.selected.saturating_sub(RESULT_ROWS - 1);
    if hp.found.is_empty() {
        let msg = if hp.searching {
            "Searching…"
        } else if hp.query.trim().is_empty() {
            "No finished commands yet"
        } else {
            "No matching runs"
        };
        layer.text(
            sans(engine, msg, 12.5, Weight::NORMAL),
            x + PAD_X,
            rows_top + (ROW_H - 16.9) / 2.0,
            theme.text_muted,
        );
    }
    for (row, f) in hp.found.iter().enumerate().skip(scroll).take(shown) {
        let i = row;
        let ry = rows_top + (row - scroll) as f32 * ROW_H;
        let rect = [x, ry, w, ROW_H];
        let selected = i == hp.selected;
        if selected {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.07), 0.0);
            layer.rect([x, ry], [2.0, ROW_H], theme.accent, 0.0);
        } else if hot(rect) {
            layer.rect([x, ry], [w, ROW_H], lift(theme, 0.04), 0.0);
        }
        hits.push(Hit {
            rect,
            target: ClickTarget::HistoryRow(i),
        });
        let rcy = ry + ROW_H / 2.0;
        let dot = match f.exit {
            Some(0) => theme.success,
            Some(_) => theme.error,
            None => theme.text_muted.with_alpha(0.6),
        };
        layer.circle([x + PAD_X + 4.0, rcy], 8.0, dot);
        let cmd_x = x + PAD_X + 20.0;
        let right = x + w - PAD_X - age_w - ctx_w - 16.0;
        let line = shown_command(&f.command);
        let hl = f
            .in_command
            .clone()
            .filter(|_| line.len() == f.command.len());
        let color = if selected {
            theme.text_bright
        } else {
            theme.text_ui
        };
        let buf = mono_marked(engine, theme, &line, hl, 12.5, color);
        layer.text_clipped(
            buf,
            cmd_x,
            rcy - 8.0,
            theme.text_ui,
            TextBounds {
                left: cmd_x as i32,
                top: ry as i32,
                right: right.max(cmd_x) as i32,
                bottom: (ry + ROW_H) as i32,
            },
        );
        let ctx = super::palette::elide_to(engine, &found_context(f), 12.0, mono_family(), ctx_w);
        let mut cb = Buffer::new(&mut engine.font_system, Metrics::new(12.0, 16.0));
        cb.set_text(
            &mut engine.font_system,
            &ctx,
            &Attrs::new().family(Family::Name(mono_family())),
            Shaping::Advanced,
            None,
        );
        layer.text(cb, right + 16.0, rcy - 8.0, theme.text_muted);
        let when = saved_age(f.finished, now);
        let aw = engine.measure(&when, 12.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            sans(engine, &when, 12.0, Weight::NORMAL),
            x + w - PAD_X - aw,
            rcy - 8.1,
            theme.text_muted,
        );
    }

    // the selected run's output, the match marked
    let py = rows_top + shown as f32 * ROW_H;
    layer.hline(x, py, w, 1.0, theme.border);
    layer.rect(
        [x, py + 1.0],
        [
            w,
            PREVIEW_HEAD_H + PREVIEW_SHOWN as f32 * PREVIEW_LINE_H + 8.0 - 1.0,
        ],
        lift(theme, 0.025),
        0.0,
    );
    let Some(f) = hp.current_found() else {
        return;
    };
    layer.text(
        sans(engine, &preview_heading(f), 11.5, Weight::MEDIUM),
        x + PAD_X,
        py + PREVIEW_HEAD_H / 2.0 - 7.8,
        theme.text_dim,
    );
    let lines_top = py + PREVIEW_HEAD_H;
    // keep the match in view
    let first = match f.hit {
        Some((i, _)) if i >= PREVIEW_SHOWN => i + 1 - PREVIEW_SHOWN,
        _ => 0,
    };
    for (n, line) in f.preview.iter().enumerate().skip(first).take(PREVIEW_SHOWN) {
        let ly = lines_top + (n - first) as f32 * PREVIEW_LINE_H;
        let hl = f
            .hit
            .as_ref()
            .filter(|(i, _)| *i == n)
            .map(|(_, r)| r.clone());
        if hl.is_some() {
            layer.rect(
                [x, ly],
                [w, PREVIEW_LINE_H],
                theme.accent.with_alpha(0.1),
                0.0,
            );
        }
        let buf = mono_marked(engine, theme, line, hl, 12.0, theme.text_ui);
        layer.text_clipped(
            buf,
            x + PAD_X,
            ly,
            theme.text_ui,
            TextBounds {
                left: (x + PAD_X) as i32,
                top: ly as i32,
                right: (x + w - PAD_X) as i32,
                bottom: (ly + PREVIEW_LINE_H) as i32,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn e(cmd: &str, exit: Option<i32>, here: bool) -> Entry {
        Entry {
            command: cmd.into(),
            cwd: None,
            exit,
            finished: None,
            here,
        }
    }

    #[test]
    fn a_row_draws_its_command_masked() {
        let token = format!("ghp_{}", "FAKE0123".repeat(5)); // fake: never issued
        let cmd = format!("export GH={token}\necho ok");
        let masker = rift_core::secrets::Masker::new(&[]).unwrap();
        let shown = masked_line(&cmd, Some(&masker));
        assert!(!shown.contains(&token), "{shown}");
        assert!(
            shown.starts_with("export GH=") && shown.ends_with(" ⏎ echo ok"),
            "{shown}"
        );
        // masking off: the command as typed
        assert_eq!(
            masked_line(&cmd, None),
            format!("export GH={token} ⏎ echo ok")
        );
    }

    #[test]
    fn down_past_the_last_row_asks_to_close() {
        let mut hp = HistoryPane::default();
        hp.open(vec![e("new", None, true), e("old", None, true)], "");
        assert!(hp.step_down());
        assert_eq!(hp.current_command(), Some("old"));
        assert!(!hp.step_down());
        assert_eq!(hp.current_command(), Some("old"));
        // nothing matches: ↓ closes at once
        hp.filter("zzz");
        assert!(!hp.step_down());
    }

    #[test]
    fn a_kept_row_shows_its_latest_run() {
        let at = |s| Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s));
        // ranked first: passed here yesterday; later: failed elsewhere a
        // minute ago
        let rows = dedupe(vec![
            Entry {
                finished: at(1_000),
                ..e("cargo test", Some(0), true)
            },
            Entry {
                finished: at(90_000),
                cwd: Some("~/other".into()),
                ..e("cargo test", Some(101), false)
            },
        ]);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].exit, rows[0].finished, rows[0].here),
            (Some(101), at(90_000), true)
        );
        assert_eq!(rows[0].cwd.as_deref(), Some("~/other"));
        let mut hp = HistoryPane::default();
        hp.open(rows, "");
        hp.set_scope(Scope::Failed);
        assert_eq!(hp.results, vec![0]);
    }

    #[test]
    fn scope_and_query_narrow_the_rows() {
        let mut hp = HistoryPane::default();
        hp.open(
            dedupe(vec![
                e("cargo test", Some(0), true),
                e("cargo build", Some(101), false),
                e("cargo test", Some(1), false),
                e("ls", None, false),
            ]),
            "cargo",
        );
        // the newer duplicate wins
        assert_eq!(hp.entries.len(), 3);
        assert_eq!(hp.results, vec![0, 1]);
        hp.set_scope(Scope::Failed);
        assert_eq!(hp.results, vec![1]);
        hp.set_scope(Scope::Pane);
        assert_eq!(hp.results, vec![0]);
        hp.set_scope(Scope::All);
        hp.filter("");
        assert_eq!(hp.results.len(), 3);
        hp.move_by(5);
        assert_eq!(hp.selected, 2);
        hp.filter("zzz");
        assert!(hp.current().is_none());
        assert_eq!(Scope::Failed.next(), Scope::Results);
        assert_eq!(Scope::Results.next(), Scope::Pane);
    }

    #[test]
    fn a_stale_search_never_replaces_the_current_one() {
        use rift_core::result_search::{Found, Host, Outcome, Output, RunId, Source};
        let run = |cmd: &str| Found {
            id: RunId::Live {
                pane: "p.1".into(),
                line: 0,
            },
            command: cmd.into(),
            exit: None,
            finished: None,
            cwd: None,
            host: Host::Unknown,
            branch: None,
            source: Source::Pane,
            output_state: Output::Full,
            in_command: None,
            preview: Vec::new(),
            hit: None,
            hit_line: None,
        };
        let mut hp = HistoryPane::default();
        hp.open(Vec::new(), "mig");
        hp.set_scope(Scope::Results);
        assert!(hp.wants_search());
        // the window asked twice (typing): only the second answer counts
        hp.results_seq = 2;
        hp.results_query = Some("mig".into());
        hp.searching = true;
        assert!(!hp.wants_search());
        let old = Outcome {
            found: vec![run("old")],
            ..Default::default()
        };
        assert!(!hp.take_results(1, old));
        assert!(hp.found.is_empty() && hp.searching);
        let new = Outcome {
            found: vec![run("make migrate")],
            ..Default::default()
        };
        assert!(hp.take_results(2, new));
        assert_eq!(hp.current_command(), Some("make migrate"));
        assert!(!hp.searching);
        // closed: a late answer finds nothing to fill
        hp.close();
        assert!(!hp.take_results(2, Outcome::default()));
        assert_eq!(hp.results_seq, 2);
    }

    #[test]
    fn ages_read_like_a_person_would_say_them() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10 * 86_400);
        let ago = |s| now - Duration::from_secs(s);
        assert_eq!(age(ago(5), now), "Just now");
        assert_eq!(age(ago(240), now), "4 min ago");
        assert_eq!(age(ago(7200), now), "2 h ago");
        assert_eq!(age(ago(90_000), now), "yesterday");
        assert_eq!(age(ago(5 * 86_400), now), "5 days ago");
        assert_eq!(saved_age(Some(u64::MAX), now), "date unknown");
        assert_eq!(saved_age(None, now), "date unknown");
        assert_eq!(saved_age(Some(10 * 86_400 - 240), now), "4 min ago");
    }
}
