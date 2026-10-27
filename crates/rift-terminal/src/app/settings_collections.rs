//! Draft-based editors for structured config: save first, then apply everywhere.
use super::*;
use crate::state::{CollectionKind as Kind, CollectionOp as Op};
use crate::views::settings::collections::{self, Draft, Entry};

fn collection_page(kind: Kind) -> settings::Page {
    match kind {
        Kind::Ssh => settings::Page::Profiles,
        Kind::Actions => settings::Page::Actions,
        Kind::Secrets => settings::Page::Terminal,
        Kind::Colors => settings::Page::Appearance,
    }
}

fn put(config: &mut Config, kind: Kind, values: Vec<Entry>) {
    match kind {
        Kind::Ssh => {
            config.ssh_profiles = values
                .into_iter()
                .filter_map(|e| if let Entry::Ssh(p) = e { Some(p) } else { None })
                .collect()
        }
        Kind::Actions => {
            config.actions = values
                .into_iter()
                .filter_map(|e| {
                    if let Entry::Action(p) = e {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect()
        }
        Kind::Secrets => {
            config.secret_patterns = values
                .into_iter()
                .filter_map(|e| {
                    if let Entry::Secret(p) = e {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect()
        }
        Kind::Colors => {
            config.tab_color_rules = values
                .into_iter()
                .filter_map(|e| {
                    if let Entry::Color(p) = e {
                        Some(p)
                    } else {
                        None
                    }
                })
                .collect()
        }
    }
}

fn validate(config: &Config, kind: Kind) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for e in collections::entries(config, kind) {
        if e.name().trim().is_empty() {
            return Err("A name or directory is required".into());
        }
        if e.name().chars().any(char::is_control) {
            return Err("Names cannot contain control characters".into());
        }
        if kind != Kind::Colors && !names.insert(e.name().trim().to_string()) {
            return Err("That name is already used".into());
        }
        match e {
            Entry::Ssh(p)
                if p.host.trim().is_empty()
                    || p.host.starts_with('-')
                    || p.host.chars().any(char::is_whitespace)
                    || p.args.iter().any(|s| s.contains('\0')) =>
            {
                return Err("Enter a host or SSH alias, without spaces or a leading dash".into());
            }
            Entry::Color(p) if rift_core::tab_color::TabColor::parse(&p.color).is_none() => {
                return Err("Choose a named color or enter #rgb or #rrggbb".into());
            }
            _ => {}
        }
    }
    rift_core::actions::validate(&config.actions, &rift_core::keybindings::all_actions())?;
    rift_core::secrets::validate(&config.secret_patterns)?;
    Ok(())
}

impl App {
    pub(super) fn collection_text(&self, field: &str) -> String {
        let Some(d) = &self.win.settings.collection else {
            return String::new();
        };
        if field == "sample" {
            return d.sample.clone();
        }
        match (&d.entry, field) {
            (_, "name") => d.entry.name().into(),
            (Entry::Ssh(p), "host") => p.host.clone(),
            (Entry::Ssh(p), "args") => settings::join_words(&p.args),
            (Entry::Action(p), "key") => p.key.clone().unwrap_or_default(),
            (Entry::Action(p), "step" | "send") => p
                .steps
                .get(d.step)
                .and_then(|s| s.action.as_ref().or(s.send.as_ref()).or(s.open.as_ref()))
                .cloned()
                .unwrap_or_default(),
            (Entry::Secret(p), "regex") => p.regex.clone(),
            (Entry::Color(p), "dir") => p.dir.clone(),
            (Entry::Color(p), "color") => p.color.clone(),
            _ => String::new(),
        }
    }

    pub(super) fn set_collection_text(&mut self, field: &str, text: &str) -> Result<bool, String> {
        let multiline = matches!(field, "send" | "sample");
        if text
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
        {
            return Err("This field contains an unsupported control character".into());
        }
        let t = if multiline { text } else { text.trim() };
        let Some(d) = self.win.settings.collection.as_mut() else {
            return Ok(false);
        };
        if field == "sample" {
            d.sample = t.into();
            return Ok(false);
        }
        match (&mut d.entry, field) {
            (Entry::Ssh(p), "name") => p.name = t.into(),
            (Entry::Ssh(p), "host") => p.host = t.into(),
            (Entry::Ssh(p), "args") => p.args = settings::split_words(t),
            (Entry::Action(p), "name") => p.name = t.into(),
            (Entry::Action(p), "key") => p.key = (!t.is_empty()).then(|| t.into()),
            (Entry::Action(p), "step" | "send") => {
                if let Some(s) = p.steps.get_mut(d.step)
                    && let Some(v) = s.action.as_mut().or(s.send.as_mut()).or(s.open.as_mut())
                {
                    *v = t.into();
                }
            }
            (Entry::Secret(p), "name") => p.name = t.into(),
            (Entry::Secret(p), "regex") => p.regex = t.into(),
            (Entry::Color(p), "dir") => p.dir = t.into(),
            (Entry::Color(p), "color") => p.color = t.into(),
            _ => {}
        }
        d.error = None;
        Ok(false) // Only the draft changed; Save is the sole persistence boundary.
    }

    fn save_collection(&mut self, operation: Op) -> Result<(), String> {
        let d = self
            .win
            .settings
            .collection
            .clone()
            .ok_or("No editor is open")?;
        if self.config_error.is_some() {
            return Err("Fix config.toml before saving".into());
        }
        // Read disk immediately: a watcher may not have delivered its change yet.
        let mut next = Config::try_load()?.unwrap_or_default();
        let mut values = collections::entries(&next, d.kind);
        if values != d.baseline {
            return Err("This list changed elsewhere. Cancel and reopen it before saving".into());
        }
        let mut index = d.index;
        match operation {
            Op::ConfirmDelete => {
                values.remove(index.ok_or("This entry is not saved")?);
                index = None;
            }
            Op::Up | Op::Down => {
                let i = index.ok_or("Save this entry before moving it")?;
                let j = if operation == Op::Up {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(values.len() - 1)
                };
                values[i] = d.entry.clone();
                values.swap(i, j);
                index = Some(j);
            }
            _ => {
                if let Some(i) = index {
                    values[i] = d.entry.clone();
                } else {
                    index = Some(values.len());
                    values.push(d.entry.clone());
                }
            }
        }
        // An action rename preserves callers. A deletion with callers is refused
        // by the normal action validator rather than leaving broken sequences.
        if d.kind == Kind::Actions
            && operation != Op::ConfirmDelete
            && let Some(i) = d.index
        {
            let old = d.baseline[i].name();
            let new = d.entry.name();
            if old != new {
                for e in &mut values {
                    if let Entry::Action(a) = e {
                        for s in &mut a.steps {
                            if s.action.as_deref() == Some(old) {
                                s.action = Some(new.into());
                            }
                        }
                    }
                }
            }
        }
        put(&mut next, d.kind, values);
        validate(&next, d.kind)?;
        if let Entry::Action(a) = &d.entry
            && operation != Op::ConfirmDelete
        {
            let layouts = rift_core::launch::load_all();
            for step in &a.steps {
                if let Some(name) = &step.open
                    && !layouts.iter().any(|l| l.name == *name)
                {
                    return Err(format!("No launch configuration named {name:?}"));
                }
            }
            if let Some(key) = a.key.as_deref()
                && let Some(binding) = rift_core::Binding::parse(key)
            {
                let own = d
                    .index
                    .map(|i| format!("{}{i}", rift_core::actions::KEYMAP_PREFIX))
                    .unwrap_or_default();
                if let Some(other) = self.keymap.conflict(&binding, &own) {
                    return Err(format!(
                        "Shortcut is already used by {other}; choose another or change it in Keybindings"
                    ));
                }
            }
        }
        next.save().map_err(|e| format!("Could not save: {e}"))?;
        self.load_config(false);
        self.apply_config();
        self.ssh_profiles = rift_core::ssh::all_profiles(&self.config.ssh_profiles);
        self.refresh_config_panes();
        let baseline = collections::entries(&self.config, d.kind);
        // the reload may not hold what was written (an unreadable file): no
        // draft then, rather than one pointing past the list
        let saved = index.and_then(|i| baseline.get(i)).cloned();
        if operation == Op::ConfirmDelete {
            self.win.settings.collection = None;
        } else if let Some(entry) = saved {
            self.win.settings.collection = Some(Draft {
                baseline,
                entry,
                index,
                error: None,
                deleting: false,
                ..d
            });
        }
        self.each_window(|app| {
            if app.win.palette.open {
                app.refresh_palette();
            }
            if let Some(w) = &app.win.window {
                w.request_redraw();
            }
        });
        Ok(())
    }

    pub(super) fn collection_op(
        &mut self,
        kind: Kind,
        i: usize,
        op: Op,
        event_loop: &ActiveEventLoop,
    ) {
        if !self.commit_text_edit(true) {
            return;
        }
        if matches!(op, Op::Add | Op::Edit) {
            if self.win.settings.collection.as_ref().is_some_and(|d| {
                d.index
                    .map(|j| d.baseline.get(j) != Some(&d.entry))
                    .unwrap_or(true)
            }) {
                if let Some(d) = &mut self.win.settings.collection {
                    d.error = Some("Save or cancel this edit first".into());
                    self.win.settings.page = collection_page(d.kind);
                    self.win.settings.scroll = 0.0;
                    self.win.settings.query.clear();
                }
                return;
            }
            let baseline = collections::entries(&self.config, kind);
            let entry = if op == Op::Edit {
                let Some(e) = baseline.get(i) else { return };
                e.clone()
            } else {
                let base = format!("New {}", collections::label(kind));
                let name = (1..)
                    .map(|n| {
                        if n == 1 {
                            base.clone()
                        } else {
                            format!("{base} {n}")
                        }
                    })
                    .find(|n| !baseline.iter().any(|e| e.name() == n))
                    .unwrap();
                match kind {
                    Kind::Ssh => Entry::Ssh(rift_core::ssh::SshProfile {
                        name,
                        host: String::new(),
                        args: vec![],
                    }),
                    Kind::Actions => Entry::Action(rift_core::actions::UserAction {
                        name,
                        key: None,
                        steps: vec![rift_core::actions::StepSpec {
                            send: Some(String::new()),
                            ..Default::default()
                        }],
                    }),
                    Kind::Secrets => Entry::Secret(rift_core::secrets::SecretPattern {
                        name,
                        regex: String::new(),
                    }),
                    Kind::Colors => Entry::Color(rift_core::tab_color::TabColorRule {
                        dir: String::new(),
                        color: "blue".into(),
                    }),
                }
            };
            self.win.settings.collection = Some(Draft {
                kind,
                index: (op == Op::Edit).then_some(i),
                baseline,
                entry,
                step: 0,
                sample: String::new(),
                error: None,
                deleting: false,
                recording: false,
            });
            self.win.settings.page = collection_page(kind);
            self.win.settings.query.clear();
            return;
        }
        if op == Op::Cancel {
            if let Some(d) = &mut self.win.settings.collection
                && d.deleting
            {
                d.deleting = false;
                return;
            }
            self.win.settings.collection = None;
            return;
        }
        if matches!(
            op,
            Op::Save | Op::ConfirmDelete | Op::Up | Op::Down | Op::Connect
        ) {
            if let Err(e) = self.save_collection(op) {
                if let Some(d) = &mut self.win.settings.collection {
                    d.error = Some(e);
                }
                return;
            }
            if op == Op::Connect {
                let name = self
                    .win
                    .settings
                    .collection
                    .as_ref()
                    .map(|d| d.entry.name().to_string())
                    .unwrap_or_default();
                self.ssh_profiles = rift_core::ssh::all_profiles(&self.config.ssh_profiles);
                if let Some(j) = self.ssh_profiles.iter().position(|p| p.name == name) {
                    self.close_settings_tab();
                    self.run_palette_action(PaletteAction::Ssh(j), event_loop);
                }
            }
            return;
        }
        let Some(d) = self
            .win
            .settings
            .collection
            .as_mut()
            .filter(|d| d.kind == kind)
        else {
            return;
        };
        d.error = None;
        match op {
            Op::Delete => d.deleting = true,
            Op::RecordKey => d.recording = true,
            Op::Color(n) => {
                if let Entry::Color(p) = &mut d.entry
                    && let Some(n) = rift_core::tab_color::NAMES.get(n as usize)
                {
                    p.color = (*n).into();
                }
            }
            _ => {
                if let Entry::Action(p) = &mut d.entry {
                    match op {
                        Op::AddStep if p.steps.len() < rift_core::actions::MAX_STEPS => {
                            p.steps.push(rift_core::actions::StepSpec {
                                send: Some(String::new()),
                                ..Default::default()
                            });
                            d.step = p.steps.len() - 1;
                        }
                        Op::AddStep => {
                            d.error = Some(format!(
                                "An action can have at most {} steps",
                                rift_core::actions::MAX_STEPS
                            ));
                        }
                        Op::SelectStep(s) => d.step = s.min(p.steps.len().saturating_sub(1)),
                        Op::RemoveStep if d.step < p.steps.len() => {
                            p.steps.remove(d.step);
                            d.step = d.step.min(p.steps.len().saturating_sub(1));
                        }
                        Op::StepUp if d.step > 0 => {
                            p.steps.swap(d.step, d.step - 1);
                            d.step -= 1;
                        }
                        Op::StepDown if d.step + 1 < p.steps.len() => {
                            p.steps.swap(d.step, d.step + 1);
                            d.step += 1;
                        }
                        Op::StepType(n) => {
                            if let Some(s) = p.steps.get_mut(d.step) {
                                let value = s
                                    .action
                                    .take()
                                    .or(s.send.take())
                                    .or(s.open.take())
                                    .unwrap_or_default();
                                *s = Default::default();
                                match n {
                                    0 => s.action = Some(value),
                                    1 => s.send = Some(value),
                                    _ => s.open = Some(value),
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub(super) fn collection_record_key(&mut self, event: &input::KeyPress) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if !self
            .win
            .settings
            .collection
            .as_ref()
            .is_some_and(|d| d.recording)
        {
            return false;
        }
        let d = self.win.settings.collection.as_mut().unwrap();
        if event.logical_key == Key::Named(NamedKey::Escape) {
            d.recording = false;
            return true;
        }
        let m = self.win.modifiers;
        let key = match (&event.unmodified, &event.logical_key) {
            (Some(Key::Character(s)), Key::Character(_)) if m.alt_key() => Some(s.to_lowercase()),
            _ => input::key_string(event),
        };
        let Some(key) = key else { return true };
        let function = key.len() > 1 && !matches!(key.as_str(), "space" | "enter" | "tab");
        if !m.control_key() && !m.super_key() && !m.alt_key() && !function {
            d.error = Some("Use a modifier or function key".into());
            return true;
        }
        let b = rift_core::Binding {
            ctrl: m.control_key(),
            shift: m.shift_key(),
            alt: m.alt_key(),
            cmd: m.super_key(),
            key,
        };
        if let Entry::Action(a) = &mut d.entry {
            a.key = Some(b.chord());
        }
        d.recording = false;
        true
    }

    pub(super) fn collection_color_value(&self, id: &str) -> f32 {
        let Some(Draft {
            entry: Entry::Color(p),
            ..
        }) = &self.win.settings.collection
        else {
            return 0.0;
        };
        let c = rift_core::tab_color::TabColor::parse(&p.color)
            .map(|c| c.resolve(&self.theme))
            .unwrap_or(self.theme.accent);
        match id {
            "collection.red" => c.r,
            "collection.green" => c.g,
            _ => c.b,
        }
    }
    pub(super) fn collection_color_slider(&mut self, id: &str, t: f32) {
        let Some(Draft {
            entry: Entry::Color(p),
            ..
        }) = &mut self.win.settings.collection
        else {
            return;
        };
        let mut c = rift_core::tab_color::TabColor::parse(&p.color)
            .map(|c| c.resolve(&self.theme))
            .unwrap_or(self.theme.accent);
        match id {
            "collection.red" => c.r = t,
            "collection.green" => c.g = t,
            _ => c.b = t,
        }
        p.color = c.to_hex();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_validation_rejects_invalid_entries_without_changing_other_lists() {
        let mut c = Config {
            ssh_profiles: vec![rift_core::ssh::SshProfile {
                name: "server".into(),
                host: "-option".into(),
                args: vec![],
            }],
            ..Default::default()
        };
        assert!(validate(&c, Kind::Ssh).is_err());
        c.ssh_profiles[0].host = "user@fixture".into();
        assert!(validate(&c, Kind::Ssh).is_ok());
        c.ssh_profiles.push(c.ssh_profiles[0].clone());
        assert!(validate(&c, Kind::Ssh).is_err());
        c.tab_color_rules = vec![rift_core::tab_color::TabColorRule {
            dir: "~/work".into(),
            color: "invalid".into(),
        }];
        assert!(validate(&c, Kind::Colors).is_err());
        c.tab_color_rules[0].color = "#123abc".into();
        c.tab_color_rules.push(c.tab_color_rules[0].clone());
        assert!(
            validate(&c, Kind::Colors).is_ok(),
            "equal directories are intentional; last wins"
        );
        c.secret_patterns = vec![rift_core::secrets::SecretPattern {
            name: "fixture".into(),
            regex: "[".into(),
        }];
        assert!(validate(&c, Kind::Secrets).is_err());
    }
    #[test]
    fn preview_masks_first_capture_and_keeps_sample_out_of_config() {
        let d = Draft {
            kind: Kind::Secrets,
            index: None,
            baseline: vec![],
            entry: Entry::Secret(rift_core::secrets::SecretPattern {
                name: "fixture".into(),
                regex: "fixture=(MASKME)".into(),
            }),
            step: 0,
            sample: "fixture=MASKME untouched".into(),
            error: None,
            deleting: false,
            recording: false,
        };
        assert_eq!(
            collections::preview(&d, None).as_deref(),
            Ok("fixture=•••••• untouched")
        );
        let mut c = Config::default();
        put(&mut c, Kind::Secrets, vec![d.entry]);
        let file = toml::to_string(&c).unwrap();
        assert!(!file.contains("untouched"));
    }
}
