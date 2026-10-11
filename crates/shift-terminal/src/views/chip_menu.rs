//! The prompt context chips' menus (Warp): the cwd chip opens a searchable
//! list of the parent and the sub-directories, the branch chip one of the
//! repository's local branches, the PHP and Python chips the installed
//! versions, the Docker chip the compose project's containers. Picking a row
//! runs `cd` / `git checkout` / `shift-use` / `docker compose logs` as an
//! ordinary command in the pane, so the shell's own state follows and the
//! block shows what happened.

use std::path::{Path, PathBuf};

use shift_core::docker::{ComposeInfo, Container, Health};
use shift_core::runtimes::Runtime;

use shift_core::{Color, Theme};
use shift_pty::PaneId;
use shift_renderer::glyphon::Weight;
use shift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::glyphs;
use crate::views::icons::{self, Icon};
use crate::views::palette::{lift, sans};

pub const MENU_W: f32 = 360.0;
/// The container list is a table: wider, with a column header.
const DOCKER_W: f32 = 520.0;
const HEADER_H: f32 = 22.0;
const SEARCH_H: f32 = 34.0;
const ROW_H: f32 = 26.0;
const MAX_ROWS: usize = 8;
/// Directory entries read at most (a huge folder still opens at once).
const MAX_DIRS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipKind {
    Dir,
    Branch,
    Php,
    Python,
    Docker,
}

impl ChipKind {
    /// The menu a prompt chip opens, if any.
    pub fn of(chip: shift_core::PromptChip) -> Option<Self> {
        use shift_core::PromptChip as C;
        Some(match chip {
            C::Cwd => ChipKind::Dir,
            C::GitBranch => ChipKind::Branch,
            C::Php => ChipKind::Php,
            C::Python => ChipKind::Python,
            C::Docker => ChipKind::Docker,
            _ => return None,
        })
    }

    pub fn chip(self) -> shift_core::PromptChip {
        use shift_core::PromptChip as C;
        match self {
            ChipKind::Dir => C::Cwd,
            ChipKind::Branch => C::GitBranch,
            ChipKind::Php => C::Php,
            ChipKind::Python => C::Python,
            ChipKind::Docker => C::Docker,
        }
    }

