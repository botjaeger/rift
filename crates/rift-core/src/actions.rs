//! User actions (`[[actions]]`): a named, ordered list of steps that runs
//! from the palette or a chord (Windows Terminal `sendInput` /
//! `multipleActions`). A step runs an existing action, types text into the
//! pane the sequence targets, or opens a launch configuration:
//!
//! ```toml
//! [[actions]]
//! name = "Split and watch"
//! key = "ctrl+alt+h"
//! steps = [{ action = "split_vertical" }, { send = "htop\n" }]
//! ```
//!
//! A step may name another user action; it runs inline. The file is
//! rejected (like an invalid `[[profiles]]` entry) when a name repeats,
//! a step names nothing Rift knows, a chord does not parse, a list is empty
//! or longer than [`MAX_STEPS`] (also once nested actions are expanded), or
//! actions call each other in a cycle.

use serde::{Deserialize, Serialize};

/// Steps in one action, and in the sequence it expands to.
pub const MAX_STEPS: usize = 32;
/// Characters one `send` step may type.
pub const MAX_SEND: usize = 4096;
/// Keymap entry name of user action `i` (`user_action_3`): the keymap maps
/// chords to names, the app maps this one back to the index.
pub const KEYMAP_PREFIX: &str = "user_action_";

/// One `[[actions]]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAction {
    /// The palette row ("Action: `<name>`"); unique.
    pub name: String,
    /// A chord, as `[keybindings]` writes them; a `[keybindings]` entry on
    /// the same chord wins, and this one wins over a default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub steps: Vec<StepSpec>,
}

/// A step as written: exactly one of its fields is set.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StepSpec {
    /// An action name (as `[keybindings]` names them) or another user
    /// action's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Text typed into the target pane; `\n` is Enter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send: Option<String>,
    /// A launch configuration's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<String>,
}

/// A step ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A built-in action (user actions are expanded before running).
    Action(String),
    Send(String),
    Open(String),
}

impl Step {
    /// How a failure toast names the step.
    pub fn describe(&self) -> String {
        match self {
            Step::Action(a) => a.clone(),
            Step::Send(s) => {
                let shown: String = s.chars().take(24).collect();
                let more = if s.chars().count() > 24 { "…" } else { "" };
                format!("send {:?}", format!("{shown}{more}"))
            }
            Step::Open(o) => format!("open {o:?}"),
        }
    }
}

impl StepSpec {
    fn parse(&self) -> Result<Step, String> {
        match (&self.action, &self.send, &self.open) {
            (Some(a), None, None) => Ok(Step::Action(a.trim().to_string())),
            (None, Some(s), None) => Ok(Step::Send(s.clone())),
            (None, None, Some(o)) => Ok(Step::Open(o.trim().to_string())),
            (None, None, None) => Err("needs one of action, send or open".into()),
            _ => Err("sets more than one of action, send and open".into()),
        }
    }
}

fn label(a: &UserAction, i: usize) -> String {
    let name = a.name.trim();
    if name.is_empty() {
        format!("actions[{i}]")
    } else {
        format!("action {name:?}")
    }
}

