/// Keybinding parsing + lookup. Defaults per spec §4; `[keybindings]`
/// overrides from config.toml win last.
use std::collections::BTreeMap;

/// Named keys a chord may use (what the app's key normalizer produces).
pub const NAMED_KEYS: &[&str] = &[
    "tab",
    "left",
    "right",
    "up",
    "down",
    "pageup",
    "pagedown",
    "home",
    "end",
    "enter",
    "escape",
    "space",
    "backspace",
    "delete",
    "insert",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub cmd: bool,
    /// Lowercase character ("t") or named key ("tab", "left", "pageup", ",").
    pub key: String,
}

impl Binding {
    /// Parse "ctrl+shift+t", "cmd+,", "alt+left", "shift+pageup".
    pub fn parse(s: &str) -> Option<Binding> {
        let mut b = Binding {
            ctrl: false,
            shift: false,
            alt: false,
            cmd: false,
            key: String::new(),
        };
        for part in s.split('+') {
            let p = part.trim().to_lowercase();
            match p.as_str() {
                "ctrl" | "control" | "^" => b.ctrl = true,
                "shift" | "⇧" => b.shift = true,
                "alt" | "opt" | "option" | "meta" => b.alt = true,
                "cmd" | "super" | "command" | "⌘" => b.cmd = true,
                "" => return None,
                _ => {
                    if !b.key.is_empty() {
                        return None; // two non-modifier keys
                    }
                    // "plus" alias: a literal '+' key can't survive split('+')
                    let key = if p == "plus" { "+".into() } else { p };
                    // one character, or a key name input::key_string emits:
                    // anything else could never match and only unbinds
                    if key.chars().count() != 1 && !NAMED_KEYS.contains(&key.as_str()) {
                        return None;
                    }
                    b.key = key;
                }
            }
        }
        if b.key.is_empty() { None } else { Some(b) }
    }

    /// Canonical chord string (`ctrl+alt+shift+cmd+key`) — parses back to
    /// the same binding.
    pub fn chord(&self) -> String {
        let mut s = String::new();
        for (on, name) in [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
            (self.cmd, "cmd+"),
        ] {
            if on {
                s.push_str(name);
            }
        }
        s.push_str(if self.key == "+" { "plus" } else { &self.key });
        s
    }

    /// Display chips, macOS style (spec: shortcuts shown for macOS).
    pub fn chips(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ctrl {
            out.push("⌃".into());
        }
        if self.alt {
            out.push("⌥".into());
        }
        if self.shift {
            out.push("⇧".into());
        }
        if self.cmd {
            out.push("⌘".into());
        }
        out.push(match self.key.as_str() {
            "left" => "←".into(),
            "right" => "→".into(),
            "up" => "↑".into(),
            "down" => "↓".into(),
            "tab" => "Tab".into(),
            "pageup" => "PgUp".into(),
            "pagedown" => "PgDn".into(),
            k => k.to_uppercase(),
        });
        out
    }
}

