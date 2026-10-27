//! Structured configuration editors share Settings' forms and theme tokens.
use super::*;
use crate::state::{CollectionKind as Kind, CollectionOp as Op};

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Ssh(rift_core::ssh::SshProfile),
    Action(rift_core::actions::UserAction),
    Secret(rift_core::secrets::SecretPattern),
    Color(rift_core::tab_color::TabColorRule),
}

impl Entry {
    pub fn name(&self) -> &str {
        match self {
            Self::Ssh(p) => &p.name,
            Self::Action(p) => &p.name,
            Self::Secret(p) => &p.name,
            Self::Color(p) => &p.dir,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Draft {
    pub kind: Kind,
    pub index: Option<usize>,
    pub baseline: Vec<Entry>,
    pub entry: Entry,
    pub step: usize,
    pub sample: String,
    pub error: Option<String>,
    pub deleting: bool,
    pub recording: bool,
}

/// A secret draft's preview: what its pattern alone masks in the sample,
/// both as typed so far (`edit`: the Settings field being typed in).
pub fn preview(d: &Draft, edit: Option<(&str, &str)>) -> Result<String, String> {
    let Entry::Secret(p) = &d.entry else {
        return Ok(String::new());
    };
    let live = |id: &str, saved: &'_ str| -> String {
        edit.filter(|(f, _)| *f == id)
            .map_or(saved, |(_, t)| t)
            .to_string()
    };
    let regex = live("collection.regex", &p.regex).trim().to_string();
    if regex.is_empty() {
        return Ok("Enter a pattern and sample text".into());
    }
    let pattern = rift_core::secrets::SecretPattern {
        name: p.name.clone(),
        regex,
    };
    rift_core::secrets::preview(&pattern, &live("collection.sample", &d.sample))
}

pub fn entries(config: &Config, kind: Kind) -> Vec<Entry> {
    match kind {
        Kind::Ssh => config
            .ssh_profiles
            .iter()
            .cloned()
            .map(Entry::Ssh)
            .collect(),
        Kind::Actions => config.actions.iter().cloned().map(Entry::Action).collect(),
        Kind::Secrets => config
            .secret_patterns
            .iter()
            .cloned()
            .map(Entry::Secret)
            .collect(),
        Kind::Colors => config
            .tab_color_rules
            .iter()
            .cloned()
            .map(Entry::Color)
            .collect(),
    }
}

pub fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Ssh => "SSH profile",
        Kind::Actions => "action",
        Kind::Secrets => "secret pattern",
        Kind::Colors => "tab color rule",
    }
}

fn target(kind: Kind, i: usize, op: Op) -> ClickTarget {
    ClickTarget::Collection(kind, i, op)
}

pub(super) fn page(form: &mut Form, config: &Config, state: &SettingsState, page: Page) {
    let kind = match page {
        Page::Profiles => Kind::Ssh,
        Page::Actions => Kind::Actions,
        Page::Terminal => Kind::Secrets,
        Page::Appearance => Kind::Colors,
        _ => return,
    };
    render(form, config, state, kind);
}

fn buttons(form: &mut Form, kind: Kind, i: usize, items: &[(&str, Op, String)]) {
    choices(form, kind, i, items, None);
}

