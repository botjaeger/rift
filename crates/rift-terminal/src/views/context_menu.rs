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
    /// The output with its secrets as printed (`mask_secrets` on: copies
    /// are masked otherwise).
    CopyOutputUnmasked,
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
    /// Forget the block's output filter (query and toggles).
    ClearFilter,
    /// Command and output as HTML with the theme's colors (+ plain text).
    CopyHtml,
    /// Command and fenced output as Markdown.
    CopyMarkdown,
    /// Write the block's output to a file.
    SaveOutput,
    /// Compare with the previous finished run of the same command, in the
    /// same directory, in this pane.
    ComparePrevious,
    // a multi-block selection
    /// Put the selected blocks' commands into the input, one per line.
    ReinputCommands,
    /// Compare the two selected blocks' outputs.
    CompareSelected,
    // terminal area
    /// Write the pane's whole buffer (scrollback and screen) to a file.
    ExportBuffer,
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
    /// The pane leaves for a window of its own (shells keep running).
    MovePaneToNewWindow,
    /// Refuse (or allow again) input into the pane.
    ToggleReadOnly,
    /// Start the pane's program again in place.
    RestartPane,
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
    /// Pin or unpin the tab.
    PinTab(usize),
    /// Group the tab with the picked ones.
    GroupTabs(usize),
    /// The tab leaves its group.
    LeaveGroup(usize),
    // tab group header (payload: group id)
    ToggleGroup(u64),
    RenameGroup(u64),
    Ungroup(u64),
    CloseGroup(u64),
    /// The + button's menu: open profile `.0` (index into the profiles
    /// listed when the menu opened).
    NewTabProfile(usize),
    /// The + button's menu, below its divider.
    OpenSettings,
    OpenPalette,
    OpenAbout,
    /// The tab leaves for a window of its own.
    MoveTabToNewWindow(usize),
    /// The tab moves to another window (tab, that window's place in
    /// opening order).
    MoveTabToWindow(usize, usize),
    // file viewer ⋮
    ViewerInsertPath,
    ViewerCopyPath,
    ViewerFind,
    ViewerRendered(bool),
    ViewerClose,
    /// "Send to agent…" for the code picked when the menu opened (a review
    /// hunk or file, the viewer's lines).
    SendToAgent,
    // a link or file path right-clicked in output (`WindowState::link_menu`)
    LinkOpen,
    LinkCopy,
    LinkReveal,
    /// Ask for tab `.0`'s colour (a name or any `#rrggbb`).
    CustomTabColor(usize),
    /// The pane's sticky command header: minimized ↔ shown.
    ToggleStickyHeader,
    /// A block that printed a Markdown file (`cat README.md`): open that
    /// file in the viewer.
    OpenInViewer,
    // an explorer row right-clicked (`WindowState::explorer_menu`)
    ExplorerOpen,
    ExplorerCopyPath,
    ExplorerCopyRelativePath,
    ExplorerNewFile,
    ExplorerReveal,
    /// The ghost text's hint chip: the key that accepts it.
    AcceptKey(crate::views::editor::AcceptKey),
}

pub struct MenuItem {
    /// Fixed text, or a row built from live state ("Move to window 2: …").
    pub label: std::borrow::Cow<'static, str>,
    pub action: MenuAction,
    pub enabled: bool,
    /// Draw a divider above this row.
    pub separator: bool,
    /// A bold label (the default profile).
    pub bold: bool,
    /// A Nerd Font glyph in the icon column instead of the action's icon.
    pub glyph: Option<char>,
    /// One of a choice (a radio item): `Some(true)` is the current one, a
    /// check in the icon column.
    pub radio: Option<bool>,
}

impl MenuItem {
    pub fn new(label: impl Into<std::borrow::Cow<'static, str>>, action: MenuAction) -> Self {
        MenuItem {
            label: label.into(),
            action,
            enabled: true,
            separator: false,
            bold: false,
            glyph: None,
            radio: None,
        }
    }

    pub fn radio(mut self, checked: bool) -> Self {
        self.radio = Some(checked);
        self
    }

    pub fn bold(mut self, on: bool) -> Self {
        self.bold = on;
        self
    }

