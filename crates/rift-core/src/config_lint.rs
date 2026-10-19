//! Problems in config.toml text, each at its line and columns, for the
//! config editor: TOML syntax, a value of the wrong type, an unknown key
//! (with the closest known one), a key Rift no longer reads, an invalid
//! `[[profiles]]` / `[[actions]]` / `[[secret_patterns]]` entry, an unknown
//! action or unparsable chord in `[keybindings]`, and a generated profile
//! that is no longer Rift's.
//!
//! Invariant: text with no [`Severity::Error`] loads (`Config::try_load_from`
//! is `Ok`), so text that would be rejected always shows an error. Not the
//! other way round: the loader ignores unknown keys and warns about bad
//! `[keybindings]` entries, which lint calls errors.

use crate::Config;
use std::collections::BTreeSet;
use std::ops::Range;
use toml_edit::{Item, TableLike, Value};

/// How bad a [`Diagnostic`] is: an error blocks saving the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One problem in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 0-based line.
    pub line: usize,
    /// Character columns within the line (an empty range: a point).
    pub cols: Range<usize>,
    pub severity: Severity,
    pub message: String,
}

/// Top-level keys of earlier versions that Rift reads no more (a file
/// holding them still loads).
pub const REMOVED_KEYS: &[&str] = &[
    "background_processes",
    "remote_when_closed",
    "menu_bar_icon",
];

/// The problems in `text`, in line order.
pub fn lint(text: &str) -> Vec<Diagnostic> {
    lint_with(text, None)
}

/// [`lint`], also warning about a `source = "shells"` profile whose command
/// is not in `seen` (the generated-profiles list): Rift did not write it, so
/// it is now the user's own.
pub fn lint_with(text: &str, seen: Option<&BTreeSet<String>>) -> Vec<Diagnostic> {
    let doc = match toml_edit::Document::parse(text) {
        Ok(d) => d,
        Err(e) => return vec![at(text, e.span(), Severity::Error, e.message().to_string())],
    };
    let mut out = Vec::new();
    let schema = config_schema();
    let top_keys: Vec<String> = match &schema {
        Schema::Struct(keys) => keys.iter().map(|(k, _)| k.clone()).collect(),
        _ => Vec::new(),
    };
    let mut l = Linter {
        text,
        out: &mut out,
        top_keys,
    };
    l.walk(doc.as_table(), &schema, None);
    if let Some(kb) = doc.get("keybindings").and_then(Item::as_table_like) {
        l.keybindings(kb);
    }
    match toml::from_str::<Config>(text) {
        Err(e) => l.push(e.span(), Severity::Error, e.message().to_string()),
        Ok(c) => l.entries(&doc, &c, seen),
    }
    out.sort_by_key(|d| (d.line, d.cols.start));
    out
}

/// The known keys of a table.
enum Schema {
    /// Any value (a plain setting, an array of strings).
    Any,
    /// A map: any key (`[keybindings]`, a profile's `env`).
    Map,
    Struct(Vec<(String, Schema)>),
    /// An array of tables, each one of these.
    Tables(Box<Schema>),
}

fn fields(names: &[&str]) -> Vec<(String, Schema)> {
    names.iter().map(|n| (n.to_string(), Schema::Any)).collect()
}

/// `[[profiles]]` keys; `profile_keys_are_complete` keeps it in step with
/// `Profile`.
const PROFILE_KEYS: &[&str] = &[
    "name", "command", "args", "cwd", "title", "color", "icon", "theme", "source", "hidden",
];

fn profile_schema() -> Schema {
    let mut f = fields(PROFILE_KEYS);
    f.push(("env".into(), Schema::Map));
    Schema::Struct(f)
}

fn action_schema() -> Schema {
    let mut f = fields(&["name", "key"]);
    let step = Schema::Struct(fields(&["action", "send", "open"]));
    f.push(("steps".into(), Schema::Tables(Box::new(step))));
    Schema::Struct(f)
}