    /// The menu's accessible name.
    pub fn title(self) -> &'static str {
        match self {
            ChipKind::Dir => "Change directory",
            ChipKind::Branch => "Change git branch",
            ChipKind::Php => "Change PHP version",
            ChipKind::Python => "Change Python version",
            ChipKind::Docker => "Docker containers",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChipAction {
    /// `cd` into this directory name (`..` = the parent).
    Cd(String),
    Checkout(String),
    /// `git checkout -b`: a branch named by the query.
    Create(String),
    /// Switch this shell to a runtime: the whole `shift-use` command.
    Use(String),
    /// Follow a compose service's logs (in the project's directory).
    Logs(PathBuf, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// Muted text after the label: a version's source.
    pub detail: String,
    /// A Docker row's container: drawn as columns (ports, uptime, health).
    pub container: Option<Container>,
    pub action: ChipAction,
    /// The checked-out branch, the active version, a running container.
    pub current: bool,
}

pub struct ChipMenu {
    pub kind: ChipKind,
    pub pane: PaneId,
    /// The chip it drops from (window coordinates).
    pub anchor: [f32; 4],
    pub query: String,
    items: Vec<Item>,
    pub selected: usize,
    /// How far the list is scrolled, in pixels (trackpads scroll smoothly,
    /// not row by row).
    pub scroll: f32,
    /// What an empty list says (no query): no versions, Docker down…
    empty: String,
}

impl ChipMenu {
    /// The parent, then `cwd`'s sub-directories by name (hidden ones only
    /// for a query starting with `.`).
    pub fn dirs(pane: PaneId, anchor: [f32; 4], cwd: &Path) -> Self {
        let mut names: Vec<String> = std::fs::read_dir(cwd)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .take(MAX_DIRS)
                    // symlinks to folders count (`metadata` follows them)
                    .filter(|e| e.path().metadata().is_ok_and(|m| m.is_dir()))
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by_key(|n| n.to_lowercase());
        let mut items = Vec::new();
        if cwd.parent().is_some() {
            items.push(Item {
                label: ".. (Parent Directory)".into(),
                detail: String::new(),
                container: None,
                action: ChipAction::Cd("..".into()),
                current: false,
            });
        }
        items.extend(names.into_iter().map(|n| Item {
            label: n.clone(),
            detail: String::new(),
            container: None,
            action: ChipAction::Cd(n),
            current: false,
        }));
        Self::new(ChipKind::Dir, pane, anchor, items)
    }

    /// Local branches, the checked-out one first.
    pub fn branches(pane: PaneId, anchor: [f32; 4], names: Vec<String>, current: &str) -> Self {
        let items = names
            .into_iter()
            .map(|n| Item {
                current: n == current,
                label: n.clone(),
                detail: String::new(),
                container: None,
                action: ChipAction::Checkout(n),
            })
            .collect();
        Self::new(ChipKind::Branch, pane, anchor, items)
    }

    /// Installed PHP or Python versions, newest first; `active` is the
    /// one the shell runs now.
    pub fn runtimes(
        kind: ChipKind,
        pane: PaneId,
        anchor: [f32; 4],
        list: Vec<Runtime>,
        active: Option<&str>,
    ) -> Self {
        let tool = if kind == ChipKind::Php {
            "php"
        } else {
            "python"
        };
        let items = list
            .into_iter()
            .map(|r| Item {
                current: active == Some(r.version.as_str()),
                action: ChipAction::Use(r.command(tool)),
                detail: r.source,
                container: None,
                label: r.version,
            })
            .collect();
        let mut menu = Self::new(kind, pane, anchor, items);
        menu.empty = match kind {
            ChipKind::Php => "No PHP installs found (Homebrew, phpenv, asdf, mise)",
            _ => "No Python installs found (pyenv, Homebrew, asdf, mise)",
        }
        .into();
        // the active version starts selected
        if let Some(i) = menu.items.iter().position(|it| it.current) {
            menu.move_by(i as isize);
        }
        menu
    }

    /// A compose project's containers, running ones first.
    pub fn containers(pane: PaneId, anchor: [f32; 4], info: Option<&ComposeInfo>) -> Self {
        let mut menu = Self::new(ChipKind::Docker, pane, anchor, Vec::new());
        menu.set_containers(info);
        menu
    }

    /// Replace the container rows (a fresher `docker compose ps`).
    pub fn set_containers(&mut self, info: Option<&ComposeInfo>) {
        self.items = info
            .map(|d| {
                d.containers
                    .iter()
                    .map(|c| Item {
                        label: c.service.clone(),
                        detail: String::new(),
                        container: Some(c.clone()),
                        action: ChipAction::Logs(d.dir.clone(), c.service.clone()),
                        current: c.running(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.empty = match info {
            None => "No compose project here".into(),
            Some(d) => d
                .error
                .clone()
                .unwrap_or_else(|| "No containers: docker compose up".into()),
        };
        let n = self.count();
        self.selected = self.selected.min(n.saturating_sub(1));
        self.scroll_px(0.0);
    }

    fn new(kind: ChipKind, pane: PaneId, anchor: [f32; 4], items: Vec<Item>) -> Self {
        ChipMenu {
            kind,
            pane,
            anchor,
            query: String::new(),
            items,
            selected: 0,
            scroll: 0.0,
            empty: "No matches".into(),
        }
    }

    /// What the list says when nothing shows.
    pub fn empty_text(&self) -> &str {
        if self.query.is_empty() {
            &self.empty
        } else {
            "No matches"
        }
    }

    /// The rows the query keeps (case-insensitive substring). Branches end
    /// with "Create branch" for a query no branch has.
    pub fn results(&self) -> Vec<Item> {
        let mut out: Vec<Item> = self.kept().cloned().collect();
        if self.offers_create() {
            let name = self.query.trim();
            out.push(Item {
                label: format!("Create branch {name}"),
                detail: String::new(),
                container: None,
                action: ChipAction::Create(name.to_string()),
                current: false,
            });
        }
        out
    }

    /// The items the query keeps, uncloned.
    fn kept(&self) -> impl Iterator<Item = &Item> {
        let q = self.query.to_lowercase();
        let hidden_ok = q.starts_with('.');
        self.items.iter().filter(move |it| match &it.action {
            ChipAction::Cd(n) if n == ".." => q.is_empty(),
            ChipAction::Cd(n) => {
                (hidden_ok || !n.starts_with('.')) && n.to_lowercase().contains(&q)
            }
            _ => it.label.to_lowercase().contains(&q),
        })
    }

    /// Branches offer "Create branch" for a query no branch has.
    fn offers_create(&self) -> bool {
        let name = self.query.trim();
        self.kind == ChipKind::Branch
            && !name.is_empty()
            && !name.contains(char::is_whitespace)
            && !self.items.iter().any(|it| it.label == name)
    }

    /// How many rows the query keeps (no copies: wheel events come fast).
    pub fn count(&self) -> usize {
        self.kept().count() + usize::from(self.offers_create())
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected = 0;
        self.scroll = 0.0;
    }

    /// Move the selection by `d` rows, scrolling it fully into view.
    pub fn move_by(&mut self, d: isize) {
        let n = self.count();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + d).clamp(0, n as isize - 1) as usize;
        let top = self.selected as f32 * ROW_H;
        if top < self.scroll {
            self.scroll = top;
        } else if top + ROW_H > self.scroll + MAX_ROWS as f32 * ROW_H {
            self.scroll = top + ROW_H - MAX_ROWS as f32 * ROW_H;
        }
    }

    /// Scroll the list by `dy` pixels (wheel, trackpad), the selection
    /// staying put; clamped to the list.
    pub fn scroll_px(&mut self, dy: f32) {
        let max = self.count().saturating_sub(MAX_ROWS) as f32 * ROW_H;
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    /// One wheel notch: a row.
    pub const NOTCH: f32 = ROW_H;
}

/// A container row's ports column: its published ports, else a dash.
fn ports_cell(item: &Item) -> String {
    match &item.container {
        Some(c) if !c.ports.is_empty() => c.ports.join(", "),
        _ => "—".into(),
    }
}

/// The shell command a picked row runs.
pub fn command(action: &ChipAction) -> String {
    let q = crate::input::shell_quote;
    match action {
        ChipAction::Cd(n) if n == ".." => "cd ..".into(),
        // `./` keeps a name starting with `-` from reading as an option
        ChipAction::Cd(n) => format!("cd {}", q(&format!("./{n}"))),
        ChipAction::Checkout(b) => format!("git checkout {}", q(b)),
        ChipAction::Create(b) => format!("git checkout -b {}", q(b)),
        ChipAction::Use(cmd) => cmd.clone(),
        ChipAction::Logs(dir, service) => format!(
            "docker compose --project-directory {} logs --follow --tail 200 {}",
            q(&dir.to_string_lossy()),
            q(service)
        ),
    }
}

/// Draw the menu over or under its chip in a window `[w, h]`, `top_min`
/// below the tab bar; returns its card and the rows' rects with their
/// result index.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    menu: &ChipMenu,
    [w, h]: [f32; 2],
    top_min: f32,
    mouse: (f32, f32),
    blink_on: bool,
) -> ([f32; 4], Vec<([f32; 4], usize)>) {
    let results = menu.results();
    let shown = results.len().clamp(1, MAX_ROWS);
    let table = menu.kind == ChipKind::Docker && !results.is_empty();
    let menu_w = if menu.kind == ChipKind::Docker {
        DOCKER_W.min(w - 16.0)
    } else {
        MENU_W
    };
    let header_h = if table { HEADER_H } else { 0.0 };
    let card_h = SEARCH_H + header_h + shown as f32 * ROW_H + 8.0;
    let a = menu.anchor;
    let mx = a[0].clamp(8.0, (w - menu_w - 8.0).max(8.0));
    // above the chip (the command line sits at the bottom), else under it
    let above = a[1] - 6.0 - card_h;
    let my = if above >= top_min {
        above
    } else {
        (a[1] + a[3] + 6.0).min(h - card_h - 8.0)
    };
    let card = [mx, my, menu_w, card_h];
    layer.shadow(
        [mx, my + 8.0],
        [menu_w, card_h],
        Color::BLACK.with_alpha(0.45),
        8.0,
        18.0,
    );
    layer.bordered(
        [mx, my],
        [menu_w, card_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );
    layer.set_clip(Some((
        [mx + 1.0, my + 1.0, menu_w - 2.0, card_h - 2.0],
        7.0,
    )));

    // ── search ──
    let cy = my + SEARCH_H / 2.0;
    let qx = mx + 14.0;
    if menu.query.is_empty() {
        let placeholder = match menu.kind {
            ChipKind::Dir => "Search directories…",
            ChipKind::Branch => "Search branches…",
            ChipKind::Php | ChipKind::Python => "Search versions…",
            ChipKind::Docker => "Search containers…",
        };
        layer.text(
            sans(engine, placeholder, 13.0, Weight::NORMAL),
            qx + 4.0,
            cy - 9.0,
            theme.text_muted,
        );
    } else {
        layer.text(
            sans(engine, &menu.query, 13.0, Weight::NORMAL),
            qx,
            cy - 9.0,
            theme.text_bright,
        );
    }
    if blink_on {
        let qw = engine.measure(&menu.query, 13.0, FONT_SANS, Weight::NORMAL);
        layer.rect([qx + qw + 1.0, cy - 8.0], [2.0, 16.0], theme.accent, 0.0);
    }
    layer.hline(mx + 1.0, my + SEARCH_H, menu_w - 2.0, 1.0, theme.border);

    // ── rows ──
    let mut rows = Vec::new();
    let y0 = my + SEARCH_H + 4.0;
    if results.is_empty() {
        let room = menu_w - 28.0;
        let empty = super::palette::elide_to(engine, menu.empty_text(), 12.5, FONT_SANS, room);
        layer.text(
            sans(engine, &empty, 12.5, Weight::NORMAL),
            mx + 14.0,
            y0 + (ROW_H - 17.0) / 2.0,
            theme.text_muted,
        );
    }
    let inside = |r: &[f32; 4]| {
        mouse.0 >= r[0] && mouse.0 < r[0] + r[2] && mouse.1 >= r[1] && mouse.1 < r[1] + r[3]
    };
    // the container table: each column as wide as its widest cell (capped),
    // so names, ports and uptimes line up
    let cols = table.then(|| {
        let widest = |engine: &mut TextEngine, cells: Vec<String>, size: f32, cap: f32| {
            cells
                .iter()
                .map(|t| engine.measure(t, size, FONT_SANS, Weight::NORMAL))
                .fold(0.0f32, f32::max)
                .min(cap)
        };
        let names = results.iter().map(|it| it.label.clone()).collect();
        let ports = results.iter().map(ports_cell).collect();
        let x0 = mx + 4.0 + 32.0;
        let name_w = widest(engine, names, 13.0, 150.0).max(64.0);
        let ports_w = widest(engine, ports, 12.0, 180.0).max(40.0);
        let x1 = x0 + name_w + 18.0;
        let x2 = x1 + ports_w + 18.0;
        for (x, title) in [(x0, "Container"), (x1, "Ports"), (x2, "Uptime")] {
            layer.text(
                sans(engine, title, 11.5, Weight::MEDIUM),
                x,
                y0 + 3.0,
                theme.text_muted,
            );
        }
        (x0, name_w, x1, ports_w, x2)
    });
    let y0 = y0 + header_h;
    // rows slide by pixels under a clip: a part-scrolled row shows its
    // visible part, and only that part takes clicks
    let list = [mx + 1.0, y0, menu_w - 2.0, shown as f32 * ROW_H];
    layer.set_clip(Some((list, 0.0)));
    let first = (menu.scroll / ROW_H) as usize;
    for (vi, item) in results.iter().enumerate().skip(first).take(MAX_ROWS + 1) {
        let ry = y0 + vi as f32 * ROW_H - menu.scroll;
        let r = [mx + 4.0, ry, menu_w - 8.0, ROW_H];
        let top = ry.max(list[1]);
        let bottom = (ry + ROW_H).min(list[1] + list[3]);
        if bottom <= top {
            continue;
        }
        let hit = [r[0], top, r[2], bottom - top];
        let selected = vi == menu.selected;
        if selected {
            layer.rect(
                [r[0], r[1]],
                [r[2], r[3]],
                theme.accent.with_alpha(0.16),
                4.0,
            );
        } else if inside(&hit) {
            layer.rect([r[0], r[1]], [r[2], r[3]], lift(theme, 0.05), 4.0);
        }
        // the chip's own Nerd Font mark; ↑ and + are UI icons
        let mark = match &item.action {
            ChipAction::Cd(n) if n == ".." => Err(Icon::ChevronUp),
            ChipAction::Cd(_) => Ok(glyphs::FOLDER),
            ChipAction::Checkout(_) => Ok(glyphs::BRANCH),
            ChipAction::Create(_) => Err(Icon::Plus),
            ChipAction::Use(_) if menu.kind == ChipKind::Php => Ok(glyphs::PHP),
            ChipAction::Use(_) => Ok(glyphs::PYTHON),
            ChipAction::Logs(..) => Ok(glyphs::DOCKER),
        };
        let rcy = ry + ROW_H / 2.0;
        let mark_color = if selected {
            theme.text_ui
        } else {
            theme.text_dim
        };
        match mark {
            Ok(g) => glyphs::draw(layer, engine, g, [r[0] + 16.0, rcy], 13.0, mark_color),
            Err(icon) => icons::draw(layer, icon, [r[0] + 16.0, rcy], 13.0, mark_color),
        }
        let parent = matches!(&item.action, ChipAction::Cd(n) if n == "..");
        let color = if parent {
            theme.text_dim
        } else if selected {
            theme.text_bright
        } else {
            theme.text_ui
        };
        if let (Some((x0, name_w, x1, ports_w, x2)), Some(c)) = (cols, &item.container) {
            let elide = |engine: &mut TextEngine, t: &str, size: f32, room: f32| {
                super::palette::elide_to(engine, t, size, FONT_SANS, room)
            };
            let up = c.running();
            let name = elide(engine, &item.label, 13.0, name_w);
            layer.text(
                sans(engine, &name, 13.0, Weight::NORMAL),
                x0,
                rcy - 9.0,
                if up { color } else { theme.text_dim },
            );
            let ports = ports_cell(item);
            let ports = elide(engine, &ports, 12.0, ports_w);
            layer.text(
                sans(engine, &ports, 12.0, Weight::NORMAL),
                x1,
                rcy - 8.0,
                if c.ports.is_empty() {
                    theme.text_muted
                } else {
                    theme.text_dim
                },
            );
            // uptime, then the health check's verdict in its colour
            let right = r[0] + r[2] - 10.0;
            let health = c.health.map(|h| {
                let tone = match h {
                    Health::Healthy => theme.success,
                    Health::Unhealthy => theme.error,
                    Health::Starting => theme.warning,
                };
                (h.label(), tone)
            });
            let health_w = health.map_or(0.0, |(t, _)| {
                engine.measure(t, 12.0, FONT_SANS, Weight::NORMAL) + 22.0
            });
            let uptime = elide(engine, &c.uptime(), 12.0, (right - x2 - health_w).max(24.0));
            layer.text(
                sans(engine, &uptime, 12.0, Weight::NORMAL),
                x2,
                rcy - 8.0,
                if up { theme.text_ui } else { theme.text_muted },
            );
            if let Some((t, tone)) = health {
                let hx = x2 + engine.measure(&uptime, 12.0, FONT_SANS, Weight::NORMAL) + 10.0;
                layer.circle([hx + 3.0, rcy], 6.0, tone);
                layer.text(
                    sans(engine, t, 12.0, Weight::NORMAL),
                    hx + 10.0,
                    rcy - 8.0,
                    tone,
                );
            }
            rows.push((hit, vi));
            continue;
        }
        let room = r[2] - 40.0 - if item.current { 24.0 } else { 0.0 };
        let label = super::palette::elide_to(engine, &item.label, 13.0, FONT_SANS, room);
        layer.text(
            sans(engine, &label, 13.0, Weight::NORMAL),
            r[0] + 32.0,
            rcy - 9.0,
            color,
        );
        // the detail takes what the label leaves, in a smaller muted face
        let label_w = engine.measure(&label, 13.0, FONT_SANS, Weight::NORMAL);
        let detail_room = room - label_w - 10.0;
        if !item.detail.is_empty() && detail_room > 24.0 {
            let detail =
                super::palette::elide_to(engine, &item.detail, 12.0, FONT_SANS, detail_room);
            layer.text(
                sans(engine, &detail, 12.0, Weight::NORMAL),
                r[0] + 32.0 + label_w + 10.0,
                rcy - 8.0,
                theme.text_muted,
            );
        }
        if item.current {
            icons::draw(
                layer,
                Icon::Check,
                [r[0] + r[2] - 14.0, rcy],
                13.0,
                theme.success,
            );
        }
        rows.push((hit, vi));
    }
    layer.set_clip(None);
    (card, rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_list_the_parent_and_visible_subdirectories() {
        let dir = std::env::temp_dir().join(format!("shift-chip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["src", "Docs", ".git", "-odd"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        let mut m = ChipMenu::dirs(1, [0.0; 4], &dir);
        let labels =
            |m: &ChipMenu| -> Vec<String> { m.results().into_iter().map(|i| i.label).collect() };
        assert_eq!(labels(&m), [".. (Parent Directory)", "-odd", "Docs", "src"]);
        m.set_query("d".into());
        assert_eq!(labels(&m), ["-odd", "Docs"]);
        m.set_query(".g".into());
        assert_eq!(labels(&m), [".git"]);
        assert_eq!(command(&m.results()[0].action), "cd ./.git");
        assert_eq!(command(&ChipAction::Cd("my dir".into())), "cd './my dir'");
        assert_eq!(command(&ChipAction::Cd("..".into())), "cd ..");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn branches_offer_to_create_a_missing_one() {
        let names = vec!["main".to_string(), "feature/x".to_string()];
        let mut m = ChipMenu::branches(1, [0.0; 4], names, "main");
        assert!(m.results()[0].current);
        m.set_query("feat".into());
        let r = m.results();
        assert_eq!(r[0].label, "feature/x");
        assert_eq!(r[1].action, ChipAction::Create("feat".into()));
        assert_eq!(command(&r[1].action), "git checkout -b feat");
        // an exact name, or one with a space, offers no creation
        m.set_query("main".into());
        assert_eq!(m.results().len(), 1);
        m.set_query("a b".into());
        assert!(m.results().is_empty());
        // the selection moves within the results and scrolls with them
        let many = (0..20).map(|i| format!("b{i:02}")).collect();
        let mut m = ChipMenu::branches(1, [0.0; 4], many, "b00");
        m.move_by(9);
        assert_eq!((m.selected, m.scroll), (9, 2.0 * ROW_H));
        m.move_by(-100);
        assert_eq!((m.selected, m.scroll), (0, 0.0));
        // a trackpad's small deltas add up; the end of the list stops them
        m.scroll_px(3.0);
        m.scroll_px(3.0);
        assert_eq!(m.scroll, 6.0);
        m.scroll_px(1e6);
        assert_eq!(m.scroll, (20 - MAX_ROWS) as f32 * ROW_H);
        m.scroll_px(-1e6);
        assert_eq!(m.scroll, 0.0);
    }
}