/// Reject what [`flatten`] could not run (see the module doc). `builtin`:
/// every action name `[keybindings]` accepts.
pub fn validate(actions: &[UserAction], builtin: &[&str]) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    let mut keys: Vec<(crate::Binding, &str)> = Vec::new();
    for (i, a) in actions.iter().enumerate() {
        let label = label(a, i);
        let name = a.name.trim();
        if name.is_empty() {
            return Err(format!("{label}: name is empty"));
        }
        if !names.insert(name) {
            return Err(format!("{label}: the name is used twice"));
        }
        if builtin.contains(&name) {
            return Err(format!("{label}: the name is a built-in action"));
        }
        if let Some(key) = &a.key {
            let Some(b) = crate::Binding::parse(key) else {
                return Err(format!("{label}: key {key:?} is not a valid chord"));
            };
            if let Some((_, other)) = keys.iter().find(|(k, _)| *k == b) {
                return Err(format!("{label}: key {key:?} is also action {other:?}'s"));
            }
            keys.push((b, name));
        }
        if a.steps.is_empty() {
            return Err(format!("{label}: steps is empty"));
        }
        if a.steps.len() > MAX_STEPS {
            return Err(format!("{label}: more than {MAX_STEPS} steps"));
        }
        for (n, spec) in a.steps.iter().enumerate() {
            let at = |why: String| format!("{label}: step {}: {why}", n + 1);
            match spec.parse().map_err(at)? {
                Step::Action(name) => {
                    let user = actions.iter().any(|o| o.name.trim() == name);
                    if !user && !builtin.contains(&name.as_str()) {
                        return Err(at(format!("{name:?} is not an action")));
                    }
                }
                Step::Send(text) => {
                    if text.is_empty() {
                        return Err(at("send is empty".into()));
                    }
                    if text.contains('\0') {
                        return Err(at("send holds a NUL byte".into()));
                    }
                    if text.chars().count() > MAX_SEND {
                        return Err(at(format!("send is longer than {MAX_SEND} characters")));
                    }
                }
                Step::Open(name) if name.is_empty() => {
                    return Err(at("open is empty".into()));
                }
                Step::Open(_) => {}
            }
        }
    }
    for i in 0..actions.len() {
        flatten(actions, i, builtin)?;
    }
    Ok(())
}

/// Action `i`'s steps with every user action it names expanded in place:
/// built-in actions, sends and opens only. Errors on a cycle or an
/// expansion longer than [`MAX_STEPS`].
pub fn flatten(actions: &[UserAction], i: usize, builtin: &[&str]) -> Result<Vec<Step>, String> {
    fn walk(
        actions: &[UserAction],
        i: usize,
        builtin: &[&str],
        stack: &mut Vec<usize>,
        out: &mut Vec<Step>,
    ) -> Result<(), String> {
        if let Some(at) = stack.iter().position(|&s| s == i) {
            let cycle: Vec<&str> = stack[at..]
                .iter()
                .chain([&i])
                .map(|&s| actions[s].name.trim())
                .collect();
            return Err(format!(
                "{}: actions call each other in a cycle ({})",
                label(&actions[stack[0]], stack[0]),
                cycle.join(", ")
            ));
        }
        stack.push(i);
        for spec in &actions[i].steps {
            let step = spec.parse()?;
            let nested = match &step {
                Step::Action(name) if !builtin.contains(&name.as_str()) => {
                    actions.iter().position(|o| o.name.trim() == name)
                }
                _ => None,
            };
            match nested {
                Some(j) => walk(actions, j, builtin, stack, out)?,
                None => out.push(step),
            }
            if out.len() > MAX_STEPS {
                return Err(format!(
                    "{}: expands to more than {MAX_STEPS} steps",
                    label(&actions[stack[0]], stack[0])
                ));
            }
        }
        stack.pop();
        Ok(())
    }
    let mut out = Vec::new();
    walk(actions, i, builtin, &mut Vec::new(), &mut out)?;
    Ok(out)
}

