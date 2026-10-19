//! Settings text fields (`ClickTarget::SettingsText`): a click edits one,
//! typing fills its draft, ⏎ or a click away saves it, Esc cancels. A value
//! that is not valid is never saved: ⏎ keeps it in the field to fix, a
//! click away drops it; either way the reason shows (a profile's under its
//! form, the others in a toast).

use super::*;
use crate::views::settings::{env_text, join_words, split_words};

/// Longest draft a Settings text field takes.
const MAX_TEXT: usize = 4096;

impl App {
    /// What text field `id` shows now (its setting as text).
    pub(super) fn settings_text(&self, id: &str) -> String {
        let c = &self.config;
        match id {
            "shell" => c.shell.clone().unwrap_or_default(),
            "remote_listen" => c.remote_listen.clone(),
            "global_hotkey" => c.global_hotkey.clone(),
            "hotkey_window_screen" => c.hotkey_window_screen.clone(),
            "word_chars" => c.word_chars.clone(),
            "word_delimiters" => c.word_delimiters.clone(),
            "font_features" => join_words(&c.font_features),
            "subshell_commands" => join_words(&c.subshell_commands),
            "subshell_denylist" => join_words(&c.subshell_denylist),
            _ => {
                let (Some(field), Some(p)) = (id.strip_prefix("profile."), self.form_profile())
                else {
                    return String::new();
                };
                match field {
                    "name" => p.name.clone(),
                    "command" => p.command.clone().unwrap_or_default(),
                    "args" => join_words(&p.args),
                    "cwd" => p.cwd.clone().unwrap_or_default(),
                    "env" => env_text(&p.env),
                    "title" => p.title.clone().unwrap_or_default(),
                    _ => String::new(),
                }
            }
        }
    }

    /// Start editing field `id` (another edit ends first, saved when valid).
    pub(super) fn start_text_edit(&mut self, id: &'static str) {
        self.win.settings.focus = Some(ClickTarget::SettingsText(id));
        if self
            .win
            .settings
            .text_edit
            .as_ref()
            .is_some_and(|(e, _)| *e == id)
        {
            return;
        }
        self.commit_text_edit(false);
        let text = self.settings_text(id);
        self.win.settings.text_edit = Some((id, text));
    }

    /// End the edit: save the draft when it is valid. `keep`: a draft that
    /// is not valid stays in the field (⏎); else it is dropped. Returns
    /// whether it was saved (or changed nothing).
    pub(super) fn commit_text_edit(&mut self, keep: bool) -> bool {
        let Some((id, text)) = self.win.settings.text_edit.take() else {
            return true;
        };
        match self.set_settings_text(id, &text) {
            Ok(changed) => {
                if changed {
                    self.save_config();
                    self.apply_config();
                }
                true
            }
            Err(e) => {
                if id.starts_with("profile.") {
                    self.win.settings.profile_error = Some(e);
                } else {
                    self.toast_error(format!("{} not saved: {e}", text_label(id)));
                }
                if keep {
                    self.win.settings.text_edit = Some((id, text));
                }
                false
            }
        }
    }

