use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use rift_core::{Color, Theme};
use rift_renderer::glyphon::{Attrs, Buffer, Family, Metrics, Shaping, Weight};
use rift_renderer::{FONT_SANS, Layer, TextEngine, mono_family, text_color};

use crate::input::AppAction;
use crate::state::{ClickTarget, Hit};
use crate::views::icons::{self, Icon};

/// What running a palette row does.
#[derive(Debug, Clone, PartialEq)]
pub enum PaletteAction {
    App(AppAction),
    OpenSettings,
    SwitchTheme(usize),
    /// The active pane's own colour scheme: a theme of the `SwitchTheme`
    /// list, or `None` for the window theme ("Pane theme: default").
    PaneTheme(Option<usize>),
    RunHistory(String),
    /// ⌃R history search: Enter inserts into the editor instead of running.
    InsertCommand(String),
    /// Index into the app's loaded launch configurations.
    Launch(usize),
    ToggleVim,
    /// `mask_secrets` on or off (before sharing a screen, say).
    ToggleMaskSecrets,
    Search,
    OpenHistory,
    /// Session switcher: activate tab `.0`.
    FocusTab(usize),
    /// Session switcher: focus pane `.1` of tab `.0`.
    FocusPane(usize, rift_pty::PaneId),
    /// Session switcher: show block `.2` of pane `.1` in tab `.0`.
    ShowBlock(usize, rift_pty::PaneId, rift_core::BlockId),
    /// Pipe mode: run the typed command with the block output on stdin.
    RunPipe,
    /// Open a new tab connected to ssh profile `.0`.
    Ssh(usize),
    /// Open a tab reattached to session-service shell `.0`.
    AttachService(u64),
    /// Session switcher: raise window `.0` (pid) at tab `.1`, pane `.2`.
    FocusWindow(u32, usize, u64),
    /// Ask to install remote shell integration on ssh profile `.0`.
    SshInstall(usize),
    /// Open launch profile `.1` (a `[[profiles]]` entry).
    Profile(ProfileOpen, String),
    /// New worktree mode: create the typed branch's worktree, open a tab.
    CreateWorktree,
    /// Save mode: write the pending text to the typed path.
    SaveToPath,
    /// "Rename window…": the query becomes the window's name.
    NameWindow,
    /// Code review: compare against this base.
    ReviewBase(rift_core::review::Base),
    /// "Send to agent…": paste the pending code into this pane.
    SendToPane(rift_pty::PaneId),
    /// Theme picker: keep this theme (config name).
    PickTheme(String),
    /// Tab colour prompt: this colour (none clears it).
    SetTabColor(Option<rift_core::tab_color::TabColor>),
}

/// Where a profile opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileOpen {
    Tab,
    SplitRight,
    SplitDown,
}

/// Palette rows for launch profiles: a new tab, and a split either way.
pub fn profile_items(profiles: &[rift_core::profiles::Profile]) -> Vec<Item> {
    let mut out = Vec::new();
    for (how, verb) in [
        (ProfileOpen::Tab, "New tab"),
        (ProfileOpen::SplitRight, "Split right"),
        (ProfileOpen::SplitDown, "Split down"),
    ] {
        for p in profiles {
            out.push(Item {
                title: format!("{verb}: {}", p.name),
                id: p.command.clone().unwrap_or_default(),
                shortcut: Vec::new(),
                section: Section::Actions,
                action: PaletteAction::Profile(how, p.name.clone()),
                mono_title: false,
            });
        }
    }
    out
}

/// Palette rows for `[[actions]]`: "Action: NAME" with its chord.
pub fn user_action_items(
    actions: &[rift_core::actions::UserAction],
    keymap: &rift_core::Keymap,
) -> Vec<Item> {
    actions
        .iter()
        .enumerate()
        .map(|(i, a)| Item {
            title: format!("Action: {}", a.name.trim()),
            id: String::new(),
            shortcut: keymap
                .preferred(&format!("{}{i}", rift_core::actions::KEYMAP_PREFIX))
                .map(|b| b.chips())
                .unwrap_or_default(),
            section: Section::Actions,
            action: PaletteAction::App(AppAction::UserAction(i as u16)),
            mono_title: false,
        })
        .collect()
}

/// Palette rows for ssh profiles: connect, and install integration.
pub fn ssh_items(profiles: &[rift_core::ssh::SshProfile]) -> Vec<Item> {
    let mut out = Vec::new();
    for (i, p) in profiles.iter().enumerate() {
        out.push(Item {
            title: format!("SSH: {}", p.name),
            id: if p.name == p.host {
                String::new()
            } else {
                p.host.clone()
            },
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::Ssh(i),
            mono_title: false,
        });
    }
    for (i, p) in profiles.iter().enumerate() {
        out.push(Item {
            title: format!("Install Rift shell integration on {}", p.name),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::SshInstall(i),
            mono_title: false,
        });
    }
    out
}

/// What the modal is searching over (Warp parity: ⌘K / ⌃R).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum PaletteMode {
    #[default]
    Commands,
    History,
    /// ⇧⌘O: tabs, panes and recent blocks of this window.
    Sessions,
    /// Block action: the query is a command; the rows preview the output
    /// that will be its stdin.
    PipeOutput,
    /// "New worktree tab…": the query is the new branch's name; the row
    /// says where its worktree goes.
    NewWorktree,
    /// Code review: what the changes are compared against (Uncommitted,
    /// the default branch, any branch).
    ReviewBase,
    /// "Send to agent…": the pane that gets the code, agents first.
    SendTarget,
    /// "Save output…" / "Export buffer…": the query is the file's path.
    SavePath,
    /// "Rename window…": the query is the window's name.
    RenameWindow,
    /// The theme picker: every theme, the selected one previewed live.
    Themes,
    /// The tab's colour: a name or any `#rrggbb` typed.
    TabColor,
}