/// A row of buttons; item `active` (a picked choice) is drawn selected and
/// named so. Save is the primary button of its form.
fn choices(
    form: &mut Form,
    kind: Kind,
    i: usize,
    items: &[(&str, Op, String)],
    active: Option<usize>,
) {
    let mut x = form.x;
    let mut y = form.y + 8.0;
    for (n, (text, op, name)) in items.iter().enumerate() {
        let picked = active == Some(n);
        let width = form.engine.measure(text, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0;
        if x > form.x && x + width > form.right() {
            x = form.x;
            y += CONTROL_H + 8.0;
        }
        form.hidden = false;
        form.row_label = (*text).into();
        form.button(
            x,
            y,
            text,
            if matches!(op, Op::Delete | Op::ConfirmDelete) {
                ButtonKind::Destructive
            } else if picked {
                ButtonKind::Selected
            } else if *op == Op::Save {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            },
            target(kind, i, *op),
            if picked {
                format!("{name}, selected")
            } else {
                name.clone()
            },
        );
        x += width + 8.0;
    }
    form.y = y + CONTROL_H + 16.0;
}

/// A list row with its button at the right, as Settings › Profiles draws its
/// profiles and "New profile".
fn row(form: &mut Form, title: &str, desc: &str, button: (&str, ClickTarget, String)) {
    let (text, target, name) = button;
    let bw = (form.engine.measure(text, 12.5, FONT_SANS, Weight::MEDIUM) + 24.0).round();
    form.reserve = bw + 40.0;
    let y = form.y;
    form.label_desc(title, desc, y);
    if !form.hidden {
        form.button(
            form.right() - bw,
            y + (ROW_H - CONTROL_H) / 2.0,
            text,
            ButtonKind::Secondary,
            target,
            name,
        );
    }
}

/// The "New …" row that adds an entry.
fn add_row(form: &mut Form, kind: Kind) {
    let desc = match kind {
        Kind::Ssh => "A host to connect to from the palette",
        Kind::Actions => "Steps to run from the palette or a shortcut",
        Kind::Secrets => "Masks a value in output, copies and transcripts",
        Kind::Colors => "Colors the tabs open in a directory",
    };
    row(
        form,
        &format!("New {}", label(kind)),
        desc,
        (
            "Add",
            target(kind, 0, Op::Add),
            format!("Add {}", label(kind)),
        ),
    );
}

fn field(form: &mut Form, label: &str, desc: &str, value: &str, id: &'static str) {
    form.text_row(
        form.y,
        &TextRow {
            label,
            desc,
            value,
            placeholder: "",
            id,
            read_only: false,
        },
    );
}

fn note(form: &mut Form, text: &str, error: bool) {
    let lines = wrap_lines(form.engine, text, 12.0, FONT_SANS, form.card_w);
    let y = form.y + 8.0;
    for (i, line) in lines.iter().enumerate() {
        let ly = y + i as f32 * NOTE_LH;
        if form.visible_band(ly, NOTE_LH) {
            form.layer.text(
                sans(form.engine, line, 12.0, Weight::NORMAL),
                form.x,
                ly,
                if error {
                    form.theme.error
                } else {
                    form.theme.text_muted
                },
            );
        }
    }
    let height = lines.len() as f32 * NOTE_LH;
    form.hidden = false;
    form.row_label = text.into();
    form.push_hit([form.x, y, form.card_w, height], ClickTarget::SettingsNoop);
    form.name_last(text.into());
    form.y = y + height + 8.0;
}

pub(super) fn render(form: &mut Form, config: &Config, state: &SettingsState, kind: Kind) {
    if !form.filter.is_empty() {
        let title = match kind {
            Kind::Ssh => "SSH profiles",
            Kind::Actions => "Custom actions",
            Kind::Secrets => "Secret patterns",
            Kind::Colors => "Tab color rules",
        };
        row(
            form,
            title,
            "Add, edit and arrange",
            (
                "Add",
                target(kind, 0, Op::Add),
                format!("Add {}", label(kind)),
            ),
        );
        return;
    }
    form.group(match kind {
        Kind::Ssh => "SSH profiles",
        Kind::Actions => "Your actions",
        Kind::Secrets => "Secret patterns",
        Kind::Colors => "Tab color rules",
    });
    let values = entries(config, kind);
    let draft = state.collection.as_ref().filter(|d| d.kind == kind);
    let stale = draft.is_some_and(|d| d.baseline != values);
    if stale {
        note(
            form,
            "This list changed elsewhere. Your draft is kept; cancel and reopen the entry before saving.",
            true,
        );
        editor(form, draft.unwrap());
    }
    for (i, entry) in values.iter().enumerate() {
        let description = match entry {
            Entry::Ssh(p) => p.host.clone(),
            Entry::Action(p) => format!(
                "{} steps · {}",
                p.steps.len(),
                p.key.as_deref().unwrap_or("No shortcut")
            ),
            Entry::Secret(p) => p.regex.clone(),
            Entry::Color(p) => p.color.clone(),
        };
        row(
            form,
            entry.name(),
            &description,
            (
                "Edit",
                target(kind, i, Op::Edit),
                if values.iter().filter(|e| e.name() == entry.name()).count() > 1 {
                    format!("Edit {} {} (entry {})", label(kind), entry.name(), i + 1)
                } else {
                    format!("Edit {} {}", label(kind), entry.name())
                },
            ),
        );
        if !stale && draft.is_some_and(|d| d.index == Some(i)) {
            editor(form, draft.unwrap());
        }
    }
    if values.is_empty() {
        form.label_desc("None yet", "Add one below", form.y);
    }
    if let Some(d) = draft.filter(|d| !stale && d.index.is_none()) {
        editor(form, d);
    }
    add_row(form, kind);
    if kind == Kind::Secrets && draft.is_some() {
        form.group("Built-in patterns · read-only");
        for (name, expression) in rift_core::secrets::BUILTIN {
            form.label_desc(name, expression, form.y);
        }
    }
    if kind == Kind::Colors {
        note(
            form,
            "The deepest matching directory wins; for equal paths, the last rule wins. A tab's own color takes priority.",
            false,
        );
    }
}

fn editor(form: &mut Form, d: &Draft) {
    let kind = d.kind;
    let i = d.index.unwrap_or(0);
    match &d.entry {
        Entry::Ssh(p) => {
            field(
                form,
                "SSH name",
                "Shown in the palette",
                &p.name,
                "collection.name",
            );
            field(
                form,
                "SSH host",
                "user@host or an alias from ~/.ssh/config",
                &p.host,
                "collection.host",
            );
            field(
                form,
                "SSH arguments",
                "Separate by spaces; quote an argument containing spaces",
                &join_words(&p.args),
                "collection.args",
            );
        }
        Entry::Action(p) => {
            field(
                form,
                "Action name",
                "Shown in the palette",
                &p.name,
                "collection.name",
            );
            field(
                form,
                "Action shortcut",
                "Optional; conflicts are reported before saving",
                p.key.as_deref().unwrap_or(""),
                "collection.key",
            );
            buttons(
                form,
                kind,
                i,
                &[(
                    if d.recording {
                        "Press shortcut…"
                    } else {
                        "Record shortcut"
                    },
                    Op::RecordKey,
                    "Record action shortcut".into(),
                )],
            );
            for (s, step) in p.steps.iter().enumerate() {
                let (ty, value): (&str, &str) = if let Some(t) = &step.action {
                    ("Run action", t)
                } else if let Some(t) = &step.send {
                    ("Send text", t)
                } else {
                    (
                        "Open launch configuration",
                        step.open.as_deref().unwrap_or(""),
                    )
                };
                // the step the fields below edit says so (Save stays the one
                // primary button)
                let editing = s == d.step;
                row(
                    form,
                    &format!("Step {} · {ty}", s + 1),
                    &value.replace('\n', " ↵ "),
                    (
                        if editing { "Editing" } else { "Edit step" },
                        target(kind, i, Op::SelectStep(s)),
                        if editing {
                            format!("Editing action step {}", s + 1)
                        } else {
                            format!("Edit action step {}", s + 1)
                        },
                    ),
                );
            }
            if let Some(step) = p.steps.get(d.step) {
                // the step the fields below edit, and its kind (Settings'
                // segmented control)
                let options = ["Run action", "Send text", "Open layout"];
                let total = form.segmented_width(&options);
                form.reserve = total + 40.0;
                let y = form.y;
                form.label_desc(
                    "Step type",
                    &format!("Step {} of {}", d.step + 1, p.steps.len()),
                    y,
                );
                if !form.hidden {
                    let active = if step.action.is_some() {
                        0
                    } else if step.send.is_some() {
                        1
                    } else {
                        2
                    };
                    let targets: Vec<_> =
                        (0..3).map(|n| target(kind, i, Op::StepType(n))).collect();
                    form.segmented(
                        form.right() - total,
                        y + (ROW_H - CONTROL_H) / 2.0,
                        "Step type",
                        &options,
                        active,
                        &targets,
                    );
                }
                let value = step
                    .action
                    .as_deref()
                    .or(step.send.as_deref())
                    .or(step.open.as_deref())
                    .unwrap_or("");
                if step.send.is_some() {
                    multiline(
                        form,
                        "Send text",
                        "Shift+Enter adds a line; saving never runs it",
                        value,
                        "collection.send",
                    );
                } else {
                    field(
                        form,
                        if step.action.is_some() {
                            "Run action"
                        } else {
                            "Launch configuration"
                        },
                        "Use its existing name",
                        value,
                        "collection.step",
                    );
                }
                buttons(
                    form,
                    kind,
                    i,
                    &[
                        ("Move step up", Op::StepUp, "Move action step up".into()),
                        (
                            "Move step down",
                            Op::StepDown,
                            "Move action step down".into(),
                        ),
                        ("Remove step", Op::RemoveStep, "Remove action step".into()),
                    ],
                );
            }
            buttons(
                form,
                kind,
                i,
                &[("Add step", Op::AddStep, "Add action step".into())],
            );
        }
        Entry::Secret(p) => {
            field(
                form,
                "Pattern name",
                "Name this masking rule",
                &p.name,
                "collection.name",
            );
            field(
                form,
                "Regular expression",
                "The first capture group is masked, or the whole match",
                &p.regex,
                "collection.regex",
            );
            multiline(
                form,
                "Sample text",
                "Preview only; never saved",
                &d.sample,
                "collection.sample",
            );
            match preview(d, form.edit) {
                Ok(text) => note(form, &format!("Preview: {text}"), false),
                Err(e) => note(form, &e, true),
            }
        }
        Entry::Color(p) => {
            field(
                form,
                "Rule directory",
                "Includes subdirectories; ~ expands to your home",
                &p.dir,
                "collection.dir",
            );
            field(
                form,
                "Rule color",
                "A named color, #rgb or #rrggbb",
                &p.color,
                "collection.color",
            );
            let choices: Vec<_> = rift_core::tab_color::NAMES
                .iter()
                .enumerate()
                .map(|(j, n)| (*n, Op::Color(j as u8), format!("Pick rule color {n}")))
                .collect();
            let picked = rift_core::tab_color::NAMES
                .iter()
                .position(|n| n.eq_ignore_ascii_case(p.color.trim()));
            self::choices(form, kind, i, &choices, picked);
            let color = rift_core::tab_color::TabColor::parse(&p.color)
                .map(|c| c.resolve(form.theme))
                .unwrap_or(form.theme.accent);
            let rgb = [color.r, color.g, color.b];
            for ((label, id), value) in [
                ("Red", "collection.red"),
                ("Green", "collection.green"),
                ("Blue", "collection.blue"),
            ]
            .into_iter()
            .zip(rgb)
            {
                form.slider(
                    form.y,
                    label,
                    value,
                    &format!("{}", (value * 255.0).round() as u8),
                    id,
                );
            }
            let y = form.y;
            form.label("Color preview", y);
            if form.visible_band(y, ROW_H) {
                form.layer.bordered(
                    [form.right() - 80.0, y + 12.0],
                    [80.0, CONTROL_H],
                    color,
                    lift(form.theme, 0.12),
                    4.0,
                    1.0,
                );
            }
        }
    }
    if let Some(e) = &d.error {
        note(form, &format!("Not saved: {e}"), true);
    }
    if d.deleting {
        note(form, "Delete this entry from config.toml?", true);
        buttons(
            form,
            kind,
            i,
            &[
                (
                    "Delete",
                    Op::ConfirmDelete,
                    format!("Confirm delete {}", label(kind)),
                ),
                ("Cancel", Op::Cancel, format!("Cancel {} edit", label(kind))),
            ],
        );
    } else {
        let mut actions = vec![
            ("Save", Op::Save, format!("Save {}", label(kind))),
            ("Cancel", Op::Cancel, format!("Cancel {} edit", label(kind))),
        ];
        if d.index.is_some() {
            actions.extend([
                ("Move up", Op::Up, format!("Move {} up", label(kind))),
                ("Move down", Op::Down, format!("Move {} down", label(kind))),
                ("Delete", Op::Delete, format!("Delete {}", label(kind))),
            ]);
            if kind == Kind::Ssh {
                actions.push((
                    "Connect",
                    Op::Connect,
                    format!("Connect SSH profile {}", d.entry.name()),
                ));
            }
        }
        buttons(form, kind, i, &actions);
    }
}

/// Multiline text uses the same field chrome, with one editable row per line.
/// The native Settings editor appends at the caret (end), like its single-line fields.
fn multiline(form: &mut Form, label: &str, description: &str, value: &str, id: &'static str) {
    form.label_desc(label, description, form.y);
    if form.hidden {
        return;
    }
    let editing = form
        .edit
        .filter(|(field, _)| *field == id)
        .map(|(_, text)| text);
    let shown = editing.unwrap_or(value);
    let lines: Vec<_> = shown.split('\n').collect();
    let n = shown.split('\n').count().clamp(3, 8);
    let h = n as f32 * NOTE_LH + 16.0;
    let r = [form.x, form.y, form.card_w, h];
    if form.visible_band(r[1], h) {
        form.layer.bordered(
            [r[0], r[1]],
            [r[2], h],
            lift(form.theme, 0.03),
            if editing.is_some() {
                form.theme.accent
            } else {
                lift(form.theme, 0.12)
            },
            4.0,
            1.0,
        );
        for (j, line) in lines
            .iter()
            .rev()
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .enumerate()
        {
            let text =
                super::super::palette::elide_to(form.engine, line, 13.0, FONT_SANS, r[2] - 20.0);
            form.layer.text(
                sans(form.engine, &text, 13.0, Weight::NORMAL),
                r[0] + 10.0,
                r[1] + 8.0 + j as f32 * NOTE_LH,
                form.theme.text_bright,
            );
        }
        if editing.is_some() && form.blink_on {
            let last = shown.rsplit('\n').next().unwrap_or("");
            let x = (form.engine.measure(last, 13.0, FONT_SANS, Weight::NORMAL) + r[0] + 10.0)
                .min(r[0] + r[2] - 10.0);
            let row = shown.split('\n').count().min(n).saturating_sub(1);
            form.layer.rect(
                [x, r[1] + 8.0 + row as f32 * NOTE_LH],
                [1.0, NOTE_LH],
                form.theme.text_bright,
                0.0,
            );
        }
    }
    form.push_hit(r, ClickTarget::SettingsText(id));
    form.name_last(label.into());
    form.y += h + 8.0;
}
