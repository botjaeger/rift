use rift_core::{BlockId, Color, Theme};
use rift_pty::PaneId;
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine};

use crate::views::icons::{self, Icon};

/// Context-menu actions: block actions (right-click a block / ⌃M), tab
/// actions (right-click a tab) and terminal actions (right-click elsewhere).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAction {
    CopyCommand,
    CopyOutput,
    CopyBoth,
    CopyCwd,
    CopyBranch,
    Rerun,
    ToggleBookmark,
    ToggleCollapse,
    ScrollTopOfBlock,
    ScrollBottomOfBlock,
    FilterBlock,
    /// Send the block's output to a new command's stdin.
    PipeOutput,
    // terminal area
    Copy,
    Paste,
    SelectAll,
    SplitRight,
    SplitDown,
    ClearBlocks,
    ClosePane,
    Maximize,
    /// The pane leaves its split for a tab of its own.
    MoveToNewTab,
    /// The toolbar: Settings › Appearance › Toolbar.
    RearrangeToolbar,
    // tab bar (payload: tab index)
    RenameTab(usize),
    DuplicateTab(usize),
    ToggleSync(usize),
    SetTabColor(usize, Option<usize>),
    CloseTab(usize),
    CloseOtherTabs(usize),
    CloseTabsRight(usize),
    // file viewer ⋮
    ViewerInsertPath,
    ViewerCopyPath,
    ViewerFind,
    ViewerRendered(bool),
    ViewerClose,
}

pub struct MenuItem {
    pub label: &'static str,
    pub action: MenuAction,
    pub enabled: bool,
    /// Draw a divider above this row.
    pub separator: bool,
}

impl MenuItem {
    pub fn new(label: &'static str, action: MenuAction) -> Self {
        MenuItem {
            label,
            action,
            enabled: true,
            separator: false,
        }
    }

    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }

    pub fn sep(mut self) -> Self {
        self.separator = true;
        self
    }
}

#[derive(Default)]
pub struct ContextMenuState {
    pub open: bool,
    pub pane: PaneId,
    pub block: BlockId,
    pub pos: (f32, f32),
    pub items: Vec<MenuItem>,
    pub selected: Option<usize>,
}

impl ContextMenuState {
    pub fn close(&mut self) {
        self.open = false;
        self.items.clear();
        self.selected = None;
    }

    /// Keyboard navigation: move to the next enabled row in `dir`.
    pub fn step(&mut self, dir: i64) {
        let n = self.items.len() as i64;
        if n == 0 {
            return;
        }
        let mut i = match self.selected {
            Some(s) => s as i64,
            None if dir > 0 => -1,
            None => n,
        };
        for _ in 0..n {
            i = (i + dir).rem_euclid(n);
            if self.items[i as usize].enabled {
                self.selected = Some(i as usize);
                return;
            }
        }
    }
}

pub const MENU_W: f32 = 240.0;
pub const ROW_H: f32 = 28.0;
const SEP_H: f32 = 9.0;

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

/// The row icon of an action (none for a few plain ones).
fn icon(action: MenuAction) -> Option<Icon> {
    use MenuAction::*;
    Some(match action {
        CopyCommand | CopyOutput | CopyBoth | CopyCwd | Copy | ViewerCopyPath => Icon::Copy,
        CopyBranch => Icon::Branch,
        Rerun => Icon::Refresh,
        ToggleBookmark => Icon::Bookmark,
        ToggleCollapse => Icon::ChevronDown,
        ScrollTopOfBlock => Icon::ChevronUp,
        ScrollBottomOfBlock => Icon::ChevronDown,
        FilterBlock => Icon::Filter,
        PipeOutput | ViewerInsertPath => Icon::Terminal,
        SplitRight => Icon::SplitRight,
        SplitDown => Icon::SplitDown,
        Maximize => Icon::Maximize,
        MoveToNewTab | DuplicateTab(_) => Icon::Plus,
        ToggleSync(_) => Icon::Sync,
        ViewerFind => Icon::Search,
        RearrangeToolbar => Icon::Settings,
        ViewerRendered(_) => Icon::File,
        ClosePane | CloseTab(_) | CloseOtherTabs(_) | ViewerClose => Icon::Close,
        CloseTabsRight(_) => Icon::ChevronRight,
        Paste | SelectAll | ClearBlocks | RenameTab(_) | SetTabColor(..) => return None,
    })
}