impl PaletteMode {
    /// A one-off picker or a free-text query for one action: no
    /// mode chips, and ⇥ and the `# @ >` prefixes do not switch modes.
    pub fn takes_text(self) -> bool {
        matches!(
            self,
            PaletteMode::PipeOutput
                | PaletteMode::NewWorktree
                | PaletteMode::ReviewBase
                | PaletteMode::SendTarget
                | PaletteMode::SavePath
                | PaletteMode::RenameWindow
                | PaletteMode::Themes
                | PaletteMode::TabColor
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Section {
    Actions,
    Settings,
    Recent,
    Tabs,
    Panes,
    Blocks,
    Preview,
    /// Review base picker: Uncommitted and the default branch.
    Base,
    /// Review base picker: every local and remote branch.
    Branches,
    /// Send target picker: panes running a coding agent.
    Agents,
}

#[derive(Clone)]
pub struct Item {
    pub title: String,
    pub id: String,
    pub shortcut: Vec<String>,
    pub section: Section,
    pub action: PaletteAction,
    pub mono_title: bool,
}

pub struct MatchedItem {
    pub item: Item,
    pub indices: Vec<u32>,
    pub score: u32,
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub mode: PaletteMode,
    /// The whole query is selected (a prefill such as the worktree
    /// branch's `wt-<n>`): typing replaces it, Backspace clears it.
    pub query_selected: bool,
}

impl PaletteState {
    /// Text typed into the query: it replaces a selected query.
    pub fn type_text(&mut self, text: &str) {
        if std::mem::take(&mut self.query_selected) {
            self.query.clear();
        }
        self.query.push_str(text);
    }

    /// Backspace: clears a selected query, else its last grapheme.
    pub fn backspace(&mut self) {
        if std::mem::take(&mut self.query_selected) {
            self.query.clear();
        } else {
            crate::state::pop_grapheme(&mut self.query);
        }
    }
}

/// App actions, settings and dynamic launch/theme entries.
pub fn base_items(
    theme_names: &[String],
    launch_names: &[String],
    keymap: &rift_core::Keymap,
) -> Vec<Item> {
    let shortcuts: std::collections::BTreeMap<String, Vec<String>> =
        keymap.display_rows().into_iter().collect();
    let item = |title: &str, id: &str, action: PaletteAction| Item {
        title: title.into(),
        id: id.into(),
        shortcut: shortcuts.get(id).cloned().unwrap_or_default(),
        section: Section::Actions,
        action,
        mono_title: false,
    };

    let mut items = vec![
        item("Search in terminal", "search", PaletteAction::Search),
        item("Command history", "history", PaletteAction::OpenHistory),
        item(
            "Go to tab, pane or block",
            "session_switcher",
            PaletteAction::App(AppAction::SessionSwitcher),
        ),
        item(
            "Notifications: finished commands and alerts",
            "attention_center",
            PaletteAction::App(AppAction::AttentionCenter),
        ),
        item(
            "Rich input: compose for the running program",
            "rich_input",
            PaletteAction::App(AppAction::RichInput),
        ),
        item(
            "Show link and path hints",
            "hint_mode",
            PaletteAction::App(AppAction::HintMode),
        ),
        item(
            "Send block output to a command",
            "pipe_block_output",
            PaletteAction::App(AppAction::PipeBlockOutput),
        ),
        item(
            "Toggle vertical tabs",
            "toggle_tab_layout",
            PaletteAction::App(AppAction::ToggleTabLayout),
        ),
        item(
            "Clear saved block transcripts",
            "clear_transcripts",
            PaletteAction::App(AppAction::ClearTranscripts),
        ),
        item("New tab", "new_tab", PaletteAction::App(AppAction::NewTab)),
        item(
            "New worktree tab…",
            "new_worktree_tab",
            PaletteAction::App(AppAction::NewWorktreeTab),
        ),
        item(
            "Reopen closed tab",
            "reopen_closed_tab",
            PaletteAction::App(AppAction::ReopenClosedTab),
        ),
        item(
            "Close pane",
            "close_pane",
            PaletteAction::App(AppAction::ClosePane),
        ),
        item(
            "Next tab",
            "tab_next",
            PaletteAction::App(AppAction::TabNext),
        ),
        item(
            "Previous tab",
            "tab_prev",
            PaletteAction::App(AppAction::TabPrev),
        ),
        item(
            "Move tab left",
            "move_tab_left",
            PaletteAction::App(AppAction::MoveTabLeft),
        ),
        item(
            "Move tab right",
            "move_tab_right",
            PaletteAction::App(AppAction::MoveTabRight),
        ),
        item(
            "Split pane right",
            "split_vertical",
            PaletteAction::App(AppAction::SplitRight),
        ),
        item(
            "Split pane down",
            "split_horizontal",
            PaletteAction::App(AppAction::SplitDown),
        ),
        item(
            "Focus pane left",
            "navigate_left",
            PaletteAction::App(AppAction::FocusLeft),
        ),
        item(
            "Focus pane right",
            "navigate_right",
            PaletteAction::App(AppAction::FocusRight),
        ),
        item(
            "Focus pane above",
            "navigate_up",
            PaletteAction::App(AppAction::FocusUp),
        ),
        item(
            "Focus pane below",
            "navigate_down",
            PaletteAction::App(AppAction::FocusDown),
        ),
        item(
            "Maximize active pane",
            "maximize_pane",
            PaletteAction::App(AppAction::MaximizePane),
        ),
        item(
            "Resize pane left",
            "resize_pane_left",
            PaletteAction::App(AppAction::ResizePaneLeft),
        ),
        item(
            "Resize pane right",
            "resize_pane_right",
            PaletteAction::App(AppAction::ResizePaneRight),
        ),
        item(
            "Resize pane up",
            "resize_pane_up",
            PaletteAction::App(AppAction::ResizePaneUp),
        ),
        item(
            "Resize pane down",
            "resize_pane_down",
            PaletteAction::App(AppAction::ResizePaneDown),
        ),
        item(
            "Equalize pane sizes",
            "equalize_panes",
            PaletteAction::App(AppAction::EqualizePanes),
        ),
        item(
            "Focus terminal input",
            "focus_input",
            PaletteAction::App(AppAction::FocusInput),
        ),
        item(
            "Select previous block",
            "select_block_prev",
            PaletteAction::App(AppAction::SelectBlockPrev),
        ),
        item(
            "Select next block",
            "select_block_next",
            PaletteAction::App(AppAction::SelectBlockNext),
        ),
        item(
            "Toggle block bookmark",
            "bookmark_toggle",
            PaletteAction::App(AppAction::BookmarkToggle),
        ),
        item(
            "Previous bookmark",
            "bookmark_prev",
            PaletteAction::App(AppAction::BookmarkPrev),
        ),
        item(
            "Next bookmark",
            "bookmark_next",
            PaletteAction::App(AppAction::BookmarkNext),
        ),
        item(
            "Copy block command",
            "copy_block_command",
            PaletteAction::App(AppAction::CopyBlockCommand),
        ),
        item(
            "Copy block output",
            "copy_block_output",
            PaletteAction::App(AppAction::CopyBlockOutput),
        ),
        item(
            "Reinput selected command",
            "reinput",
            PaletteAction::App(AppAction::Reinput),
        ),
        item(
            "Open block actions",
            "block_menu",
            PaletteAction::App(AppAction::BlockMenu),
        ),
        item(
            "Clear all blocks",
            "clear_blocks",
            PaletteAction::App(AppAction::ClearBlocks),
        ),
        item(
            "Scroll one page up",
            "scroll_page_up",
            PaletteAction::App(AppAction::ScrollPageUp),
        ),
        item(
            "Scroll one page down",
            "scroll_page_down",
            PaletteAction::App(AppAction::ScrollPageDown),
        ),
        item(
            "Scroll to top",
            "scroll_top",
            PaletteAction::App(AppAction::ScrollTop),
        ),
        item(
            "Scroll to bottom",
            "scroll_bottom",
            PaletteAction::App(AppAction::ScrollBottom),
        ),
        item(
            "Increase font size",
            "font_increase",
            PaletteAction::App(AppAction::FontIncrease),
        ),
        item(
            "Decrease font size",
            "font_decrease",
            PaletteAction::App(AppAction::FontDecrease),
        ),
        item(
            "Reset font size",
            "font_reset",
            PaletteAction::App(AppAction::FontReset),
        ),
        item(
            "Increase window opacity",
            "opacity_increase",
            PaletteAction::App(AppAction::OpacityIncrease),
        ),
        item(
            "Decrease window opacity",
            "opacity_decrease",
            PaletteAction::App(AppAction::OpacityDecrease),
        ),
        item(
            "Reset window opacity",
            "opacity_reset",
            PaletteAction::App(AppAction::OpacityReset),
        ),
        item(
            "Filter block output",
            "filter_block",
            PaletteAction::App(AppAction::FilterBlock),
        ),
        item(
            "Select all",
            "select_all",
            PaletteAction::App(AppAction::SelectAll),
        ),
        item(
            "Rename tab",
            "rename_tab",
            PaletteAction::App(AppAction::RenameTab),
        ),
        item(
            "Close other tabs",
            "close_other_tabs",
            PaletteAction::App(AppAction::CloseOtherTabs),
        ),
        item(
            "Reset time on this tab",
            "reset_tab_timer",
            PaletteAction::App(AppAction::ResetTabTimer),
        ),
        item(
            "Toggle synchronized inputs in tab",
            "toggle_sync_inputs",
            PaletteAction::App(AppAction::ToggleSyncInputs),
        ),
        item(
            "Toggle read-only pane",
            "toggle_read_only",
            PaletteAction::App(AppAction::ToggleReadOnly),
        ),
        item(
            "Restart pane",
            "restart_pane",
            PaletteAction::App(AppAction::RestartPane),
        ),
        item(
            "Pin or unpin tab",
            "pin_tab",
            PaletteAction::App(AppAction::PinTab),
        ),
        item(
            "Search tabs in the rail",
            "focus_rail_search",
            PaletteAction::App(AppAction::FocusRailSearch),
        ),
        item(
            "Toggle rail rows per pane",
            "toggle_rail_panes",
            PaletteAction::App(AppAction::ToggleRailPanes),
        ),
        item(
            "Save current layout as launch configuration",
            "save_launch_config",
            PaletteAction::App(AppAction::SaveLaunchConfig),
        ),
        item(
            "New window",
            "new_window",
            PaletteAction::App(AppAction::NewWindow),
        ),
        item(
            "Toggle fullscreen",
            "toggle_fullscreen",
            PaletteAction::App(AppAction::ToggleFullscreen),
        ),
        item(
            "Toggle always on top",
            "toggle_always_on_top",
            PaletteAction::App(AppAction::ToggleAlwaysOnTop),
        ),
        item(
            "Rename window",
            "rename_window",
            PaletteAction::App(AppAction::RenameWindow),
        ),
        item(
            "Minimize window",
            "minimize",
            PaletteAction::App(AppAction::Minimize),
        ),
        item("Quit Rift", "quit", PaletteAction::App(AppAction::Quit)),
        item(
            "Open settings",
            "open_settings",
            PaletteAction::OpenSettings,
        ),
        item(
            "Open keyboard shortcuts",
            "open_keybindings",
            PaletteAction::App(AppAction::OpenKeybindings),
        ),
        item(
            "About Rift",
            "open_about",
            PaletteAction::App(AppAction::OpenAbout),
        ),
        item(
            "Check for updates",
            "check_for_updates",
            PaletteAction::App(AppAction::CheckForUpdates),
        ),
        item(
            "Install 'rift' command",
            "install_cli",
            PaletteAction::App(AppAction::InstallCli),
        ),
        item(
            "Update Rift",
            "update_rift",
            PaletteAction::App(AppAction::UpdateRift),
        ),
        item(
            "What's new",
            "whats_new",
            PaletteAction::App(AppAction::WhatsNew),
        ),
        item(
            "Theme picker (live preview)",
            "theme_picker",
            PaletteAction::App(AppAction::ThemePicker),
        ),
        item(
            "Cycle tab color",
            "cycle_tab_color",
            PaletteAction::App(AppAction::CycleTabColor),
        ),
        item(
            "Set tab color…",
            "set_tab_color",
            PaletteAction::App(AppAction::SetTabColor),
        ),
        item(
            "Toggle sticky command header in pane",
            "toggle_sticky_header",
            PaletteAction::App(AppAction::ToggleStickyHeader),
        ),
        item(
            "Toggle file explorer",
            "toggle_explorer",
            PaletteAction::App(AppAction::ToggleExplorer),
        ),
        item(
            "Toggle code review",
            "toggle_review",
            PaletteAction::App(AppAction::ToggleReview),
        ),
        item(
            "Focus code review",
            "focus_review",
            PaletteAction::App(AppAction::FocusReview),
        ),
    ];
    items.push(Item {
        title: "Toggle vim keybindings".into(),
        id: "toggle_vim".into(),
        shortcut: Vec::new(),
        section: Section::Settings,
        action: PaletteAction::ToggleVim,
        mono_title: false,
    });
    items.push(Item {
        title: "Toggle secret masking".into(),
        id: "toggle_mask_secrets".into(),
        shortcut: Vec::new(),
        section: Section::Settings,
        action: PaletteAction::ToggleMaskSecrets,
        mono_title: false,
    });
    for (i, name) in launch_names.iter().enumerate() {
        items.push(Item {
            title: format!("Launch configuration: {name}"),
            id: "launch".into(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::Launch(i),
            mono_title: false,
        });
    }
    for (i, name) in theme_names.iter().enumerate() {
        items.push(Item {
            title: format!("Theme: {name}"),
            id: "set_theme".into(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::SwitchTheme(i),
            mono_title: false,
        });
    }
    let pane_themes = std::iter::once((None, "default")).chain(
        theme_names
            .iter()
            .enumerate()
            .map(|(i, n)| (Some(i), n.as_str())),
    );
    for (i, name) in pane_themes {
        items.push(Item {
            title: format!("Pane theme: {name}"),
            id: "set_pane_theme".into(),
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::PaneTheme(i),
            mono_title: false,
        });
    }
    items
}

/// ⌃R mode: every history entry as an insertable row (newest first).
pub fn history_items(history: &[String]) -> Vec<Item> {
    history
        .iter()
        .rev()
        .take(2000)
        .map(|h| Item {
            // shown masked (`mask_secrets`); the action keeps the command
            title: rift_core::secrets::mask(h).into_owned(),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Recent,
            action: PaletteAction::InsertCommand(h.clone()),
            mono_title: true,
        })
        .collect()
}

/// New worktree mode's row: what Enter creates (`main` = the main
/// checkout of the active pane's repository; `None` outside one).
pub fn worktree_items(query: &str, main: Option<&std::path::Path>) -> Vec<MatchedItem> {
    let branch = query.trim();
    let title = match main {
        None => "Not inside a Git repository".to_string(),
        Some(_) if branch.is_empty() => "Type a name for the new branch".to_string(),
        Some(main) => {
            let dir = rift_core::git::worktree_dir(main, branch)
                .map(|d| crate::views::editor::shorten_home(&d.to_string_lossy()))
                .unwrap_or_default();
            format!("Create branch {branch} from HEAD in {dir}")
        }
    };
    vec![MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::CreateWorktree,
            mono_title: false,
        },
        indices: Vec::new(),
        score: 0,
    }]
}

/// "Rename window…": one row that names the window what is typed (or,
/// with nothing typed, clears its name).
pub fn rename_window_items(query: &str) -> Vec<MatchedItem> {
    let name = query.trim();
    let title = if name.is_empty() {
        "Clear this window's name".to_string()
    } else {
        format!("Name this window \u{201c}{name}\u{201d}")
    };
    vec![MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::NameWindow,
            mono_title: false,
        },
        indices: Vec::new(),
        score: 0,
    }]
}

/// Theme picker rows: each theme by its label (`names` and `labels` are
/// `theme::available_themes` and `theme::picker_labels`), the one in use
/// marked, narrowed by the query.
pub fn theme_items(
    names: &[String],
    labels: &[String],
    current: &str,
    query: &str,
) -> Vec<MatchedItem> {
    let items: Vec<Item> = names
        .iter()
        .zip(labels)
        .map(|(name, label)| Item {
            title: label.clone(),
            id: if name == current {
                "in use".into()
            } else {
                String::new()
            },
            shortcut: Vec::new(),
            section: Section::Settings,
            action: PaletteAction::PickTheme(name.clone()),
            mono_title: false,
        })
        .collect();
    rank(&items, &[], query)
}

/// Tab colour prompt rows: a typed `#hex` as its one row (none until it is
/// a whole `#rgb` / `#rrggbb`, so Enter sets nothing half-typed), else the
/// six names and "No color" narrowed by the query.
pub fn tab_color_items(query: &str) -> Vec<MatchedItem> {
    use rift_core::tab_color::{NAMES, TabColor};
    let typed = query.trim();
    if typed.starts_with('#') {
        return TabColor::parse(typed)
            .map(|c| MatchedItem {
                item: Item {
                    title: format!("Set tab color {}", c.to_config()),
                    id: String::new(),
                    shortcut: Vec::new(),
                    section: Section::Actions,
                    action: PaletteAction::SetTabColor(Some(c)),
                    mono_title: false,
                },
                indices: Vec::new(),
                score: 0,
            })
            .into_iter()
            .collect();
    }
    let mut items: Vec<Item> = NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| Item {
            title: format!("{}{}", n[..1].to_uppercase(), &n[1..]),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::SetTabColor(Some(TabColor::Named(i))),
            mono_title: false,
        })
        .collect();
    items.push(Item {
        title: "No color".into(),
        id: String::new(),
        shortcut: Vec::new(),
        section: Section::Actions,
        action: PaletteAction::SetTabColor(None),
        mono_title: false,
    });
    rank(&items, &[], typed)
}