/// `user_action_<i>` → `i`.
pub fn keymap_index(entry: &str) -> Option<usize> {
    entry.strip_prefix(KEYMAP_PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILTIN: &[&str] = &["split_vertical", "new_tab", "clear_blocks"];

    fn parse(toml_text: &str) -> Vec<UserAction> {
        #[derive(Deserialize)]
        struct File {
            actions: Vec<UserAction>,
        }
        toml::from_str::<File>(toml_text).unwrap().actions
    }

    #[test]
    fn steps_parse_and_nested_actions_expand_in_order() {
        let actions = parse(
            r#"
            [[actions]]
            name = "watch"
            key = "ctrl+alt+h"
            steps = [{ action = "split_vertical" }, { send = "htop\n" }]

            [[actions]]
            name = "both"
            steps = [{ open = "dev" }, { action = "watch" }, { action = "clear_blocks" }]
            "#,
        );
        assert_eq!(validate(&actions, BUILTIN), Ok(()));
        assert_eq!(actions[0].key.as_deref(), Some("ctrl+alt+h"));
        assert_eq!(
            flatten(&actions, 1, BUILTIN).unwrap(),
            vec![
                Step::Open("dev".into()),
                Step::Action("split_vertical".into()),
                Step::Send("htop\n".into()),
                Step::Action("clear_blocks".into()),
            ]
        );
    }

    #[test]
    fn invalid_actions_are_rejected_with_their_name() {
        let err = |t: &str| validate(&parse(t), BUILTIN).unwrap_err();
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = [{ action = "spit_vertical" }]"#),
            r#"action "a": step 1: "spit_vertical" is not an action"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                key = "cmd+nope"
                steps = [{ action = "new_tab" }]"#),
            r#"action "a": key "cmd+nope" is not a valid chord"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = []"#),
            r#"action "a": steps is empty"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = [{ action = "new_tab", send = "x" }]"#),
            r#"action "a": step 1: sets more than one of action, send and open"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = [{ }]"#),
            r#"action "a": step 1: needs one of action, send or open"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "new_tab"
                steps = [{ action = "new_tab" }]"#),
            r#"action "new_tab": the name is a built-in action"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = [{ action = "new_tab" }]
                [[actions]]
                name = "a"
                steps = [{ action = "new_tab" }]"#),
            r#"action "a": the name is used twice"#
        );
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                key = "cmd+y"
                steps = [{ action = "new_tab" }]
                [[actions]]
                name = "b"
                key = "cmd+y"
                steps = [{ action = "new_tab" }]"#),
            r#"action "b": key "cmd+y" is also action "a"'s"#
        );
        let many = format!(
            "[[actions]]\nname = \"a\"\nsteps = [{}]",
            vec![r#"{ action = "new_tab" }"#; MAX_STEPS + 1].join(", ")
        );
        assert_eq!(err(&many), r#"action "a": more than 32 steps"#);
        assert_eq!(
            err(r#"[[actions]]
                name = "a"
                steps = [{ send = "x\u0000" }]"#),
            r#"action "a": step 1: send holds a NUL byte"#
        );
    }

    #[test]
    fn cycles_and_long_expansions_are_rejected() {
        let cyclic = parse(
            r#"
            [[actions]]
            name = "a"
            steps = [{ action = "new_tab" }, { action = "b" }]
            [[actions]]
            name = "b"
            steps = [{ action = "a" }]
            "#,
        );
        assert_eq!(
            validate(&cyclic, BUILTIN).unwrap_err(),
            r#"action "a": actions call each other in a cycle (a, b, a)"#
        );
        let selfish = parse(
            r#"
            [[actions]]
            name = "me"
            steps = [{ action = "me" }]
            "#,
        );
        assert!(
            validate(&selfish, BUILTIN)
                .unwrap_err()
                .contains("(me, me)")
        );
        // each list is short, the expansion is not
        let twenty = vec![r#"{ action = "new_tab" }"#; 20].join(", ");
        let long = parse(&format!(
            "[[actions]]\nname = \"x\"\nsteps = [{twenty}]\n\
             [[actions]]\nname = \"y\"\nsteps = [{{ action = \"x\" }}, {{ action = \"x\" }}]"
        ));
        assert_eq!(
            validate(&long, BUILTIN).unwrap_err(),
            r#"action "y": expands to more than 32 steps"#
        );
        // the same action twice (not a cycle) is fine
        let twice = parse(
            r#"
            [[actions]]
            name = "x"
            steps = [{ action = "new_tab" }]
            [[actions]]
            name = "y"
            steps = [{ action = "x" }, { action = "x" }]
            "#,
        );
        assert_eq!(flatten(&twice, 1, BUILTIN).unwrap().len(), 2);
    }

    #[test]
    fn keymap_entries_name_the_index() {
        assert_eq!(keymap_index("user_action_3"), Some(3));
        assert_eq!(keymap_index("user_action_x"), None);
        assert_eq!(keymap_index("new_tab"), None);
    }
}