/// Default action → binding table (spec §4, Warp-parity layout).
pub fn defaults() -> Vec<(&'static str, &'static str)> {
    vec![
        ("new_tab", "ctrl+shift+t"),
        ("new_tab_alt", "cmd+t"),
        ("close_pane", "ctrl+shift+w"),
        ("close_pane_alt", "cmd+w"),
        ("split_vertical", "ctrl+shift+d"),
        ("split_vertical_alt", "cmd+d"),
        ("split_horizontal", "ctrl+shift+e"),
        ("split_horizontal_alt", "shift+cmd+d"),
        ("navigate_left", "alt+cmd+left"),
        ("navigate_right", "alt+cmd+right"),
        ("navigate_up", "alt+cmd+up"),
        ("navigate_down", "alt+cmd+down"),
        ("tab_next", "ctrl+tab"),
        ("tab_next_alt", "shift+cmd+}"),
        ("tab_prev", "ctrl+shift+tab"),
        ("tab_prev_alt", "shift+cmd+{"),
        ("copy", "ctrl+shift+c"),
        ("copy_alt", "cmd+c"),
        ("paste", "ctrl+shift+v"),
        ("paste_alt", "cmd+v"),
        ("search", "ctrl+shift+f"),
        ("search_alt", "cmd+f"),
        ("bookmark_toggle", "ctrl+shift+b"),
        ("bookmark_toggle_alt", "cmd+b"),
        ("bookmark_prev", "ctrl+shift+up"),
        ("bookmark_prev_alt", "cmd+shift+up"),
        ("bookmark_next", "ctrl+shift+down"),
        ("bookmark_next_alt", "cmd+shift+down"),
        ("bookmark_closest_prev", "alt+up"),
        ("bookmark_closest_next", "alt+down"),
        ("scroll_up", "shift+pageup"),
        ("scroll_down", "shift+pagedown"),
        ("scroll_page_up", "pageup"),
        ("scroll_page_down", "pagedown"),
        ("scroll_top", "home"),
        ("scroll_bottom", "end"),
        ("open_settings", "cmd+,"),
        ("open_keybindings", "ctrl+cmd+k"),
        ("palette", "cmd+p"),
        ("palette_alt", "ctrl+shift+p"),
        ("clear_blocks", "cmd+k"),
        ("tab_select_1", "cmd+1"),
        ("tab_select_2", "cmd+2"),
        ("tab_select_3", "cmd+3"),
        ("tab_select_4", "cmd+4"),
        ("tab_select_5", "cmd+5"),
        ("tab_select_6", "cmd+6"),
        ("tab_select_7", "cmd+7"),
        ("tab_select_8", "cmd+8"),
        ("tab_select_9", "cmd+9"),
        ("move_tab_left", "ctrl+shift+left"),
        ("move_tab_right", "ctrl+shift+right"),
        ("reopen_closed_tab", "shift+cmd+t"),
        ("font_increase", "cmd+="),
        ("font_increase_alt", "shift+cmd+plus"),
        ("font_decrease", "cmd+-"),
        ("font_reset", "cmd+0"),
        ("focus_input", "cmd+l"),
        ("maximize_pane", "shift+cmd+enter"),
        ("resize_pane_left", "ctrl+cmd+left"),
        ("resize_pane_right", "ctrl+cmd+right"),
        ("resize_pane_up", "ctrl+cmd+up"),
        ("resize_pane_down", "ctrl+cmd+down"),
        ("select_block_prev", "cmd+up"),
        ("select_block_next", "cmd+down"),
        ("copy_block_command", "shift+cmd+c"),
        ("copy_block_output", "alt+shift+cmd+c"),
        ("reinput", "cmd+i"),
        ("block_menu", "ctrl+m"),
        ("filter_block", "alt+shift+f"),
        ("toggle_sync_inputs", "alt+cmd+i"),
        ("select_all", "cmd+a"),
        ("new_window", "cmd+n"),
        ("close_window", "shift+cmd+w"),
        ("minimize", "cmd+m"),
        ("hide_app", "cmd+h"),
        ("toggle_fullscreen", "ctrl+cmd+f"),
        ("quit", "cmd+q"),
        ("session_switcher", "shift+cmd+o"),
        ("hint_mode", "shift+cmd+h"),
        ("attention_center", "shift+cmd+a"),
        ("toggle_explorer", "shift+cmd+e"),
        ("toggle_review", "shift+cmd+g"),
    ]
}

/// Compiled keymap: binding → action name.
#[derive(Debug, Clone)]
pub struct Keymap {
    entries: Vec<(Binding, String)>,
}