/// Save mode's row: where the text goes (`~` is the home directory), and
/// whether a file is there already (it is replaced).
pub fn save_items(query: &str, what: &str) -> Vec<MatchedItem> {
    let path = query.trim();
    let title = if path.is_empty() {
        format!("Type a path to save {what}")
    } else {
        let full = expand_home(path);
        if full.is_dir() {
            format!(
                "{} is a directory",
                crate::views::editor::shorten_home(path)
            )
        } else if full.exists() {
            format!(
                "Replace {} with {what}",
                crate::views::editor::shorten_home(path)
            )
        } else {
            format!(
                "Save {what} to {}",
                crate::views::editor::shorten_home(path)
            )
        }
    };
    vec![MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::SaveToPath,
            mono_title: false,
        },
        indices: Vec::new(),
        score: 0,
    }]
}

/// New-file prompt's row (explorer "New file…"): the file it creates
/// (`path` is what the typed text resolves to), or why it cannot.
pub fn new_file_items(path: Result<&std::path::Path, &str>) -> Vec<MatchedItem> {
    let title = match path {
        Err(why) => {
            let mut why = why.to_string();
            if let Some(first) = why.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            why
        }
        Ok(p) => {
            let shown = crate::views::editor::shorten_home(&p.to_string_lossy());
            if p.exists() {
                format!("{shown} already exists")
            } else {
                format!("Create {shown}")
            }
        }
    };
    vec![MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Actions,
            action: PaletteAction::SaveToPath,
            mono_title: false,
        },
        indices: Vec::new(),
        score: 0,
    }]
}

