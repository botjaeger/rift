//! Native macOS menu bar (Shift · File · Edit · View · Blocks · Window ·
//! Help). Every item names an app action or an editor chord; activating one
//! replays the item's key chord through `App::on_key`, so a menu click and
//! the keyboard shortcut take exactly the same route (overlays, editor vs
//! raw mode). Shortcut labels come from the live keymap, so `[keybindings]`
//! overrides show up in the menus after a config reload.

use muda::accelerator::Accelerator;
use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use shift_core::Keymap;

/// What a menu item does when chosen.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuCommand {
    /// A keymap action name (`new_tab`, `palette`, …).
    Action(String),
    /// A fixed editor chord that is not a keymap action (`cmd+z`).
    Chord(String),
}

impl MenuCommand {
    fn id(&self) -> String {
        match self {
            MenuCommand::Action(a) => format!("action:{a}"),
            MenuCommand::Chord(c) => format!("chord:{c}"),
        }
    }

    pub fn from_id(id: &str) -> Option<MenuCommand> {
        if let Some(a) = id.strip_prefix("action:") {
            return Some(MenuCommand::Action(a.to_string()));
        }
        id.strip_prefix("chord:")
            .map(|c| MenuCommand::Chord(c.to_string()))
    }
}

/// The installed menu (kept alive for as long as the app runs).
pub struct MenuBar {
    _menu: Menu,
}

/// Shifted US symbols → the physical key muda names.
fn physical(key: &str) -> Option<(&'static str, bool)> {
    Some(match key {
        "}" => ("]", true),
        "{" => ("[", true),
        "+" => ("=", true),
        "_" => ("-", true),
        "|" => ("\\", true),
        ":" => (";", true),
        "\"" => ("'", true),
        "<" => (",", true),
        ">" => (".", true),
        "?" => ("/", true),
        "~" => ("`", true),
        _ => return None,
    })
}

/// Menu accelerator for a chord. Only ⌘ chords become key equivalents: a
/// ctrl-only equivalent would steal the key from programs in the terminal.
fn accelerator(chord: &str) -> Option<Accelerator> {
    let b = shift_core::keybindings::Binding::parse(chord)?;
    if !b.cmd {
        return None;
    }
    let (key, extra_shift) = match physical(&b.key) {
        Some((k, s)) => (k.to_string(), s),
        None => (b.key.clone(), false),
    };
    let mut parts = Vec::new();
    if b.ctrl {
        parts.push("ctrl");
    }
    if b.alt {
        parts.push("alt");
    }
    if b.shift || extra_shift {
        parts.push("shift");
    }
    parts.push("cmd");
    let key = match key.as_str() {
        "left" => "ArrowLeft".to_string(),
        "right" => "ArrowRight".to_string(),
        "up" => "ArrowUp".to_string(),
        "down" => "ArrowDown".to_string(),
        "enter" => "Enter".to_string(),
        "tab" => "Tab".to_string(),
        k => k.to_uppercase(),
    };
    format!("{}+{key}", parts.join("+")).parse().ok()
}

/// The chord an item replays: its action's first ⌘ binding, else its first
/// binding of any kind.
pub fn chord_for(keymap: &Keymap, action: &str) -> Option<String> {
    let all = keymap.bindings_for(action);
    all.iter()
        .find(|b| b.cmd)
        .or(all.first())
        .map(|b| b.chord())
}

fn item(keymap: &Keymap, label: &str, cmd: MenuCommand) -> MenuItem {
    let chord = match &cmd {
        MenuCommand::Action(a) => chord_for(keymap, a),
        MenuCommand::Chord(c) => Some(c.clone()),
    };
    let accel = chord.as_deref().and_then(accelerator);
    MenuItem::with_id(cmd.id(), label, true, accel)
}

fn action(keymap: &Keymap, label: &str, name: &str) -> MenuItem {
    item(keymap, label, MenuCommand::Action(name.to_string()))
}

fn chord(keymap: &Keymap, label: &str, chord: &str) -> MenuItem {
    item(keymap, label, MenuCommand::Chord(chord.to_string()))
}