impl Keymap {
    /// Defaults + user overrides (overrides win: they are matched first and
    /// also unbind the default for the same action).
    pub fn from_overrides(overrides: &BTreeMap<String, String>) -> Keymap {
        let mut entries: Vec<(Binding, String)> = Vec::new();
        let known = all_actions();
        for (action, binding) in overrides {
            if !known.contains(&action.as_str()) {
                log::warn!("unknown keybinding action {action:?} (ignored)");
                continue;
            }
            // `none` (or empty) unbinds the action's defaults
            if matches!(binding.trim(), "" | "none") {
                continue;
            }
            if let Some(b) = Binding::parse(binding) {
                entries.push((b, action.clone()));
            } else {
                log::warn!("invalid keybinding for {action}: {binding:?}");
            }
        }
        for (action, binding) in defaults() {
            let base = action.strip_suffix("_alt").unwrap_or(action);
            if overrides.contains_key(base) {
                continue; // user override replaces default (and its alt)
            }
            if let Some(b) = Binding::parse(binding) {
                entries.push((b, base.to_string()));
            }
        }
        Keymap { entries }
    }

    /// `key` must be pre-normalized: lowercase char or named key string.
    pub fn lookup(&self, ctrl: bool, shift: bool, alt: bool, cmd: bool, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(b, _)| {
                b.ctrl == ctrl && b.shift == shift && b.alt == alt && b.cmd == cmd && b.key == key
            })
            .map(|(_, a)| a.as_str())
    }

    /// Every binding of `action`, in match order (overrides first).
    pub fn bindings_for(&self, action: &str) -> Vec<&Binding> {
        self.entries
            .iter()
            .filter(|(_, a)| a == action)
            .map(|(b, _)| b)
            .collect()
    }

    /// The binding of `action` to show: its first ⌘ one on macOS, its first
    /// one without ⌘ elsewhere (the `_alt` defaults are stored under the
    /// base name, after the Linux-style chord), else its first.
    pub fn preferred(&self, action: &str) -> Option<&Binding> {
        let all = self.bindings_for(action);
        all.iter()
            .find(|b| b.cmd == cfg!(target_os = "macos"))
            .or(all.first())
            .copied()
    }

    /// For the settings Keybindings page: every bindable action with the
    /// chips of its preferred binding (empty when unbound), sorted by name.
    pub fn display_rows(&self) -> Vec<(String, Vec<String>)> {
        let mut rows: Vec<(String, Vec<String>)> = all_actions()
            .into_iter()
            .map(|a| {
                let chips = self.preferred(a).map(|b| b.chips()).unwrap_or_default();
                (a.to_string(), chips)
            })
            .collect();
        rows.sort();
        rows
    }

    /// The action (other than `except`) already bound to `b`, if any.
    pub fn conflict(&self, b: &Binding, except: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(eb, a)| eb == b && a != except)
            .map(|(_, a)| a.as_str())
    }
}