/// Every key config.toml may hold: the default config's (nested tables
/// recursively), the optional ones a default omits, and the entries of the
/// arrays of tables.
fn config_schema() -> Schema {
    let Ok(toml::Value::Table(default)) = toml::Value::try_from(Config::default()) else {
        return Schema::Map;
    };
    fn of(v: &toml::Value) -> Schema {
        match v {
            toml::Value::Table(t) if t.is_empty() => Schema::Map,
            toml::Value::Table(t) => {
                Schema::Struct(t.iter().map(|(k, v)| (k.clone(), of(v))).collect())
            }
            _ => Schema::Any,
        }
    }
    let tables = |s: Schema| Schema::Tables(Box::new(s));
    let mut keys: Vec<(String, Schema)> = default
        .iter()
        .map(|(k, v)| {
            let s = match k.as_str() {
                "profiles" => tables(profile_schema()),
                "actions" => tables(action_schema()),
                "ssh_profiles" => tables(Schema::Struct(fields(&["name", "host", "args"]))),
                "secret_patterns" => tables(Schema::Struct(fields(&["name", "regex"]))),
                "tab_color_rules" => tables(Schema::Struct(fields(&["dir", "color"]))),
                _ => of(v),
            };
            (k.clone(), s)
        })
        .collect();
    keys.extend(fields(crate::config::OPTIONAL_KEYS));
    Schema::Struct(keys)
}

struct Linter<'a> {
    text: &'a str,
    out: &'a mut Vec<Diagnostic>,
    /// The top-level keys: one met inside a table was meant above it.
    top_keys: Vec<String>,
}