/// `path` with a leading `~/` (or a lone `~`) as the home directory.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        (None, Some(home)) if path == "~" => home,
        _ => std::path::PathBuf::from(path),
    }
}

/// Pipe mode rows: what runs, then a preview of the stdin text.
pub fn pipe_items(query: &str, output: &str) -> Vec<MatchedItem> {
    let lines: Vec<&str> = output.lines().collect();
    let row = |title: String, section: Section, mono_title: bool| MatchedItem {
        item: Item {
            title,
            id: String::new(),
            shortcut: Vec::new(),
            section,
            action: PaletteAction::RunPipe,
            mono_title,
        },
        indices: Vec::new(),
        score: 0,
    };
    let head = if query.trim().is_empty() {
        format!("Type a command to receive {} lines on stdin", lines.len())
    } else {
        format!("Run `{}` with {} lines on stdin", query.trim(), lines.len())
    };
    let mut out = vec![row(head, Section::Actions, false)];
    for l in lines.iter().take(6) {
        out.push(row(l.to_string(), Section::Preview, true));
    }
    if lines.len() > 6 {
        out.push(row(
            format!("… {} more lines", lines.len() - 6),
            Section::Preview,
            false,
        ));
    }
    out
}

/// The shell command that runs `command` with `file` as its stdin. The
/// output is never interpolated into the command line, only the (quoted)
/// path of a private file holding it.
pub fn pipe_command(command: &str, file: &str, shell: &str) -> String {
    let quoted = crate::input::shell_quote(file);
    if shell.ends_with("fish") {
        format!("begin; {command}; end < {quoted}")
    } else {
        format!("( {command} ) < {quoted}")
    }
}

