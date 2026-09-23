/// Keybinding parsing + lookup. Defaults per spec §4; `[keybindings]`
/// overrides from config.toml win last.
use std::collections::BTreeMap;

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
                    b.key = if p == "plus" { "+".into() } else { p };
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
        ("workflows", "ctrl+shift+r"),
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
        for (action, binding) in overrides {
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

    /// For the settings Keybindings page: every bindable action with the
    /// chips of its first binding (empty when unbound), sorted by name.
    pub fn display_rows(&self) -> Vec<(String, Vec<String>)> {
        let mut rows: Vec<(String, Vec<String>)> = all_actions()
            .into_iter()
            .map(|a| {
                let chips = self
                    .entries
                    .iter()
                    .find(|(_, name)| name == a)
                    .map(|(b, _)| b.chips())
                    .unwrap_or_default();
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
    for extra in ["rename_tab", "close_other_tabs", "save_launch_config"] {
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