/// Every action name `[keybindings]` accepts (defaults' base names plus the
/// actions that ship unbound).
pub fn all_actions() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (action, _) in defaults() {
        let base = action.strip_suffix("_alt").unwrap_or(action);
        if !out.contains(&base) {
            out.push(base);
        }
    }
    for extra in [
        "rename_tab",
        "close_other_tabs",
        "save_launch_config",
        "pipe_block_output",
        "toggle_tab_layout",
        "clear_transcripts",
        "whats_new",
        "open_about",
        "check_for_updates",
        "update_rift",
    ] {
        if !out.contains(&extra) {
            out.push(extra);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bindings() {
        let b = Binding::parse("ctrl+shift+t").unwrap();
        assert!(b.ctrl && b.shift && !b.cmd);
        assert_eq!(b.key, "t");
        assert_eq!(Binding::parse("cmd+,").unwrap().key, ",");
        assert!(Binding::parse("ctrl+shift").is_none());
        let plus = Binding::parse("shift+cmd+plus").unwrap();
        assert!(plus.shift && plus.cmd);
        assert_eq!(plus.key, "+");
        // a key name the app never produces could only unbind the action
        assert!(Binding::parse("cmd+foo").is_none());
        assert!(Binding::parse("cmd+f13").is_none());
        assert_eq!(Binding::parse("alt+pagedown").unwrap().key, "pagedown");
        for (action, chord) in defaults() {
            assert!(Binding::parse(chord).is_some(), "{action} = {chord:?}");
        }
    }

    #[test]
    fn unknown_actions_are_ignored() {
        let mut o = BTreeMap::new();
        o.insert("not_an_action".to_string(), "cmd+y".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "y"), None);
    }

    #[test]
    fn defaults_lookup() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        assert_eq!(km.lookup(true, true, false, false, "t"), Some("new_tab"));
        assert_eq!(km.lookup(false, false, false, true, "t"), Some("new_tab"));
        // pane nav moved to alt+cmd (Warp layout); plain alt+left is free
        assert_eq!(
            km.lookup(false, false, true, true, "left"),
            Some("navigate_left")
        );
        assert_eq!(km.lookup(false, false, true, false, "left"), None);
        assert_eq!(km.lookup(false, false, false, false, "t"), None);
    }

    #[test]
    fn warp_layout_defaults() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        assert_eq!(
            km.lookup(false, false, false, true, "k"),
            Some("clear_blocks")
        );
        assert_eq!(km.lookup(false, false, false, true, "p"), Some("palette"));
        assert_eq!(
            km.lookup(false, false, false, true, "1"),
            Some("tab_select_1")
        );
        assert_eq!(km.lookup(false, true, false, true, "}"), Some("tab_next"));
        assert_eq!(
            km.lookup(false, true, false, true, "+"),
            Some("font_increase")
        );
        assert_eq!(
            km.lookup(false, true, false, true, "c"),
            Some("copy_block_command")
        );
        assert_eq!(
            km.lookup(false, true, true, true, "c"),
            Some("copy_block_output")
        );
        assert_eq!(
            km.lookup(false, false, false, false, "pageup"),
            Some("scroll_page_up")
        );
        assert_eq!(
            km.lookup(false, false, false, true, "up"),
            Some("select_block_prev")
        );
        // ctrl+w freed for editor cut-word
        assert_eq!(km.lookup(true, false, false, false, "w"), None);
    }

    #[test]
    fn the_shown_shortcut_is_the_platform_one() {
        let km = Keymap::from_overrides(&BTreeMap::new());
        // new_tab is ctrl+shift+t, with cmd+t as its `_alt`
        let shown = km.preferred("new_tab").unwrap().chord();
        if cfg!(target_os = "macos") {
            assert_eq!(shown, "cmd+t");
        } else {
            assert_eq!(shown, "ctrl+shift+t");
        }
        let rows = km.display_rows();
        let row = rows.iter().find(|(a, _)| a == "new_tab").unwrap();
        assert_eq!(row.1, km.preferred("new_tab").unwrap().chips());
        // an override replaces both
        let mut o = BTreeMap::new();
        o.insert("new_tab".to_string(), "ctrl+n".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.preferred("new_tab").unwrap().chord(), "ctrl+n");
    }

    #[test]
    fn none_unbinds_and_conflicts_are_found() {
        let mut o = BTreeMap::new();
        o.insert("clear_blocks".to_string(), "none".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "k"), None);
        assert!(km.bindings_for("clear_blocks").is_empty());
        let t = Binding::parse("cmd+t").unwrap();
        assert_eq!(km.conflict(&t, "palette"), Some("new_tab"));
        assert_eq!(km.conflict(&t, "new_tab"), None);
        // chords round-trip through the parser
        for s in ["cmd+t", "shift+cmd+plus", "ctrl+alt+shift+cmd+left"] {
            let b = Binding::parse(s).unwrap();
            assert_eq!(Binding::parse(&b.chord()).unwrap(), b);
        }
        let rows = km.display_rows();
        assert!(
            rows.iter()
                .any(|(a, c)| a == "clear_blocks" && c.is_empty())
        );
        assert!(rows.iter().any(|(a, _)| a == "rename_tab"));
    }

    #[test]
    fn override_wins_and_unbinds_default() {
        let mut o = BTreeMap::new();
        o.insert("new_tab".to_string(), "cmd+n".to_string());
        let km = Keymap::from_overrides(&o);
        assert_eq!(km.lookup(false, false, false, true, "n"), Some("new_tab"));
        assert_eq!(km.lookup(true, true, false, false, "t"), None);
        // override on base name also unbinds the _alt sibling
        assert_eq!(km.lookup(false, false, false, true, "t"), None);
    }
}
