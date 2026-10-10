//! Command history pane (⌃R): docked above the command line across the
//! pane, like the completion panel. The command line is the search field;
//! rows show whether the command succeeded, the command with the match
//! highlighted, where it ran and how long ago.

use std::time::SystemTime;

use shift_core::{Color, Theme};
use shift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, TextBounds, Weight};
use shift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

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
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Pane, Scope::All, Scope::Failed];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Pane => "This pane",
            Scope::All => "All shells",
            Scope::Failed => "Failed",
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
            ..HistoryPane::default()
        };
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
            })
            .filter(|(_, e)| q.is_empty() || e.command.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.results.len().saturating_sub(1));
    }

    pub fn current(&self) -> Option<&Entry> {
        self.results.get(self.selected).map(|&i| &self.entries[i])
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.results.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + delta).clamp(0, n - 1) as usize;
        }
    }
}

/// Newest first, keeping the first (newest) occurrence of each command.
pub fn dedupe(entries: Vec<Entry>) -> Vec<Entry> {
    let mut seen = std::collections::HashSet::new();
    entries
        .into_iter()
        .filter(|e| !e.command.trim().is_empty() && seen.insert(e.command.clone()))
        .collect()
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

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 28.0;
const FOOTER_H: f32 = 30.0;
const MAX_ROWS: usize = 8;
const PAD_X: f32 = 16.0;

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
    let shown = hp.results.len().clamp(1, MAX_ROWS);
    let panel_h = HEADER_H + shown as f32 * ROW_H + FOOTER_H;
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

    layer.rect([x, y], [w, panel_h], theme.ground(theme.bg), 0.0);
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
    let count = format!("{} of {}", hp.results.len(), hp.entries.len());
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
        let line = e.command.replace('\n', " ⏎ ");
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

    let fy = y + panel_h - FOOTER_H;
    layer.hline(x, fy, w, 1.0, theme.border);
    key_hints(
        layer,
        engine,
        theme,
        &[
            (&["↑", "↓"], "navigate"),
            (&["⏎"], "run"),
            (&["⇥"], "edit first"),
            (&["⌘", "⏎"], "run in new split"),
            (&["⌃", "R"], "scope"),
            (&["esc"], "close"),
        ],
        x + PAD_X,
        fy + FOOTER_H / 2.0,
        x + w - PAD_X,
    );
    layer.set_clip(None);
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
        assert_eq!(Scope::Failed.next(), Scope::Pane);
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
    }
}