/// Fuzzy-rank items + recent history against the query.
pub fn rank(items: &[Item], history: &[String], query: &str) -> Vec<MatchedItem> {
    let mut all: Vec<Item> = items.to_vec();
    for h in history.iter().rev().take(20) {
        all.push(Item {
            // shown masked (`mask_secrets`); the action keeps the command
            title: rift_core::secrets::mask(h).into_owned(),
            id: String::new(),
            shortcut: Vec::new(),
            section: Section::Recent,
            action: PaletteAction::RunHistory(h.clone()),
            mono_title: true,
        });
    }

    if query.is_empty() {
        return all
            .into_iter()
            .map(|item| MatchedItem {
                item,
                indices: vec![],
                score: 0,
            })
            .collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut out: Vec<MatchedItem> = all
        .into_iter()
        .filter_map(|item| {
            let mut buf = Vec::new();
            let hay = Utf32Str::new(&item.title, &mut buf);
            let mut indices = Vec::new();
            pattern
                .indices(hay, &mut matcher, &mut indices)
                .map(|score| MatchedItem {
                    item,
                    indices,
                    score,
                })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.score));
    out
}

const MODAL_W: f32 = 640.0;
const ROW_H: f32 = 36.0;
const SECTION_H: f32 = 28.0;
/// Height of the "No matches" band shown for an empty result list.
const EMPTY_H: f32 = 56.0;
const SEARCH_H: f32 = 48.0;
const MODES_H: f32 = 40.0;
const FOOTER_H: f32 = 34.0;

/// The palette's switchable modes, their chip labels and typed prefixes.
pub const MODES: [(PaletteMode, &str, char); 3] = [
    (PaletteMode::Commands, "Actions", '>'),
    (PaletteMode::History, "History", '#'),
    (PaletteMode::Sessions, "Sessions", '@'),
];

/// The mode a typed prefix switches to (`#` history, `@` sessions, `>`
/// actions).
pub fn mode_for_prefix(c: char) -> Option<PaletteMode> {
    MODES.iter().find(|m| m.2 == c).map(|m| m.0)
}

/// ⇥: the next switchable mode, wrapping.
pub fn next_mode(mode: PaletteMode) -> PaletteMode {
    let i = MODES.iter().position(|m| m.0 == mode).unwrap_or(0);
    MODES[(i + 1) % MODES.len()].0
}

pub(crate) fn sans(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
    buf.set_text(
        &mut engine.font_system,
        text,
        &Attrs::new().family(Family::Name(FONT_SANS)).weight(weight),
        Shaping::Advanced,
        None,
    );
    buf
}

pub(crate) fn mono(engine: &mut TextEngine, text: &str, size: f32, weight: Weight) -> Buffer {
    let mut buf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
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

pub(crate) fn lift(theme: &Theme, alpha: f32) -> Color {
    if theme.is_light() {
        Color::BLACK.with_alpha(alpha)
    } else {
        Color::WHITE.with_alpha(alpha)
    }
}

/// Render the palette modal over a dimmed backdrop: the query, the mode
/// chips, grouped rows with icons and shortcut caps, and a key-hint footer.
/// Row targets go to `row_hits` as (row_rect, index); mode chips to `hits`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    layer: &mut Layer,
    engine: &mut TextEngine,
    theme: &Theme,
    state: &PaletteState,
    results: &[MatchedItem],
    w: f32,
    h: f32,
    blink_on: bool,
    hits: &mut Vec<Hit>,
    row_hits: &mut Vec<([f32; 4], usize)>,
) {
    layer.rect([0.0, 0.0], [w, h], Color::BLACK.with_alpha(0.4), 0.0);

    // the modal narrows with the window (16px margin each side)
    let modal_w = MODAL_W.min((w - 32.0).max(160.0));
    let mx = (w - modal_w) / 2.0;
    let my = crate::views::tabbar::TAB_BAR_H + 54.0;
    let modes_h = if state.mode.takes_text() {
        0.0
    } else {
        MODES_H
    };

    // Keep the keyboard-selected row visible when the result list is longer
    // than the modal. Row hit indices remain absolute into `results`.
    // (fewer rows when the window is short: the modal must fit under the tab bar)
    let room = h - my - SEARCH_H - modes_h - FOOTER_H - 14.0 - 8.0 - SECTION_H;
    let max_rows = ((room / ROW_H).max(0.0) as usize).clamp(1, 9);
    let shown = results.len().min(max_rows);
    let start = state
        .selected
        .saturating_add(1)
        .saturating_sub(max_rows)
        .min(results.len().saturating_sub(shown));
    let visible = &results[start..start + shown];
    let mut list_h = 6.0 * 2.0;
    let mut last_section = None;
    for m in visible {
        if last_section != Some(m.item.section) {
            list_h += SECTION_H;
        }
        last_section = Some(m.item.section);
        list_h += ROW_H;
    }
    // no results: a short "No matches" band instead of a blank gap
    if visible.is_empty() {
        list_h += EMPTY_H;
    }
    let modal_h = SEARCH_H + modes_h + list_h + FOOTER_H;

    layer.shadow(
        [mx, my + 12.0],
        [modal_w, modal_h],
        Color::BLACK.with_alpha(0.55),
        8.0,
        24.0,
    );
    layer.bordered(
        [mx, my],
        [modal_w, modal_h],
        theme.surface,
        lift(theme, 0.1),
        8.0,
        1.0,
    );
    layer.set_clip(Some((
        [mx + 1.0, my + 1.0, modal_w - 2.0, modal_h - 2.0],
        7.0,
    )));

    // ── search row: magnifier, query, esc ──
    let cy = my + SEARCH_H / 2.0;
    icons::draw(
        layer,
        Icon::Search,
        [mx + 16.0 + 8.0, cy],
        16.0,
        theme.text_dim,
    );
    let qx = mx + 16.0 + 16.0 + 10.0;
    if !state.query.is_empty() {
        if state.query_selected {
            let qw = engine.measure(&state.query, 15.0, FONT_SANS, Weight::NORMAL);
            layer.rect(
                [qx - 1.0, cy - 10.0],
                [qw + 2.0, 20.0],
                theme.selection,
                2.0,
            );
        }
        layer.text(
            sans(engine, &state.query, 15.0, Weight::NORMAL),
            qx,
            cy - 10.0,
            theme.text_bright,
        );
    } else {
        let placeholder = match state.mode {
            PaletteMode::Sessions => "Go to tab, pane or block…",
            PaletteMode::PipeOutput => "Command that reads the output on stdin…",
            PaletteMode::NewWorktree => "Name of the new branch…",
            PaletteMode::History => "Search command history…",
            PaletteMode::Commands => "Search actions, or type # @ to switch…",
            PaletteMode::ReviewBase => "Compare changes against…",
            PaletteMode::SendTarget => "Send to which pane…",
            PaletteMode::SavePath => "Path of the file to write…",
            PaletteMode::RenameWindow => "Name of this window…",
            PaletteMode::Themes => "Search themes: ↑ ↓ preview, ⏎ keeps, esc reverts…",
            PaletteMode::TabColor => "Tab color: a name or #rrggbb…",
        };
        layer.text(
            sans(engine, placeholder, 15.0, Weight::NORMAL),
            qx + 4.0,
            cy - 10.0,
            theme.text_muted,
        );
    }
    if blink_on {
        let qw = engine.measure(&state.query, 15.0, FONT_SANS, Weight::NORMAL);
        layer.rect([qx + qw + 1.0, cy - 9.0], [2.0, 18.0], theme.accent, 0.0);
    }
    // result count, then the esc cap
    let esc_w = (engine.measure("esc", 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0);
    let esc_x = mx + modal_w - 16.0 - esc_w;
    super::completion::key_cap(layer, engine, theme, "esc", esc_x, cy);
    let count = format!("{} results", results.len());
    let cw = engine.measure(&count, 11.5, FONT_SANS, Weight::NORMAL);
    let qw_now = engine.measure(&state.query, 15.0, FONT_SANS, Weight::NORMAL);
    if qx + qw_now + 24.0 < esc_x - 12.0 - cw {
        layer.text(
            sans(engine, &count, 11.5, Weight::NORMAL),
            esc_x - 12.0 - cw,
            cy - 7.8,
            theme.text_muted,
        );
    }
    layer.hline(mx + 1.0, my + SEARCH_H, modal_w - 2.0, 1.0, theme.border);

    // ── mode chips ──
    if modes_h > 0.0 {
        let ccy = my + SEARCH_H + MODES_H / 2.0;
        let mut cx = mx + 12.0;
        for (i, (mode, label, prefix)) in MODES.iter().enumerate() {
            let on = state.mode == *mode;
            let lw = engine.measure(label, 12.0, FONT_SANS, Weight::MEDIUM);
            let pw = engine.measure(&prefix.to_string(), 11.0, mono_family(), Weight::NORMAL);
            let r = [cx, ccy - 12.0, 10.0 + lw + 6.0 + pw + 10.0, 24.0];
            if on {
                layer.bordered(
                    [r[0], r[1]],
                    [r[2], r[3]],
                    lift(theme, 0.08),
                    lift(theme, 0.1),
                    4.0,
                    1.0,
                );
            }
            layer.text(
                sans(engine, label, 12.0, Weight::MEDIUM),
                r[0] + 10.0,
                ccy - 8.1,
                if on {
                    theme.text_bright
                } else {
                    theme.text_dim
                },
            );
            layer.text(
                mono(engine, &prefix.to_string(), 11.0, Weight::NORMAL),
                r[0] + 10.0 + lw + 6.0,
                ccy - 7.4,
                theme.text_muted,
            );
            hits.push(Hit {
                rect: r,
                target: ClickTarget::PaletteMode(i as u8),
            });
            cx += r[2] + 4.0;
        }
        layer.hline(
            mx + 1.0,
            my + SEARCH_H + MODES_H,
            modal_w - 2.0,
            1.0,
            theme.border,
        );
    }

    // ── results ──
    let mut y = my + SEARCH_H + modes_h + 6.0;
    if visible.is_empty() {
        let msg = "No matches";
        let mw = engine.measure(msg, 13.0, FONT_SANS, Weight::NORMAL);
        layer.text(
            sans(engine, msg, 13.0, Weight::NORMAL),
            mx + (modal_w - mw) / 2.0,
            y + (EMPTY_H - 17.5) / 2.0,
            theme.text_muted,
        );
    }
    let mut last_section = None;
    for (visible_i, m) in visible.iter().enumerate() {
        let i = start + visible_i;
        if last_section != Some(m.item.section) {
            let label = match m.item.section {
                Section::Actions => match state.mode {
                    PaletteMode::History => "HISTORY",
                    _ => "ACTIONS",
                },
                Section::Settings => "SETTINGS",
                Section::Recent => "RECENT COMMANDS",
                Section::Tabs => "TABS",
                Section::Panes if state.mode == PaletteMode::SendTarget => "OTHER PANES",
                Section::Panes => "PANES",
                Section::Blocks => "RECENT BLOCKS",
                Section::Preview => "OUTPUT (STDIN)",
                Section::Base => "COMPARE AGAINST",
                Section::Branches => "BRANCHES",
                Section::Agents => "AGENTS",
            };
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
            layer.text(lb, mx + 16.0, y + 10.0, theme.text_muted);
            y += SECTION_H;
        }
        last_section = Some(m.item.section);

        let row_rect = [mx + 6.0, y, modal_w - 12.0, ROW_H];
        let selected = i == state.selected;
        if selected {
            layer.rect(
                [row_rect[0], row_rect[1]],
                [row_rect[2], row_rect[3]],
                theme.accent.with_alpha(0.16),
                4.0,
            );
        }
        row_hits.push((row_rect, i));

        let rcy = y + ROW_H / 2.0;
        let icon_color = if selected {
            theme.text_ui
        } else {
            theme.text_dim
        };
        icons::draw(
            layer,
            item_icon(&m.item),
            [row_rect[0] + 10.0 + 8.0, rcy],
            16.0,
            icon_color,
        );

        // title with match highlighting
        let tx = row_rect[0] + 10.0 + 16.0 + 12.0;
        let title_color = if selected {
            theme.text_bright
        } else {
            theme.text_ui
        };
        let (size, family) = if m.item.mono_title {
            (12.5, mono_family())
        } else {
            (13.5, FONT_SANS)
        };
        // everything left of the shortcut caps must fit the row
        let caps_w: f32 = m
            .item
            .shortcut
            .iter()
            .map(|k| (engine.measure(k, 10.5, FONT_SANS, Weight::MEDIUM) + 10.0).max(18.0) + 4.0)
            .sum();
        let max_right = mx + modal_w - 16.0 - caps_w - 8.0;
        // a long title keeps at least 55% of the row; the detail gets the rest
        let avail = max_right - tx;
        let id_w = if m.item.id.is_empty() {
            0.0
        } else {
            engine.measure(&m.item.id, 11.0, mono_family(), Weight::NORMAL) + 12.0
        };
        let full_w = engine.measure(&m.item.title, size, family, Weight::NORMAL);
        let budget = if full_w + id_w <= avail {
            avail
        } else {
            (avail - id_w).max(avail * 0.55)
        };
        let title = elide_to(engine, &m.item.title, size, family, budget);
        let mut tbuf = Buffer::new(&mut engine.font_system, Metrics::new(size, size * 1.35));
        let chars: Vec<char> = title.chars().collect();
        let mut spans: Vec<(String, bool)> = Vec::new();
        for (ci, ch) in chars.iter().enumerate() {
            let matched = m.indices.contains(&(ci as u32));
            match spans.last_mut() {
                Some((s, was)) if *was == matched => s.push(*ch),
                _ => spans.push((ch.to_string(), matched)),
            }
        }
        let rich: Vec<(&str, Attrs)> = spans
            .iter()
            .map(|(s, matched)| {
                let attrs = Attrs::new().family(Family::Name(family));
                if *matched {
                    (
                        s.as_str(),
                        attrs
                            .weight(Weight::SEMIBOLD)
                            .color(text_color(theme.accent)),
                    )
                } else {
                    (s.as_str(), attrs.color(text_color(title_color)))
                }
            })
            .collect();
        tbuf.set_rich_text(
            &mut engine.font_system,
            rich,
            &Attrs::new().family(Family::Name(family)),
            Shaping::Advanced,
            None,
        );
        let title_w = engine.measure(&title, size, family, Weight::NORMAL);
        layer.text(tbuf, tx, y + (ROW_H - size * 1.35) / 2.0, title_color);

        // action id / detail, in whatever room is left
        let id_x = tx + title_w + 12.0;
        if !m.item.id.is_empty() && max_right - id_x > 30.0 {
            let id = elide_to(engine, &m.item.id, 11.0, mono_family(), max_right - id_x);
            layer.text(
                mono(engine, &id, 11.0, Weight::NORMAL),
                id_x,
                y + (ROW_H - 14.85) / 2.0,
                theme.text_muted,
            );
        }

        // shortcut caps (right-aligned)
        let mut kx = mx + modal_w - 16.0 - caps_w + 4.0;
        for key in &m.item.shortcut {
            kx += super::completion::key_cap(layer, engine, theme, key, kx, rcy) + 4.0;
        }

        y += ROW_H;
    }

    // ── footer ──
    let fy = my + modal_h - FOOTER_H;
    layer.rect(
        [mx, fy],
        [modal_w, FOOTER_H],
        Color::BLACK.with_alpha(0.14),
        0.0,
    );
    layer.hline(mx + 1.0, fy, modal_w - 2.0, 1.0, theme.border);
    let enter = match state.mode {
        PaletteMode::Sessions => "go to",
        PaletteMode::PipeOutput => "run with output on stdin",
        PaletteMode::NewWorktree => "create worktree and open a tab",
        PaletteMode::ReviewBase => "compare",
        PaletteMode::SendTarget => "paste (no Enter)",
        PaletteMode::SavePath => "write the file",
        PaletteMode::RenameWindow => "name the window",
        PaletteMode::Themes => "keep this theme",
        PaletteMode::TabColor => "set the color",
        _ => "run",
    };
    let hints: Vec<(&[&str], &str)> = if modes_h > 0.0 {
        vec![
            (&["↑", "↓"], "navigate"),
            (&["⏎"], enter),
            (&["⇥"], "next mode"),
        ]
    } else {
        vec![(&["↑", "↓"], "navigate"), (&["⏎"], enter)]
    };
    super::completion::key_hints(
        layer,
        engine,
        theme,
        &hints,
        mx + 16.0,
        fy + FOOTER_H / 2.0,
        mx + modal_w - 16.0,
    );
    layer.set_clip(None);
}

/// Longest prefix of `text` (plus `…`) that fits `max_w`.
pub(crate) fn elide_to(
    engine: &mut TextEngine,
    text: &str,
    size: f32,
    family: &'static str,
    max_w: f32,
) -> String {
    if engine.measure(text, size, family, Weight::NORMAL) <= max_w {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    // binary search on the prefix length
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if engine.measure(&candidate, size, family, Weight::NORMAL) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

/// The row icon of an item, from the shared icon set.
fn item_icon(item: &Item) -> Icon {
    match &item.action {
        PaletteAction::Search => Icon::Search,
        PaletteAction::App(AppAction::SplitRight) => Icon::SplitRight,
        PaletteAction::App(AppAction::SplitDown) => Icon::SplitDown,
        PaletteAction::App(AppAction::NewTab) | PaletteAction::Profile(ProfileOpen::Tab, _) => {
            Icon::Plus
        }
        PaletteAction::Profile(ProfileOpen::SplitRight, _) => Icon::SplitRight,
        PaletteAction::Profile(ProfileOpen::SplitDown, _) => Icon::SplitDown,
        PaletteAction::App(AppAction::ClosePane) => Icon::Close,
        PaletteAction::App(AppAction::ToggleExplorer) => Icon::Sidebar,
        PaletteAction::OpenSettings => Icon::Settings,
        PaletteAction::SwitchTheme(_) | PaletteAction::PaneTheme(_) => Icon::Themes,
        PaletteAction::Launch(_) => Icon::Launch,
        PaletteAction::InsertCommand(_)
        | PaletteAction::RunHistory(_)
        | PaletteAction::OpenHistory => Icon::History,
        PaletteAction::ToggleVim => Icon::Keyboard,
        PaletteAction::ToggleMaskSecrets => Icon::Lock,
        PaletteAction::FocusTab(_)
        | PaletteAction::FocusPane(..)
        | PaletteAction::FocusWindow(..) => Icon::Terminal,
        PaletteAction::ShowBlock(..) => Icon::ChevronRight,
        PaletteAction::RunPipe => Icon::Filter,
        PaletteAction::CreateWorktree => Icon::Terminal,
        PaletteAction::Ssh(_) | PaletteAction::SshInstall(_) | PaletteAction::AttachService(_) => {
            Icon::Open
        }
        PaletteAction::ReviewBase(rift_core::review::Base::Uncommitted) => Icon::File,
        PaletteAction::ReviewBase(_) => Icon::Branch,
        PaletteAction::SendToPane(_) => Icon::Terminal,
        _ => Icon::ChevronRight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selected_prefill_is_replaced_by_typing() {
        let mut s = PaletteState {
            query: "wt-2".into(),
            query_selected: true,
            ..Default::default()
        };
        s.type_text("f");
        s.type_text("ix");
        assert_eq!(s.query, "fix");
        assert!(!s.query_selected);
        s.query = "wt-3".into();
        s.query_selected = true;
        s.backspace();
        assert_eq!(s.query, "");
        s.type_text("ab");
        s.backspace();
        assert_eq!(s.query, "a", "unselected: one character");
    }

    #[test]
    fn command_palette_exposes_full_action_set_and_resolved_shortcuts() {
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("open_settings".into(), "ctrl+,".into());
        let keymap = rift_core::Keymap::from_overrides(&overrides);
        let items = base_items(&["dark-default".into()], &[], &keymap);

        assert!(items.len() >= 42);
        assert!(items.iter().any(|i| i.title == "Command history"));
        assert!(items.iter().any(|i| i.title == "Maximize active pane"));
        assert!(items.iter().any(|i| i.title == "Copy block output"));

        let settings = items
            .iter()
            .find(|i| i.id == "open_settings")
            .expect("settings action");
        assert_eq!(settings.shortcut, vec!["⌃", ","]);
    }

    #[test]
    fn worktree_row_says_where_the_branch_goes() {
        let main = std::path::Path::new("/src/rift");
        let rows = worktree_items(" wt-2 ", Some(main));
        assert_eq!(
            rows[0].item.title,
            "Create branch wt-2 from HEAD in /src/rift.worktrees/wt-2"
        );
        assert_eq!(rows[0].item.action, PaletteAction::CreateWorktree);
        assert_eq!(
            worktree_items("", Some(main))[0].item.title,
            "Type a name for the new branch"
        );
        assert_eq!(
            worktree_items("x", None)[0].item.title,
            "Not inside a Git repository"
        );
        assert!(PaletteMode::NewWorktree.takes_text() && !PaletteMode::Commands.takes_text());
        // listed among the actions under its keymap name
        let items = base_items(
            &[],
            &[],
            &rift_core::Keymap::from_overrides(&Default::default()),
        );
        let item = items.iter().find(|i| i.id == "new_worktree_tab").unwrap();
        assert_eq!(item.title, "New worktree tab…");
    }

    #[test]
    fn pipe_rows_preview_output_and_never_embed_it() {
        let rows = pipe_items("wc -l", "a\nb\n$(rm -rf ~)\n");
        assert_eq!(rows[0].item.title, "Run `wc -l` with 3 lines on stdin");
        assert_eq!(rows[3].item.title, "$(rm -rf ~)");
        assert!(rows.iter().all(|r| r.item.action == PaletteAction::RunPipe));
        let cmd = pipe_command("grep x | wc -l", "/tmp/rift pipe/1.txt", "/bin/zsh");
        assert_eq!(cmd, "( grep x | wc -l ) < '/tmp/rift pipe/1.txt'");
        let fish = pipe_command("wc -l", "/tmp/p.txt", "/opt/homebrew/bin/fish");
        assert_eq!(fish, "begin; wc -l; end < /tmp/p.txt");
    }

    #[test]
    fn tab_color_rows_take_names_and_whole_hex_only() {
        use rift_core::tab_color::TabColor;
        let all = tab_color_items("");
        assert_eq!(all.len(), 7, "six names and No color");
        assert_eq!(all[6].item.action, PaletteAction::SetTabColor(None));
        let typed = tab_color_items("#123abc");
        assert_eq!(typed.len(), 1);
        assert_eq!(
            typed[0].item.action,
            PaletteAction::SetTabColor(TabColor::parse("#123abc"))
        );
        // half-typed: nothing Enter could set
        assert!(tab_color_items("#12").is_empty());
        assert!(tab_color_items("#1234").is_empty());
        let blue = tab_color_items("blu");
        assert_eq!(
            blue[0].item.action,
            PaletteAction::SetTabColor(Some(TabColor::Named(3)))
        );
        assert!(PaletteMode::TabColor.takes_text() && PaletteMode::Themes.takes_text());
    }

    #[test]
    fn theme_rows_mark_the_theme_in_use() {
        let names = vec!["dark-default".to_string(), "nord".to_string()];
        let labels = vec!["Dark Default".to_string(), "Nord".to_string()];
        let rows = theme_items(&names, &labels, "nord", "");
        let nord = rows
            .iter()
            .find(|m| m.item.title == "Nord")
            .expect("nord row");
        assert_eq!(nord.item.id, "in use");
        assert_eq!(nord.item.action, PaletteAction::PickTheme("nord".into()));
        assert_eq!(theme_items(&names, &labels, "nord", "dark").len(), 1);
    }
}