    /// Set the setting behind field `id` from `text`: `Ok(true)` when it
    /// changed (the caller saves), an error naming why it is not valid.
    fn set_settings_text(&mut self, id: &str, text: &str) -> Result<bool, String> {
        let t = text.trim();
        if t.chars().any(char::is_control) {
            return Err("it holds a control character".into());
        }
        if let Some(field) = id.strip_prefix("profile.") {
            let Some(i) = self.win.settings.profile_open else {
                return Ok(false);
            };
            return self.set_profile_text(i, field, t);
        }
        let c = &mut self.config;
        let before = c.clone();
        match id {
            "shell" if t.is_empty() => c.shell = None,
            "shell" => {
                // a bare name is looked up on PATH: the setting holds a path
                let path = std::env::var_os("PATH");
                let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                let found = rift_core::profiles::find_program(t, path.as_deref(), home.as_deref())
                    .ok_or_else(|| format!("{t} is not an executable"))?;
                c.shell = Some(if t.starts_with('/') || t.starts_with('~') {
                    t.to_string()
                } else {
                    found
                });
            }
            "remote_listen" => {
                t.parse::<std::net::SocketAddr>()
                    .map_err(|_| format!("{t:?} is not IP:PORT"))?;
                c.remote_listen = t.to_string();
            }
            "global_hotkey" => {
                if !t.is_empty() {
                    crate::hotkey::parse(t)?;
                }
                c.global_hotkey = t.to_string();
            }
            "hotkey_window_screen" => {
                let ok = matches!(t, "" | "pointer" | "primary")
                    || t.parse::<usize>().is_ok_and(|n| n >= 1);
                if !ok {
                    return Err(format!("{t:?} is not pointer, primary or a screen number"));
                }
                c.hotkey_window_screen = if t.is_empty() { "pointer" } else { t }.to_string();
            }
            // blanks can matter here: kept as typed
            "word_chars" => c.word_chars = text.to_string(),
            "word_delimiters" => c.word_delimiters = text.to_string(),
            "font_features" => {
                let words = split_words(t);
                if let Some(bad) = words
                    .iter()
                    .find(|w| rift_core::config::font_feature(w).is_none())
                {
                    return Err(format!(
                        "{bad:?} is not an OpenType feature (ss01, -calt, cv01=2)"
                    ));
                }
                c.font_features = words;
            }
            "subshell_commands" => {
                let words = split_words(t);
                for w in &words {
                    regex::Regex::new(w).map_err(|e| format!("{w:?}: {e}"))?;
                }
                c.subshell_commands = words;
            }
            "subshell_denylist" => c.subshell_denylist = split_words(t),
            _ => return Ok(false),
        }
        Ok(*c != before)
    }

    /// Keys while a text field is edited: text, ⌫ (⌥⌫ / ⌘⌫ clear it), ⏎
    /// (save; a value that is not valid stays), Esc (cancel), ⌘V (paste).
    /// Tab and other chords end the edit (saved when valid) and act as
    /// usual. Returns whether the key was taken.
    pub(super) fn text_edit_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if self.win.settings.text_edit.is_none() {
            return false;
        }
        let mods = self.win.modifiers;
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.win.settings.text_edit = None;
                self.win.settings.profile_error = None;
            }
            Key::Named(NamedKey::Enter) => {
                self.commit_text_edit(true);
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some((_, draft)) = self.win.settings.text_edit.as_mut() {
                    if mods.super_key() || mods.alt_key() {
                        draft.clear();
                    } else {
                        crate::state::pop_grapheme(draft);
                    }
                }
            }
            _ if mods.super_key() || mods.control_key() => {
                let action = input::app_action(event, mods, &self.keymap);
                if action == Some(AppAction::Paste) {
                    self.paste_from_clipboard();
                    return true;
                }
                self.commit_text_edit(false);
                return false;
            }
            Key::Named(NamedKey::Tab) => {
                self.commit_text_edit(false);
                return false;
            }
            _ => {
                if let Some(t) = &event.text
                    && !t.is_empty()
                    && !t.chars().any(char::is_control)
                    && let Some((_, draft)) = self.win.settings.text_edit.as_mut()
                    && draft.len() + t.len() <= MAX_TEXT
                {
                    draft.push_str(t);
                }
            }
        }
        true
    }

    /// A paste into the edited text field (one line: controls dropped).
    /// False when no field is edited.
    pub(super) fn paste_into_text_edit(&mut self, text: &str) -> bool {
        if !self.win.settings.open {
            return false;
        }
        let Some((_, draft)) = self.win.settings.text_edit.as_mut() else {
            return false;
        };
        let clean: String = text.trim().chars().filter(|c| !c.is_control()).collect();
        if draft.len() + clean.len() <= MAX_TEXT {
            draft.push_str(&clean);
        }
        if let Some(w) = &self.win.window {
            w.request_redraw();
        }
        true
    }
}

/// How a field is named in a "not saved" toast.
fn text_label(id: &str) -> &'static str {
    match id {
        "shell" => "Shell path",
        "remote_listen" => "Listen address",
        "global_hotkey" => "Global hotkey",
        "hotkey_window_screen" => "Screen",
        "word_chars" => "Word characters",
        "word_delimiters" => "Word delimiters",
        "font_features" => "Font features",
        "subshell_commands" => "More subshell commands",
        "subshell_denylist" => "Never offer it for",
        _ => "Setting",
    }
}