/// Keymap actions whose shortcut a row shows (`Keymap::preferred`).
fn shortcut_actions(action: MenuAction) -> &'static [&'static str] {
    use MenuAction::*;
    match action {
        CopyCommand => &["copy_block_command"],
        CopyOutput => &["copy_block_output"],
        ToggleBookmark => &["bookmark_toggle"],
        FilterBlock => &["filter_block"],
        Copy => &["copy"],
        Paste => &["paste"],
        SelectAll => &["select_all"],
        SplitRight => &["split_vertical"],
        SplitDown => &["split_horizontal"],
        Maximize => &["maximize_pane"],
        ClearBlocks => &["clear_blocks"],
        ClosePane | ViewerClose => &["close_pane"],
        ToggleSync(_) => &["toggle_sync_inputs"],
        ViewerFind => &["search"],
        _ => &[],
    }
}

/// Rows that close something read in the error color.
fn destructive(action: MenuAction) -> bool {
    matches!(
        action,
        MenuAction::ClosePane
            | MenuAction::CloseTab(_)
            | MenuAction::CloseOtherTabs(_)
            | MenuAction::CloseTabsRight(_)
    )
}

/// Draw the menu: an icon column, the label, the shortcut from the live
/// keymap; returns per-row rects aligned with `state.items`.
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &ContextMenuState,
    keymap: &rift_core::Keymap,
    w: f32,
    h: f32,
) -> Vec<[f32; 4]> {
    let seps = state.items.iter().filter(|i| i.separator).count() as f32;
    // rows (and separators) shrink together so a tall menu still fits the
    // window and every item stays reachable
    let natural_h = state.items.len() as f32 * ROW_H + seps * SEP_H;
    let scale = ((h - 16.0 - 8.0) / natural_h.max(1.0)).clamp(0.5, 1.0);
    let (row_h, sep_h) = (ROW_H * scale, SEP_H * scale);
    let menu_h = state.items.len() as f32 * row_h + seps * sep_h + 8.0;
    let x = state.pos.0.min(w - MENU_W - 8.0).max(8.0);
    let y = state.pos.1.min(h - menu_h - 8.0).max(8.0);

    layer.shadow(
        [x, y + 6.0],
        [MENU_W, menu_h],
        Color::BLACK.with_alpha(if theme.is_light() { 0.25 } else { 0.5 }),
        4.0,
        14.0,
    );
    let edge = if theme.is_light() {
        Color::BLACK.with_alpha(0.12)
    } else {
        Color::WHITE.with_alpha(0.1)
    };
    layer.bordered([x, y], [MENU_W, menu_h], theme.surface, edge, 4.0, 1.0);

    let font = if row_h < 24.0 { 12.0 } else { 13.0 };
    let mut rects = Vec::new();
    let mut ry = y + 4.0;
    for (i, item) in state.items.iter().enumerate() {
        if item.separator {
            layer.hline(x + 8.0, ry + sep_h / 2.0, MENU_W - 16.0, 1.0, edge);
            ry += sep_h;
        }
        let rect = [x + 4.0, ry, MENU_W - 8.0, row_h];
        if state.selected == Some(i) && item.enabled {
            layer.rect(
                [rect[0], rect[1]],
                [rect[2], rect[3]],
                theme.accent.with_alpha(0.18),
                3.0,
            );
        }
        let base = if destructive(item.action) {
            theme.error
        } else {
            theme.text_ui
        };
        let color = if item.enabled {
            base
        } else {
            theme.text_muted.with_alpha(0.6)
        };
        let cy = ry + row_h / 2.0;
        // icon column; tab-color rows carry a swatch there
        if let MenuAction::SetTabColor(_, Some(c)) = item.action {
            layer.circle([rect[0] + 16.0, cy], 10.0, tab_color(theme, c));
        } else if let Some(icon) = icon(item.action) {
            let ic = if !item.enabled {
                color
            } else if destructive(item.action) {
                theme.error
            } else {
                theme.text_dim
            };
            icons::draw(layer, icon, [rect[0] + 16.0, cy], 14.0, ic);
        }
        layer.text(
            sans(engine, item.label, font, Weight::NORMAL),
            rect[0] + 32.0,
            ry + (row_h - font * 1.35) / 2.0,
            color,
        );
        let keys = shortcut_actions(item.action)
            .iter()
            .find_map(|a| keymap.preferred(a).map(|b| b.chips().concat()));
        if let Some(keys) = keys {
            let kw = engine.measure(&keys, 11.5, FONT_SANS, Weight::NORMAL);
            layer.text(
                sans(engine, &keys, 11.5, Weight::NORMAL),
                rect[0] + rect[2] - 10.0 - kw,
                cy - 7.8,
                if destructive(item.action) && item.enabled {
                    theme.error.with_alpha(0.8)
                } else {
                    theme.text_muted
                },
            );
        }
        rects.push(rect);
        ry += row_h;
    }
    rects
}

/// Render color of tab color `i` (see `state::TAB_COLORS`), from the theme's
/// bright ANSI palette so it follows the theme.
pub fn tab_color(theme: &Theme, i: usize) -> Color {
    // red green yellow blue magenta cyan → ANSI 9..=14
    theme.ansi[9 + i.min(5)]
}