/// Build and install the app menu for the current keymap.
pub fn install(keymap: &Keymap) -> Option<MenuBar> {
    let sep = PredefinedMenuItem::separator;
    let about = muda::AboutMetadata {
        name: Some("Shift".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        ..Default::default()
    };
    let app = Submenu::with_items(
        "Shift",
        true,
        &[
            &PredefinedMenuItem::about(Some("About Shift"), Some(about)),
            &sep(),
            &action(keymap, "Settings…", "open_settings"),
            &action(keymap, "Keyboard Shortcuts", "open_keybindings"),
            &sep(),
            &PredefinedMenuItem::services(None),
            &sep(),
            &action(keymap, "Hide Shift", "hide_app"),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &sep(),
            // through the app, so running commands still get the warning
            &action(keymap, "Quit Shift", "quit"),
        ],
    )
    .ok()?;
    let file = Submenu::with_items(
        "File",
        true,
        &[
            &action(keymap, "New Tab", "new_tab"),
            &action(keymap, "New Window", "new_window"),
            &sep(),
            &action(keymap, "Split Pane Right", "split_vertical"),
            &action(keymap, "Split Pane Down", "split_horizontal"),
            &sep(),
            &action(keymap, "Reopen Closed Tab", "reopen_closed_tab"),
            &action(
                keymap,
                "Save Layout as Launch Configuration",
                "save_launch_config",
            ),
            &sep(),
            &action(keymap, "Close Pane", "close_pane"),
            &action(keymap, "Close Other Tabs", "close_other_tabs"),
            &action(keymap, "Close Window", "close_window"),
        ],
    )
    .ok()?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &chord(keymap, "Undo", "cmd+z"),
            &chord(keymap, "Redo", "shift+cmd+z"),
            &sep(),
            &chord(keymap, "Cut", "cmd+x"),
            &action(keymap, "Copy", "copy"),
            &action(keymap, "Paste", "paste"),
            &action(keymap, "Select All", "select_all"),
            &sep(),
            &action(keymap, "Find…", "search"),
            &action(keymap, "Filter Block Output…", "filter_block"),
            &sep(),
            &action(keymap, "Clear Blocks", "clear_blocks"),
        ],
    )
    .ok()?;
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &action(keymap, "Command Palette", "palette"),
            &action(keymap, "Workflows", "workflows"),
            &chord(keymap, "Command History", "ctrl+r"),
            &sep(),
            &action(keymap, "File Explorer", "toggle_explorer"),
            &action(keymap, "Code Review", "toggle_review"),
            &sep(),
            &action(keymap, "Bigger", "font_increase"),
            &action(keymap, "Smaller", "font_decrease"),
            &action(keymap, "Actual Size", "font_reset"),
            &sep(),
            &action(keymap, "Maximize Pane", "maximize_pane"),
            &action(keymap, "Toggle Full Screen", "toggle_fullscreen"),
        ],
    )
    .ok()?;
    let blocks = Submenu::with_items(
        "Blocks",
        true,
        &[
            &action(keymap, "Select Previous Block", "select_block_prev"),
            &action(keymap, "Select Next Block", "select_block_next"),
            &sep(),
            &action(keymap, "Copy Command", "copy_block_command"),
            &action(keymap, "Copy Output", "copy_block_output"),
            &action(keymap, "Reinput Command", "reinput"),
            &action(keymap, "Block Actions…", "block_menu"),
            &sep(),
            &action(keymap, "Toggle Bookmark", "bookmark_toggle"),
            &action(keymap, "Previous Bookmark", "bookmark_prev"),
            &action(keymap, "Next Bookmark", "bookmark_next"),
            &sep(),
            &action(keymap, "Scroll to Top", "scroll_top"),
            &action(keymap, "Scroll to Bottom", "scroll_bottom"),
        ],
    )
    .ok()?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &action(keymap, "Minimize", "minimize"),
            &PredefinedMenuItem::zoom(None),
            &sep(),
            &action(keymap, "Show Next Tab", "tab_next"),
            &action(keymap, "Show Previous Tab", "tab_prev"),
            &action(keymap, "Move Tab Left", "move_tab_left"),
            &action(keymap, "Move Tab Right", "move_tab_right"),
            &action(keymap, "Rename Tab…", "rename_tab"),
            &sep(),
            &action(keymap, "Select Pane Left", "navigate_left"),
            &action(keymap, "Select Pane Right", "navigate_right"),
            &action(keymap, "Select Pane Above", "navigate_up"),
            &action(keymap, "Select Pane Below", "navigate_down"),
            &action(keymap, "Synchronize Inputs in Tab", "toggle_sync_inputs"),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    )
    .ok()?;
    let help = Submenu::with_items(
        "Help",
        true,
        &[
            &action(keymap, "What's New", "whats_new"),
            &action(keymap, "Keyboard Shortcuts", "open_keybindings"),
        ],
    )
    .ok()?;
    let menu = Menu::with_items(&[&app, &file, &edit, &view, &blocks, &window, &help]).ok()?;
    menu.init_for_nsapp();
    log::info!("menu bar installed ({} menus)", menu.items().len());
    window.set_as_windows_menu_for_nsapp();
    help.set_as_help_menu_for_nsapp();
    Some(MenuBar { _menu: menu })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_only_for_cmd_chords() {
        assert!(accelerator("cmd+t").is_some());
        assert!(accelerator("shift+cmd+}").is_some());
        assert!(accelerator("shift+cmd+plus").is_some());
        assert!(accelerator("alt+cmd+left").is_some());
        assert!(accelerator("cmd+,").is_some());
        assert!(accelerator("ctrl+shift+t").is_none());
        assert!(accelerator("pageup").is_none());
    }

    #[test]
    fn menu_ids_round_trip() {
        for c in [
            MenuCommand::Action("new_tab".into()),
            MenuCommand::Chord("cmd+z".into()),
        ] {
            assert_eq!(MenuCommand::from_id(&c.id()), Some(c));
        }
        assert_eq!(MenuCommand::from_id("about"), None);
    }

    #[test]
    fn chords_prefer_cmd_bindings() {
        let km = Keymap::from_overrides(&Default::default());
        assert_eq!(chord_for(&km, "new_tab").as_deref(), Some("cmd+t"));
        assert_eq!(chord_for(&km, "palette").as_deref(), Some("cmd+p"));
        // no ⌘ binding: falls back to the plain one
        assert_eq!(chord_for(&km, "scroll_top").as_deref(), Some("home"));
    }
}