impl Linter<'_> {
    fn push(&mut self, span: Option<Range<usize>>, severity: Severity, message: String) {
        self.out.push(at(self.text, span, severity, message));
    }

    /// Unknown and removed keys under `table`, recursively. `header`: the
    /// table's header as written (`[[profiles]]`); `None`: the top level.
    fn walk(&mut self, table: &dyn TableLike, schema: &Schema, header: Option<&str>) {
        let Schema::Struct(known) = schema else {
            return;
        };
        for (key, item) in table.iter() {
            let span = table.get_key_value(key).and_then(|(k, _)| k.span());
            match known.iter().find(|(k, _)| k == key) {
                Some((_, sub)) => self.item(key, header, item, sub),
                None if header.is_none() && REMOVED_KEYS.contains(&key) => self.push(
                    span,
                    Severity::Warning,
                    format!("`{key}` is no longer used and is ignored"),
                ),
                // a setting typed below a table (the generated profiles end
                // the file) belongs to that table
                None if header.is_some() && self.top_keys.iter().any(|k| k == key) => {
                    let header = header.unwrap_or_default();
                    self.push(
                        span,
                        Severity::Error,
                        format!(
                            "`{key}` is read as part of {header} here: move it above the first table"
                        ),
                    )
                }
                None => {
                    let message = match closest(key, known.iter().map(|(k, _)| k.as_str())) {
                        Some(m) => format!("unknown key `{key}` — did you mean `{m}`?"),
                        None => format!("unknown key `{key}`"),
                    };
                    self.push(span, Severity::Error, message)
                }
            }
        }
    }

    /// The value of `key` (in the table under `parent`) against its schema.
    fn item(&mut self, key: &str, parent: Option<&str>, item: &Item, schema: &Schema) {
        let path = match parent {
            Some(p) => format!("{}.{key}", p.trim_matches(['[', ']', '`'])),
            None => key.to_string(),
        };
        match (item, schema) {
            (Item::ArrayOfTables(a), Schema::Tables(s)) => {
                let header = format!("[[{path}]]");
                for t in a.iter() {
                    self.walk(t, s, Some(&header));
                }
            }
            (Item::Value(Value::Array(a)), Schema::Tables(s)) => {
                let header = format!("`{path}`");
                for v in a.iter() {
                    if let Value::InlineTable(t) = v {
                        self.walk(t, s, Some(&header));
                    }
                }
            }
            (_, Schema::Struct(_)) => {
                if let Some(t) = item.as_table_like() {
                    let header = if item.is_table() {
                        format!("[{path}]")
                    } else {
                        format!("`{path}`")
                    };
                    self.walk(t, schema, Some(&header));
                }
            }
            _ => {}
        }
    }

    /// `[keybindings]`: each key an action, each value a chord (or `none`).
    fn keybindings(&mut self, table: &dyn TableLike) {
        let actions = crate::keybindings::all_actions();
        for (action, item) in table.iter() {
            if !actions.contains(&action) {
                let span = table.get_key_value(action).and_then(|(k, _)| k.span());
                let message = match closest(action, actions.iter().copied()) {
                    Some(m) => format!("unknown action `{action}` — did you mean `{m}`?"),
                    None => format!("unknown action `{action}`"),
                };
                self.push(span, Severity::Error, message);
                continue;
            }
            // a non-string value is the type error `toml` reports
            let Some(chord) = item.as_str() else {
                continue;
            };
            if !matches!(chord.trim(), "" | "none")
                && crate::keybindings::Binding::parse(chord).is_none()
            {
                self.push(
                    item.span(),
                    Severity::Error,
                    format!("`{chord}` is not a key chord (e.g. `cmd+shift+t`)"),
                );
            }
        }
    }

    /// The checks the loader runs on whole entries, at the first entry that
    /// fails them; then the generated profiles Rift did not write.
    fn entries(
        &mut self,
        doc: &toml_edit::Document<&str>,
        c: &Config,
        seen: Option<&BTreeSet<String>>,
    ) {
        /// An array's key, its length, and the loader's check of its first
        /// `n` entries.
        type Check<'a> = (&'static str, usize, &'a dyn Fn(usize) -> Result<(), String>);
        let builtin = crate::keybindings::all_actions();
        let checks: [Check; 3] = [
            ("profiles", c.profiles.len(), &|n| {
                crate::profiles::validate(&c.profiles[..n])
            }),
            ("actions", c.actions.len(), &|n| {
                crate::actions::validate(&c.actions[..n], &builtin)
            }),
            ("secret_patterns", c.secret_patterns.len(), &|n| {
                crate::secrets::validate(&c.secret_patterns[..n])
            }),
        ];
        for (key, len, check) in checks {
            let Err(e) = check(len) else {
                continue;
            };
            // the first prefix failing with this same error ends at the
            // offending entry (an earlier prefix may fail otherwise: an
            // action naming one further down)
            let at = (1..=len)
                .find(|&n| check(n).err().as_ref() == Some(&e))
                .map(|n| n - 1);
            let span = at
                .and_then(|i| entry_span(doc, key, i))
                .or_else(|| key_span(doc, key));
            self.push(span, Severity::Error, e);
        }
        // a default naming no profile (one renamed or deleted here)
        if let Some(name) = c
            .default_profile
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            && !c.profiles.iter().any(|p| p.name.trim() == name)
        {
            let span = doc
                .get("default_profile")
                .and_then(Item::span)
                .or_else(|| key_span(doc, "default_profile"));
            self.push(
                span,
                Severity::Warning,
                format!("no profile is named \"{name}\": new tabs run the Shell path"),
            );
        }
        if let Some(seen) = seen {
            for (i, p) in c.profiles.iter().enumerate() {
                if p.is_generated() && !p.command.as_ref().is_some_and(|cmd| seen.contains(cmd)) {
                    let span = entry_span(doc, "profiles", i).or_else(|| key_span(doc, "profiles"));
                    self.push(
                        span,
                        Severity::Warning,
                        "this is now your own profile".to_string(),
                    );
                }
            }
        }
    }
}

/// Where the `i`-th entry of the array `key` starts (its `[[key]]` header,
/// or the inline table).
fn entry_span(doc: &toml_edit::Document<&str>, key: &str, i: usize) -> Option<Range<usize>> {
    match doc.get(key)? {
        Item::ArrayOfTables(a) => a.get(i)?.span(),
        Item::Value(Value::Array(a)) => a.get(i)?.span(),
        _ => None,
    }
}

