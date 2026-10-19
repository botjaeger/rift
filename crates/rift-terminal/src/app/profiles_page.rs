//! Settings › Profiles in the app: the actions on config.toml's
//! `[[profiles]]` (add, edit, duplicate, hide, delete, set default, move).
//! Every change is checked with `profiles::validate` and written
//! comment-preserving (`save_config`); one that is not valid is never
//! written and says why under the open form.

use super::*;
use crate::state::ProfileOp;
use crate::views::settings::{PROFILE_DRAG_SLOP, ProfileDrag, split_words};
use rift_core::profiles::Profile;

impl App {
    /// The profile whose form is open in Settings › Profiles.
    pub(super) fn form_profile(&self) -> Option<&Profile> {
        self.win
            .settings
            .profile_open
            .and_then(|i| self.config.profiles.get(i))
    }

    /// config.toml did not load: the list shown is not the file's, so no
    /// change is made to it (the refusal toast says why). True: refused.
    fn profiles_locked(&mut self) -> bool {
        if self.config_error.is_none() {
            return false;
        }
        // (refuses, with the "fix config.toml first" toast)
        self.save_config();
        true
    }

    /// Change profile `i` with `f`; saved when the result is valid (else
    /// the error, nothing written). `Ok(false)`: nothing changed.
    pub(super) fn edit_profile(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut Profile),
    ) -> Result<bool, String> {
        if self.config_error.is_some() {
            return Err("Not saved: config.toml has an error; fix it first".into());
        }
        let Some(old) = self.config.profiles.get(i).cloned() else {
            return Err("That profile is no longer in config.toml".into());
        };
        let mut profiles = self.config.profiles.clone();
        f(&mut profiles[i]);
        if profiles[i] == old {
            return Ok(false);
        }
        rift_core::profiles::validate(&profiles)?;
        // the default profile keeps being the default under its new name
        let renamed = profiles[i].name.trim() != old.name.trim();
        if renamed && self.default_profile_name() == Some(old.name.trim().to_string()) {
            self.config.default_profile = Some(profiles[i].name.trim().to_string());
        }
        let before = std::mem::replace(&mut self.config.profiles, profiles);
        self.follow_profiles(&before, false);
        self.win.settings.profile_error = None;
        self.save_config();
        Ok(true)
    }

    /// Set field `field` of profile `i` from its text field (`Ok(true)`:
    /// saved already, so the caller writes nothing more).
    pub(super) fn set_profile_text(
        &mut self,
        i: usize,
        field: &str,
        t: &str,
    ) -> Result<bool, String> {
        let some = |t: &str| (!t.is_empty()).then(|| t.to_string());
        let env = if field == "env" {
            let mut env = std::collections::BTreeMap::new();
            for w in split_words(t) {
                let (k, v) = w
                    .split_once('=')
                    .ok_or_else(|| format!("{w:?} is not NAME=value"))?;
                env.insert(k.to_string(), v.to_string());
            }
            Some(env)
        } else {
            None
        };
        let detected = self
            .config
            .profiles
            .get(i)
            .is_some_and(Profile::is_generated);
        self.edit_profile(i, |p| match field {
            "name" => p.name = t.to_string(),
            // a detected profile's command stays the shell's (Duplicate
            // to change it)
            "command" if detected => {}
            // empty stays empty (the Shell path); clearing one is an error
            "command" if t.is_empty() && p.command.is_none() => {}
            "command" => p.command = Some(t.to_string()),
            "args" => p.args = split_words(t),
            "cwd" => p.cwd = some(t),
            "env" => p.env = env.unwrap_or_default(),
            "title" => p.title = some(t),
            _ => {}
        })?;
        // saved (or unchanged) here: nothing for the caller to write
        Ok(false)
    }

    /// `default_profile` as a trimmed name, when set.
    fn default_profile_name(&self) -> Option<String> {
        self.config
            .default_profile
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
    }

    /// A name no profile has: `base`, else `base 2`, `base 3`, …
    fn free_profile_name(&self, base: &str) -> String {
        let taken = |n: &str| self.config.profiles.iter().any(|p| p.name.trim() == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|k| format!("{base} {k}"))
            .find(|n| !taken(n))
            .expect("a free name")
    }

    /// Settings › Profiles' Add: "New profile" running the login shell,
    /// its form open.
    pub(super) fn add_profile(&mut self) {
        self.commit_text_edit(false);
        if self.profiles_locked() {
            return;
        }
        let shell = rift_core::shell_integration::resolve_shell(None);
        let p = Profile {
            command: Some(shell),
            ..Profile::named(&self.free_profile_name("New profile"))
        };
        let before = self.config.profiles.clone();
        self.config.profiles.push(p);
        self.follow_profiles(&before, false);
        let i = self.config.profiles.len() - 1;
        self.win.settings.profile_open = Some(i);
        self.win.settings.profile_error = None;
        self.win.settings.focus = Some(ClickTarget::ProfileRow(i));
        self.save_config();
    }

    /// A button of profile `i`'s row or form.
    pub(super) fn profile_op(&mut self, i: usize, op: ProfileOp) {
        self.commit_text_edit(false);
        let Some(p) = self.config.profiles.get(i).cloned() else {
            return;
        };
        if op != ProfileOp::Edit && self.profiles_locked() {
            return;
        }
        match op {
            ProfileOp::Edit => {
                let s = &mut self.win.settings;
                s.profile_open = if s.profile_open == Some(i) {
                    None
                } else {
                    Some(i)
                };
                s.profile_error = None;
                // ⌥↑ / ⌥↓ then move it
                s.focus = Some(ClickTarget::Profile(i, ProfileOp::Edit));
            }
            ProfileOp::SetDefault => {
                self.config.default_profile = Some(p.name.trim().to_string());
                self.save_config();
            }
            ProfileOp::Duplicate => {
                let copy = Profile {
                    name: self.free_profile_name(&format!("{} copy", p.name.trim())),
                    // the copy is the user's own: its command can change
                    source: None,
                    orphaned: false,
                    ..p
                };
                let before = self.config.profiles.clone();
                self.config.profiles.insert(i + 1, copy);
                self.follow_profiles(&before, false);
                self.win.settings.profile_open = Some(i + 1);
                self.win.settings.profile_error = None;
                self.win.settings.focus = Some(ClickTarget::ProfileRow(i + 1));
                self.save_config();
            }
            ProfileOp::ToggleHidden => {
                if let Err(e) = self.edit_profile(i, |p| p.hidden = !p.hidden) {
                    self.win.settings.profile_error = Some(e);
                }
            }
            ProfileOp::Delete => {
                self.win.confirm = Some(ConfirmClose {
                    opened: Instant::now(),
                    focus_cancel: false,
                    action: ConfirmAction::DeleteProfile {
                        index: i,
                        name: p.name.clone(),
                        detected: p.is_generated(),
                        default: self.default_profile_name().as_deref() == Some(p.name.trim()),
                    },
                    commands: vec![crate::views::settings::profile_command(&p)],
                });
            }
        }
    }

    /// Delete profile `i` (confirmed), when it is still `name`. A detected
    /// one's shell goes on the seen list, so it never comes back; the
    /// default one leaves no default.
    pub(super) fn delete_profile(&mut self, i: usize, name: &str) {
        let Some(i) = Some(i)
            .filter(|&i| self.config.profiles.get(i).is_some_and(|p| p.name == name))
            .or_else(|| self.config.profiles.iter().position(|p| p.name == name))
        else {
            return;
        };
        let before = self.config.profiles.clone();
        let p = self.config.profiles.remove(i);
        self.follow_profiles(&before, false);
        if p.is_generated()
            && let Some(command) = p.command.as_deref()
            && let Some(path) = rift_core::shell_integration::data_dir()
                .map(|d| d.join(rift_core::generated::SEEN_FILE))
        {
            let saved = rift_core::generated::load_seen(&path).and_then(|mut seen| {
                if seen.insert(command.to_string()) {
                    rift_core::generated::save_seen(&path, &seen)
                } else {
                    Ok(())
                }
            });
            if let Err(e) = saved {
                log::warn!("generated profiles list: {e}");
            }
        }
        if self.default_profile_name().as_deref() == Some(p.name.trim()) {
            self.config.default_profile = None;
        }
        let s = &mut self.win.settings;
        s.profile_open = match s.profile_open {
            Some(o) if o == i => None,
            Some(o) if o > i => Some(o - 1),
            o => o,
        };
        s.profile_error = None;
        s.focus = None;
        self.save_config();
        self.toast_done(format!("Deleted {}", p.name));
    }

    /// Move profile `from` to index `to` (of the list without it), the
    /// open form and the focus with it.
    pub(super) fn move_profile(&mut self, from: usize, to: usize) {
        let n = self.config.profiles.len();
        if from >= n || to >= n || from == to || self.profiles_locked() {
            return;
        }
        self.commit_text_edit(false);
        let before = self.config.profiles.clone();
        let p = self.config.profiles.remove(from);
        self.config.profiles.insert(to, p);
        self.follow_profiles(&before, false);
        // where index `k` went
        let moved = |k: usize| {
            if k == from {
                to
            } else if from < k && k <= to {
                k - 1
            } else if to <= k && k < from {
                k + 1
            } else {
                k
            }
        };
        let s = &mut self.win.settings;
        s.profile_open = s.profile_open.map(moved);
        if let Some(ClickTarget::ProfileRow(k)) = s.focus {
            s.focus = Some(ClickTarget::ProfileRow(moved(k)));
        }
        if let Some(ClickTarget::Profile(k, op)) = s.focus {
            s.focus = Some(ClickTarget::Profile(moved(k), op));
        }
        self.save_config();
    }

    /// ⌥↑ / ⌥↓ on a focused profile row (or its Edit button): move it one
    /// place. False when the focus is not on a profile.
    pub(super) fn move_focused_profile(&mut self, up: bool) -> bool {
        let i = match self.win.settings.focus {
            Some(ClickTarget::ProfileRow(i) | ClickTarget::Profile(i, ProfileOp::Edit)) => i,
            _ => return false,
        };
        let to = if up {
            i.checked_sub(1)
        } else {
            Some(i + 1).filter(|&t| t < self.config.profiles.len())
        };
        if let Some(to) = to {
            self.move_profile(i, to);
        }
        true
    }

    /// The profiles list changed from `before` (another window's edit, a
    /// reload): each window's open form, focus and draft follow their
    /// profile to its new index (by name; a profile renamed in place keeps
    /// its index), or close when it is gone. `here`: the window being
    /// served too (its own edits already moved them).
    pub(super) fn follow_profiles(&mut self, before: &[Profile], here: bool) {
        let after: Vec<String> = self
            .config
            .profiles
            .iter()
            .map(|p| p.name.trim().to_string())
            .collect();
        let old: Vec<String> = before.iter().map(|p| p.name.trim().to_string()).collect();
        if old == after {
            return;
        }
        let map = |k: usize| -> Option<usize> {
            let name = old.get(k)?;
            after.iter().position(|n| n == name).or_else(|| {
                // renamed where it stood
                (old.len() == after.len() && !old.contains(&after[k])).then_some(k)
            })
        };
        let fix = |s: &mut crate::views::settings::SettingsState| {
            let open = s.profile_open.map(map);
            if let Some(o) = open {
                s.profile_open = o;
                if o.is_none() {
                    // its draft belonged to the form that closed
                    if s.text_edit
                        .as_ref()
                        .is_some_and(|(id, _)| id.starts_with("profile."))
                    {
                        s.text_edit = None;
                    }
                    s.profile_error = None;
                }
            }
            s.focus = match s.focus.take() {
                Some(ClickTarget::ProfileRow(k)) => map(k).map(ClickTarget::ProfileRow),
                Some(ClickTarget::Profile(k, op)) => map(k).map(|k| ClickTarget::Profile(k, op)),
                f => f,
            };
            s.profile_drag = None;
        };
        if here {
            fix(&mut self.win.settings);
        }
        for w in &mut self.parked {
            fix(&mut w.settings);
            if let Some(win) = &w.window {
                win.request_redraw();
            }
        }
    }

    /// A press on profile row `i`: it may become a drag.
    pub(super) fn press_profile_row(&mut self, i: usize) {
        self.win.settings.focus = Some(ClickTarget::ProfileRow(i));
        self.win.settings.profile_drag = Some(ProfileDrag {
            from: i,
            press_y: self.win.mouse.1,
            live: false,
            to: i,
        });
    }

    /// The pointer moved while a profile row is held: past the slop it
    /// drags; its landing place is the first row whose middle is below the
    /// pointer (none: the end).
    pub(super) fn drag_profile_row(&mut self) -> bool {
        let Some(mut d) = self.win.settings.profile_drag else {
            return false;
        };
        let y = self.win.mouse.1;
        if !d.live && (y - d.press_y).abs() < PROFILE_DRAG_SLOP {
            return false;
        }
        d.live = true;
        let mut rows: Vec<(usize, f32)> = self
            .win
            .hits
            .iter()
            .filter_map(|h| match h.target {
                ClickTarget::ProfileRow(j) => Some((j, h.rect[1] + h.rect[3] / 2.0)),
                _ => None,
            })
            .collect();
        rows.sort_by_key(|(j, _)| *j);
        d.to = rows
            .iter()
            .find(|(_, mid)| *mid > y)
            .map_or(self.config.profiles.len(), |(j, _)| *j);
        self.win.settings.profile_drag = Some(d);
        true
    }

    /// The button let go of a held profile row: a drag drops it where it
    /// points; a click without one opens or closes its form. False when
    /// no row was held.
    pub(super) fn release_profile_row(&mut self) -> bool {
        let Some(d) = self.win.settings.profile_drag.take() else {
            return false;
        };
        if d.live {
            // `to` counts the row itself: past it, one fewer
            let to = if d.to > d.from { d.to - 1 } else { d.to };
            self.move_profile(d.from, to);
        } else {
            self.profile_op(d.from, ProfileOp::Edit);
            self.win.settings.focus = Some(ClickTarget::ProfileRow(d.from));
        }
        true
    }
}