    pub fn glyph(mut self, glyph: Option<char>) -> Self {
        self.glyph = glyph;
        self
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
/// The widest a menu grows for a long label and its shortcut; past it a
/// label (a profile's name) ends in … before the shortcut.
const MENU_MAX_W: f32 = 320.0;
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
        CopyCommand
        | CopyOutput
        | CopyOutputUnmasked
        | CopyBoth
        | CopyCwd
        | Copy
        | ViewerCopyPath
        | LinkCopy
        | CopyHtml
        | CopyMarkdown
        | ExplorerCopyPath
        | ExplorerCopyRelativePath => Icon::Copy,
        LinkOpen | LinkReveal | ExplorerOpen | ExplorerReveal | OpenInViewer => Icon::Open,
        ExplorerNewFile => Icon::Plus,
        CustomTabColor(_) => Icon::Themes,
        ToggleStickyHeader => Icon::ChevronUp,
        SaveOutput | ExportBuffer => Icon::File,
        ComparePrevious | CompareSelected => Icon::Refresh,
        ReinputCommands => Icon::Terminal,
        ClearFilter => Icon::Close,
        CopyBranch => Icon::Branch,
        Rerun => Icon::Refresh,
        ToggleBookmark => Icon::Bookmark,
        ToggleCollapse => Icon::ChevronDown,
        ScrollTopOfBlock => Icon::ChevronUp,
        ScrollBottomOfBlock => Icon::ChevronDown,
        FilterBlock => Icon::Filter,
        PipeOutput | ViewerInsertPath | NewTabProfile(_) | SendToAgent => Icon::Terminal,
        OpenSettings => Icon::Settings,
        OpenPalette => Icon::Search,
        OpenAbout => Icon::Info,
        SplitRight => Icon::SplitRight,
        SplitDown => Icon::SplitDown,
        Maximize => Icon::Maximize,
        MoveToNewTab | DuplicateTab(_) => Icon::Plus,
        MovePaneToNewWindow | MoveTabToNewWindow(_) | MoveTabToWindow(..) => Icon::Open,
        ToggleSync(_) => Icon::Sync,
        ToggleReadOnly => Icon::Lock,
        RestartPane => Icon::Refresh,
        ViewerFind => Icon::Search,
        RearrangeToolbar => Icon::Settings,
        ViewerRendered(_) => Icon::File,
        ClosePane | CloseTab(_) | CloseOtherTabs(_) | ViewerClose | CloseGroup(_) => Icon::Close,
        PinTab(_) => Icon::Bookmark,
        GroupTabs(_) | LeaveGroup(_) | Ungroup(_) => Icon::Folder,
        ToggleGroup(_) => Icon::ChevronDown,
        RenameGroup(_) | AcceptKey(_) => return None,
        CloseTabsRight(_) => Icon::ChevronRight,
        Paste | SelectAll | ClearBlocks | RenameTab(_) | SetTabColor(..) => return None,
    })
}

/// The keymap actions that open the first nine profiles, in menu order.
pub const PROFILE_ACTIONS: [&str; 9] = [
    "new_tab_profile_1",
    "new_tab_profile_2",
    "new_tab_profile_3",
    "new_tab_profile_4",
    "new_tab_profile_5",
    "new_tab_profile_6",
    "new_tab_profile_7",
    "new_tab_profile_8",
    "new_tab_profile_9",
];

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
        ToggleReadOnly => &["toggle_read_only"],
        RestartPane => &["restart_pane"],
        ViewerFind => &["search"],
        NewTabProfile(i) => PROFILE_ACTIONS
            .get(i)
            .map(std::slice::from_ref)
            .unwrap_or(&[]),
        OpenSettings => &["open_settings"],
        OpenPalette => &["palette"],
        OpenAbout => &["open_about"],
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
            | MenuAction::CloseGroup(_)
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
    let font = if row_h < 24.0 { 12.0 } else { 13.0 };
    // each row's shortcut from the live keymap, and its column's width
    let shortcuts: Vec<(Option<String>, f32)> = state
        .items
        .iter()
        .map(|item| {
            let keys = shortcut_actions(item.action)
                .iter()
                .find_map(|a| keymap.preferred(a).map(|b| b.chips().concat()));
            let kw = keys
                .as_ref()
                .map(|k| engine.measure(k, 11.5, FONT_SANS, Weight::NORMAL) + 12.0)
                .unwrap_or(0.0);
            (keys, kw)
        })
        .collect();
    // wide enough for every label beside its shortcut, within the cap
    let natural_w = state
        .items
        .iter()
        .zip(&shortcuts)
        .map(|(item, (_, kw))| {
            let weight = if item.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            };
            engine.measure(&item.label, font, FONT_SANS, weight) + 42.0 + kw + 8.0
        })
        .fold(MENU_W, f32::max);
    let menu_w = natural_w.min(MENU_MAX_W).min((w - 16.0).max(MENU_W));
    let x = state.pos.0.min(w - menu_w - 8.0).max(8.0);
    let y = state.pos.1.min(h - menu_h - 8.0).max(8.0);

    layer.shadow(
        [x, y + 6.0],
        [menu_w, menu_h],
        Color::BLACK.with_alpha(if theme.is_light() { 0.25 } else { 0.5 }),
        4.0,
        14.0,
    );
    let edge = if theme.is_light() {
        Color::BLACK.with_alpha(0.12)
    } else {
        Color::WHITE.with_alpha(0.1)
    };
    layer.bordered([x, y], [menu_w, menu_h], theme.surface, edge, 4.0, 1.0);

    let mut rects = Vec::new();
    let mut ry = y + 4.0;
    for (i, item) in state.items.iter().enumerate() {
        if item.separator {
            layer.hline(x + 8.0, ry + sep_h / 2.0, menu_w - 16.0, 1.0, edge);
            ry += sep_h;
        }
        let rect = [x + 4.0, ry, menu_w - 8.0, row_h];
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
        } else if item.radio == Some(true) {
            icons::draw(
                layer,
                Icon::Check,
                [rect[0] + 16.0, cy],
                14.0,
                theme.text_dim,
            );
        } else if let Some(g) = item.glyph {
            let ic = if item.enabled { theme.text_dim } else { color };
            crate::views::glyphs::draw(layer, engine, g, [rect[0] + 16.0, cy], 14.0, ic);
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
        let (keys, kw) = shortcuts[i].clone();
        // a long label (a profile's name) ends in … before the shortcut
        let weight = if item.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        };
        let label =
            super::palette::elide_to(engine, &item.label, font, FONT_SANS, rect[2] - 42.0 - kw);
        layer.text(
            sans(engine, &label, font, weight),
            rect[0] + 32.0,
            ry + (row_h - font * 1.35) / 2.0,
            color,
        );
        if let Some(keys) = keys {
            let kw = kw - 12.0;
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