fn key_span(doc: &toml_edit::Document<&str>, key: &str) -> Option<Range<usize>> {
    doc.get_key_value(key).and_then(|(k, _)| k.span())
}

/// A diagnostic at byte `span` of `text` (none: its start), its columns
/// clipped to the span's first line.
fn at(text: &str, span: Option<Range<usize>>, severity: Severity, message: String) -> Diagnostic {
    let span = span.unwrap_or(0..0);
    let floor = |i: usize| {
        let mut i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = (floor(span.start), floor(span.end.max(span.start)));
    let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
    let line_end = text[start..].find('\n').map_or(text.len(), |n| start + n);
    let col = text[line_start..start].chars().count();
    let width = text[start..end.min(line_end).max(start)].chars().count();
    Diagnostic {
        line: text[..start].matches('\n').count(),
        cols: col..col + width,
        severity,
        message: message.trim().to_string(),
    }
}

/// The candidate nearest `word` within edit distance 2 (the first on a tie).
fn closest<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .map(|c| (levenshtein(word, c), c))
        .filter(|&(d, _)| d <= 2)
        .min_by_key(|&(d, _)| d)
        .map(|(_, c)| c)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cur = row[j + 1];
            row[j + 1] = (prev + usize::from(ca != *cb)).min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(text: &str) -> Vec<Diagnostic> {
        lint(text)
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .collect()
    }

    fn one(text: &str) -> Diagnostic {
        let got = lint(text);
        assert_eq!(got.len(), 1, "{got:?}");
        got.into_iter().next().unwrap()
    }

    #[test]
    fn a_valid_file_has_no_diagnostics() {
        let text = "# mine\nfont_size = 15.0\nshell = \"/bin/zsh\"\n\n[notifications]\nenabled = false\n\n[keybindings]\nnew_tab = \"cmd+shift+t\"\nclear_blocks = \"none\"\n\n[[profiles]]\nname = \"dev\"\ncommand = \"/bin/zsh\"\nsource = \"shells\"\nhidden = true\n[profiles.env]\nANY_NAME = \"1\"\n\n[[actions]]\nname = \"deploy\"\n[[actions.steps]]\nsend = \"make\\n\"\n\n[[ssh_profiles]]\nname = \"box\"\nhost = \"me@box\"\n";
        assert_eq!(lint(text), []);
    }

    #[test]
    fn a_syntax_error_is_at_its_line() {
        let d = one("font_size = 13\ntheme = \"dark\n");
        assert_eq!((d.line, d.severity), (1, Severity::Error));
        assert!(d.message.contains("string"), "{}", d.message);
    }

    #[test]
    fn a_wrong_type_keeps_serdes_message() {
        let d = one("font_size = 13\n\ninput_position = \"middle\"\n");
        assert_eq!(
            (d.line, d.cols.clone(), d.severity),
            (2, 17..25, Severity::Error)
        );
        assert!(
            d.message.contains("unknown variant `middle`") && d.message.contains("bottom"),
            "{}",
            d.message
        );
    }

    #[test]
    fn an_unknown_key_suggests_the_closest_one() {
        let d = one("theme = \"dark-default\"\nfont_sise = 14\n");
        assert_eq!(
            d,
            Diagnostic {
                line: 1,
                cols: 0..9,
                severity: Severity::Error,
                message: "unknown key `font_sise` — did you mean `font_size`?".into(),
            }
        );
        assert_eq!(one("zzzzzzzz = 1\n").message, "unknown key `zzzzzzzz`");
        // nested tables and array entries are checked against their own keys
        let d = one("[notifications]\nenabeld = true\n");
        assert_eq!(
            (d.line, d.message.as_str()),
            (1, "unknown key `enabeld` — did you mean `enabled`?")
        );
        let d = one("[[profiles]]\nname = \"a\"\ncomand = \"/bin/sh\"\n");
        assert_eq!(
            (d.line, d.message.as_str()),
            (2, "unknown key `comand` — did you mean `command`?")
        );
        let d = one("actions = [{ name = \"a\", steps = [{ send = \"x\", opn = 1 }] }]\n");
        assert_eq!(
            (d.line, d.cols, d.message.as_str()),
            (0, 48..51, "unknown key `opn` — did you mean `open`?")
        );
        // optional settings a default omits are known
        assert_eq!(
            lint(
                "default_profile = \"a\"\nright_option_as_meta = true\n[[profiles]]\nname = \"a\"\n"
            ),
            []
        );
    }

    #[test]
    fn a_setting_typed_under_a_table_says_where_it_belongs() {
        // the generated profiles end the file: a setting added at the end
        // lands in the last one
        let d = one("[[profiles]]\nname = \"zsh\"\nfont_size = 14\n");
        assert_eq!(
            (d.line, d.cols, d.message.as_str()),
            (
                2,
                0..9,
                "`font_size` is read as part of [[profiles]] here: move it above the first table"
            )
        );
        let d = one("[notifications]\nenabled = true\ntheme = \"nord\"\n");
        assert!(
            d.message.contains("part of [notifications]"),
            "{}",
            d.message
        );
        let d = one("[[actions]]\nname = \"a\"\n[[actions.steps]]\nsend = \"x\"\ntheme = \"y\"\n");
        assert!(
            d.message.contains("part of [[actions.steps]]"),
            "{}",
            d.message
        );
        // a key no level knows still gets its did-you-mean
        assert_eq!(
            one("[[profiles]]\nname = \"a\"\nthme = \"x\"\n").message,
            "unknown key `thme` — did you mean `theme`?"
        );
    }

    #[test]
    fn a_default_naming_no_profile_warns() {
        let d = one("default_profile = \"work\"\n\n[[profiles]]\nname = \"job\"\n");
        assert_eq!(
            (d.line, d.cols, d.severity, d.message.as_str()),
            (
                0,
                18..24,
                Severity::Warning,
                "no profile is named \"work\": new tabs run the Shell path"
            )
        );
        assert_eq!(
            lint("default_profile = \" job \"\n[[profiles]]\nname = \"job\"\n"),
            []
        );
        assert_eq!(lint("default_profile = \"\"\n"), []);
    }

    #[test]
    fn a_removed_key_is_a_warning() {
        let d = one("font_size = 13\nmenu_bar_icon = true\n");
        assert_eq!(
            (d.line, d.cols, d.severity, d.message.as_str()),
            (
                1,
                0..13,
                Severity::Warning,
                "`menu_bar_icon` is no longer used and is ignored"
            )
        );
    }

    #[test]
    fn an_invalid_entry_is_at_its_header() {
        let text = "[[profiles]]\nname = \"a\"\n\n[[profiles]]\nname = \"a\"\n";
        let d = one(text);
        assert_eq!(
            (d.line, d.severity, d.message.as_str()),
            (3, Severity::Error, "profile \"a\": the name is used twice")
        );
        let d = one("[[secret_patterns]]\nname = \"x\"\nregex = \"(\"\n");
        assert_eq!((d.line, d.severity), (0, Severity::Error));
        let d = one("[[actions]]\nname = \"a\"\n[[actions.steps]]\naction = \"nope\"\n");
        assert_eq!((d.line, d.severity), (0, Severity::Error));
        assert!(d.message.contains("nope"), "{}", d.message);
        // an action naming one further down is not the culprit
        let text = "[[actions]]\nname = \"a\"\nsteps = [{ action = \"b\" }]\n\n[[actions]]\nname = \"b\"\nsteps = [{ action = \"nope\" }]\n";
        let d = one(text);
        assert_eq!(
            (d.line, d.message.as_str()),
            (4, "action \"b\": step 1: \"nope\" is not an action")
        );
    }

    #[test]
    fn keybindings_need_known_actions_and_chords() {
        let d = one("[keybindings]\nnew_tabb = \"cmd+t\"\n");
        assert_eq!(
            (d.line, d.cols, d.message.as_str()),
            (
                1,
                0..8,
                "unknown action `new_tabb` — did you mean `new_tab`?"
            )
        );
        let d = one("[keybindings]\nnew_tab = \"cmd+\"\n");
        assert_eq!(
            (d.line, d.cols, d.message.as_str()),
            (1, 10..16, "`cmd+` is not a key chord (e.g. `cmd+shift+t`)")
        );
    }

    #[test]
    fn a_generated_profile_rift_did_not_write_is_the_users() {
        let text = "[[profiles]]\nname = \"zsh\"\ncommand = \"/bin/zsh\"\nsource = \"shells\"\n";
        let seen = BTreeSet::from(["/bin/zsh".to_string()]);
        assert_eq!(lint_with(text, Some(&seen)), []);
        let other = BTreeSet::from(["/bin/bash".to_string()]);
        let got = lint_with(text, Some(&other));
        assert_eq!(got.len(), 1);
        assert_eq!(
            (got[0].line, got[0].severity, got[0].message.as_str()),
            (0, Severity::Warning, "this is now your own profile")
        );
        assert_eq!(lint(text), [], "without a seen list nothing is said");
    }

    #[test]
    fn columns_count_characters() {
        let d = one("# é\n\"日本\" = 1\n");
        assert_eq!((d.line, d.cols), (1, 0..4));
        // after a non-ASCII value on the same line
        let d = one("notifications = { sound = \"é\", enabeld = true }\n");
        assert_eq!((d.line, d.cols), (0, 31..38));
    }

    #[test]
    fn profile_keys_are_complete() {
        let mut p = crate::profiles::Profile::named("n");
        p.command = Some("c".into());
        p.args = vec!["a".into()];
        p.cwd = Some("d".into());
        p.title = Some("t".into());
        p.color = Some("red".into());
        p.icon = Some("i".into());
        p.theme = Some("th".into());
        p.source = Some("s".into());
        p.hidden = true;
        p.env.insert("K".into(), "V".into());
        let toml::Value::Table(t) = toml::Value::try_from(&p).unwrap() else {
            panic!()
        };
        let mut got: Vec<&str> = t.keys().map(String::as_str).collect();
        got.sort();
        let mut want: Vec<&str> = PROFILE_KEYS.iter().copied().chain(["env"]).collect();
        want.sort();
        assert_eq!(got, want);
    }

    /// No error ⇒ the file loads, over every text these tests use (and a
    /// rejected file always has an error).
    #[test]
    fn no_error_means_the_file_loads() {
        let fixtures = [
            "",
            "font_size = 15.0\n",
            "font_size = [\n",
            "input_position = \"middle\"\n",
            "font_sise = 14\n",
            "menu_bar_icon = true\n",
            "[[profiles]]\nname = \"a\"\n\n[[profiles]]\nname = \"a\"\n",
            "[[profiles]]\nname = \"\"\n",
            "[[profiles]]\nname = \"c\"\ncolor = \"purple\"\n",
            "[[profiles]]\nname = \"zsh\"\nsource = \"shells\"\nhidden = true\n",
            "[[secret_patterns]]\nname = \"x\"\nregex = \"(\"\n",
            "[[actions]]\nname = \"a\"\n[[actions.steps]]\naction = \"nope\"\n",
            "[keybindings]\nnew_tabb = \"cmd+t\"\n",
            "[keybindings]\nnew_tab = 3\n",
            "cursor_style = 7\n",
            "notifications = 1\n",
            "default_profile = 3\n",
        ];
        let dir = std::env::temp_dir().join(format!("rift-lint-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        for text in fixtures {
            std::fs::write(&path, text).unwrap();
            let loads = Config::try_load_from(&path).is_ok();
            let clean = errors(text).is_empty();
            assert!(
                !clean || loads,
                "no lint error but the file is rejected: {text:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
