//! What a running command waits for, read from the pane (a paired phone's
//! "Needs you", docs/remote-protocol.md `waiting`): a password the terminal
//! reads without echo, or a coding agent's menu (Claude Code, Codex: a
//! question, then entries under a `❯` / `›` selector, numbered or not).
//! Pure: the window hands in the screen's rows and the terminal's state.

/// Most choices kept, characters kept per choice, per prompt and per
/// context line, and context lines kept: the wire's `MAX_WAITING_*`
/// (rift-remote, which cannot depend on this crate; a rift-terminal test
/// keeps both equal).
pub const MAX_CHOICES: usize = 9;
pub const MAX_CHOICE_CHARS: usize = 200;
pub const MAX_PROMPT_CHARS: usize = 300;
pub const MAX_CONTEXT_LINES: usize = 12;

/// The selector a menu draws in front of its current entry.
const SELECTORS: [char; 5] = ['❯', '›', '▶', '➤', '>'];
/// Frame characters drawn around a menu or between its parts.
const FRAME: &[char] = &[
    '│', '┃', '║', '▌', '▐', '╭', '╮', '╰', '╯', '─', '━', '═', '╌', '┄', '┈', '┊', '┆', '╎', '|',
];

/// A menu waiting for the user to pick an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// The question above the menu.
    pub prompt: String,
    /// The entries, without their numbers.
    pub choices: Vec<String>,
    /// The entry under the selector (0-based).
    pub selected: usize,
    /// Entries are numbered `1.` … : typing the digit picks one.
    /// Otherwise the selector moves with the arrow keys and Enter picks.
    pub numbered: bool,
    /// Screen lines above the question (what is being asked about).
    pub context: Vec<String>,
}

/// What a pane waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Waiting {
    /// A line read without echo (a password, a passphrase).
    Password {
        prompt: String,
    },
    Choice(Choices),
}

/// `s` cut to `max` characters.
fn cut(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// A row without its frame (box borders at either end) and trailing blanks;
/// a separator row stays as it is.
fn unframed(row: &str) -> &str {
    if is_rule(row) {
        return row.trim_end();
    }
    row.trim_end()
        .trim_end_matches(|c: char| FRAME.contains(&c) || c.is_whitespace())
        .trim_start_matches(|c: char| FRAME.contains(&c))
}

/// A solid separator (`───`) between parts of an agent's screen; dashed
/// ones (`╌╌╌`) frame a diff inside a part.
fn is_solid_rule(row: &str) -> bool {
    is_rule(row) && row.contains(['─', '━', '═'])
}

/// A row that is only frame (a separator line) or blank.
fn is_rule(row: &str) -> bool {
    row.chars().all(|c| FRAME.contains(&c) || c.is_whitespace())
}

/// The leading spaces of a row (its indent, in characters).
fn indent(row: &str) -> usize {
    row.chars().take_while(|c| *c == ' ').count()
}

/// `N. text` (after the indent): the number and the text.
fn numbered(text: &str) -> Option<(usize, &str)> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    let rest = text[digits.len()..].strip_prefix(". ")?;
    let n = digits.parse().ok()?;
    Some((n, rest.trim_start()))
}

/// A menu entry row: `(selected, text column, text)`; the selector, when
/// there, must be followed by a space and text.
fn entry(row: &str) -> Option<(bool, usize, &str)> {
    if is_rule(row) {
        return None;
    }
    let lead = indent(row);
    let rest = &row[lead..];
    let mut chars = rest.chars();
    let first = chars.next()?;
    if SELECTORS.contains(&first) {
        let after = &rest[first.len_utf8()..];
        let text = after.trim_start();
        // a bare selector is an empty input line, not a menu entry
        if text.is_empty() || !after.starts_with(' ') {
            return None;
        }
        let col = lead + 1 + (after.chars().count() - text.chars().count());
        return Some((true, col, text));
    }
    (!rest.is_empty()).then_some((false, lead, rest))
}

/// A coding agent's menu on the screen's last rows: a block of entries, one
/// under the selector, either numbered 1 … n (the digit picks) or, without
/// numbers, followed by a hint row such as "Enter to confirm".
pub fn parse_choices(rows: &[String]) -> Option<Choices> {
    let rows: Vec<&str> = rows.iter().map(|r| unframed(r)).collect();
    // the blank rows under the last text are not part of the screen's
    // content (a tall window); then the last 40 rows are enough for any
    // menu and its question
    let end = rows
        .iter()
        .rposition(|r| !r.trim().is_empty())
        .map_or(0, |i| i + 1);
    let rows = &rows[end.saturating_sub(40)..end];
    // the selector row lowest on screen that has a menu around it
    for (sel, row) in rows.iter().enumerate().rev() {
        let Some((true, col, _)) = entry(row) else {
            continue;
        };
        if let Some(c) = menu_at(rows, sel, col) {
            return Some(c);
        }
    }
    None
}

/// The menu whose selected entry is row `sel` with its text at `col`.
fn menu_at(rows: &[&str], sel: usize, col: usize) -> Option<Choices> {
    // entries: rows above and below whose text starts at the same column
    // (a longer entry wraps onto rows indented deeper)
    let same = |r: &str| entry(r).is_some_and(|(_, c, _)| c == col);
    // a wrapped row of an entry: deeper, not under the selector
    let wrapped =
        |r: &str| !r.trim().is_empty() && indent(r) > col && entry(r).is_some_and(|(s, _, _)| !s);
    // upward, wrapped rows count only below an entry row
    let mut first = sel;
    let mut up = sel;
    while up > 0 && (same(rows[up - 1]) || wrapped(rows[up - 1])) {
        up -= 1;
        if same(rows[up]) {
            first = up;
        }
    }
    let mut last = sel;
    while last + 1 < rows.len() && (same(rows[last + 1]) || wrapped(rows[last + 1])) {
        last += 1;
    }
    // join wrapped rows onto their entry
    let mut items: Vec<(bool, String)> = Vec::new();
    for row in &rows[first..=last] {
        let (selected, c, text) = entry(row)?;
        if c == col {
            items.push((selected, text.trim_end().to_string()));
        } else if let Some(prev) = items.last_mut() {
            prev.1.push(' ');
            prev.1.push_str(text.trim());
        }
    }
    if !(2..=MAX_CHOICES).contains(&items.len()) {
        return None;
    }
    let selected = items.iter().position(|(s, _)| *s)?;
    let parsed: Vec<Option<(usize, &str)>> = items.iter().map(|(_, t)| numbered(t)).collect();
    let is_numbered = parsed
        .iter()
        .enumerate()
        .all(|(i, p)| p.is_some_and(|(n, _)| n == i + 1));
    // after the menu: only a hint row or two, then nothing (or the frame)
    let below: Vec<&str> = rows[last + 1..]
        .iter()
        .copied()
        .filter(|r| !is_rule(r))
        .collect();
    let hinted = below.iter().take(2).any(|r| {
        let t = r.to_lowercase();
        [
            "enter to",
            "esc to",
            "to cancel",
            "to confirm",
            "to select",
            "tab to",
        ]
        .iter()
        .any(|h| t.contains(h))
    });
    if below.len() > 4 || (!is_numbered && !hinted) {
        return None;
    }
    let choices: Vec<String> = items
        .iter()
        .zip(&parsed)
        .map(|((_, t), p)| {
            let text = if is_numbered {
                p.map_or(t.as_str(), |(_, t)| t)
            } else {
                t
            };
            cut(text, MAX_CHOICE_CHARS)
        })
        .collect();
    // the question: the nearest row above with a `?`, within a few rows,
    // else the nearest text row
    let above: Vec<(usize, &str)> = rows[..first]
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, r)| !is_rule(r))
        .take(8)
        .map(|(i, r)| (i, *r))
        .collect();
    let (q_at, question) = above
        .iter()
        .find(|(_, r)| r.contains('?'))
        .or(above.first())
        .copied()?;
    // context: the rows between the last rule above the question and it
    let top = rows[..q_at]
        .iter()
        .rposition(|r| is_solid_rule(r))
        .map_or(0, |i| i + 1);
    let context: Vec<String> = rows[top..q_at]
        .iter()
        .filter(|r| !is_rule(r))
        .map(|r| cut(r.trim_end(), MAX_CHOICE_CHARS))
        .collect();
    let skip = context.len().saturating_sub(MAX_CONTEXT_LINES);
    Some(Choices {
        prompt: cut(question.trim(), MAX_PROMPT_CHARS),
        choices,
        selected,
        numbered: is_numbered,
        context: context.into_iter().skip(skip).collect(),
    })
}

/// What a pane's running command waits for: a password when the terminal
/// reads without echo in line mode (`input_hidden`) or the cursor line reads
/// like a password prompt, else a menu on `agent_rows` — the screen of a
/// coding agent; the caller passes none for any other program, whose
/// menu-shaped output a phone must not answer.
pub fn waiting(input_hidden: bool, cursor_line: &str, agent_rows: &[String]) -> Option<Waiting> {
    let line = cursor_line.trim();
    if input_hidden || crate::notifications::looks_like_password_prompt(cursor_line) {
        let prompt = if line.is_empty() { "Password:" } else { line };
        return Some(Waiting::Password {
            prompt: cut(prompt, MAX_PROMPT_CHARS),
        });
    }
    parse_choices(agent_rows).map(Waiting::Choice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn claude_codes_edit_permission_captured_from_a_real_screen() {
        let screen = rows(include_str!("../testdata/prompts/claude-edit.txt"));
        let c = parse_choices(&screen).expect("a menu");
        assert_eq!(c.prompt, "Do you want to make this edit to app.txt?");
        assert_eq!(
            c.choices,
            [
                "Yes",
                "Yes, and switch to accept edits (auto-approve file edits and common file commands) for this session (shift+tab)",
                "No"
            ]
        );
        assert_eq!((c.selected, c.numbered), (0, true));
        assert_eq!(c.context.first().map(String::as_str), Some(" Edit file"));
        assert!(c.context.iter().any(|l| l.contains("1 +ONE")));
        assert!(c.context.iter().all(|l| !l.contains('╌')));
    }

    #[test]
    fn claude_codes_folder_trust_menu_has_no_numbers() {
        let screen = rows(include_str!("../testdata/prompts/claude-trust.txt"));
        let c = parse_choices(&screen).expect("a menu");
        assert_eq!(c.choices, ["No, exit", "Yes, I trust this folder"]);
        assert_eq!((c.selected, c.numbered), (0, false));
        assert!(
            c.prompt
                .contains("Is this a project you created or one you trust?"),
            "{}",
            c.prompt
        );
    }

    #[test]
    fn codex_style_menus_and_wrapped_entries() {
        let screen = rows(
            "  Would you like to run the following command?\n\n  $ rm -rf target\n\n\
             › 1. Yes, proceed\n  2. Yes, and don't ask again for this command\n  \
             3. No, and tell Codex what to do differently (esc)\n\n  Press enter to confirm or esc to cancel",
        );
        let c = parse_choices(&screen).unwrap();
        assert_eq!(c.prompt, "Would you like to run the following command?");
        assert_eq!(c.choices.len(), 3);
        assert_eq!(
            c.choices[2],
            "No, and tell Codex what to do differently (esc)"
        );
        // a long entry wrapped onto a deeper-indented row is one choice
        let wrapped = rows(
            " Proceed?\n ❯ 1. Yes\n   2. Yes, and switch to accept edits for\n      this session\n   3. No\n Esc to cancel",
        );
        let c = parse_choices(&wrapped).unwrap();
        assert_eq!(
            c.choices,
            [
                "Yes",
                "Yes, and switch to accept edits for this session",
                "No"
            ]
        );
    }

    #[test]
    fn idle_prompts_transcripts_and_plain_lists_are_not_menus() {
        // Claude Code idle: a bare selector in its input box
        assert_eq!(
            parse_choices(&rows("⏺ Done.\n───\n❯ \n───\n  ⏵⏵ auto mode on")),
            None
        );
        // a past user turn followed by output at the same column
        assert_eq!(
            parse_choices(&rows(
                "❯ Run this exact shell command: rm hello.txt\n  Ran 1 shell command\n⏺ Done.\n✻ Cooked\n───\n❯ \n───\n status\n mode"
            )),
            None
        );
        // a numbered list in output without a selector
        assert_eq!(
            parse_choices(&rows("Steps:\n  1. build\n  2. test\n$ ")),
            None
        );
        // a shell prompt with `>` continuation
        assert_eq!(parse_choices(&rows("$ echo 'a\n> b\n> c")), None);
        // one entry is not a menu
        assert_eq!(
            parse_choices(&rows("Really?\n❯ 1. Yes\nEsc to cancel")),
            None
        );
    }

    #[test]
    fn menus_under_a_tall_windows_blank_rows_are_found() {
        for (file, prompt) in [
            (
                include_str!("../testdata/prompts/claude-edit.txt"),
                "Do you want to make this edit to app.txt?",
            ),
            (
                include_str!("../testdata/prompts/claude-trust.txt"),
                "Is this a project you created or one you trust?",
            ),
        ] {
            for height in [60, 80] {
                let mut screen = rows(file);
                screen.resize(height, String::new());
                let c = parse_choices(&screen)
                    .unwrap_or_else(|| panic!("a menu on {height} rows: {prompt}"));
                assert!(c.prompt.contains(prompt), "{}", c.prompt);
            }
        }
    }

    #[test]
    fn the_selector_below_a_wrapped_entry_keeps_every_entry() {
        let screen = rows(
            " Proceed?\n   1. Yes, and switch to accept edits for\n      this session\n ❯ 2. No\n   3. Cancel\n Esc to cancel",
        );
        let c = parse_choices(&screen).expect("a menu");
        assert_eq!(
            c.choices,
            [
                "Yes, and switch to accept edits for this session",
                "No",
                "Cancel"
            ]
        );
        assert_eq!((c.selected, c.numbered), (1, true));
        assert_eq!(c.prompt, "Proceed?");
        // a deeper row above the first entry is not part of the menu
        let screen = rows(" Proceed?\n      stray\n ❯ 1. Yes\n   2. No\n Esc to cancel");
        let c = parse_choices(&screen).expect("a menu");
        assert_eq!(c.choices, ["Yes", "No"]);
    }

    #[test]
    fn passwords_for_every_program_menus_only_from_an_agents_screen() {
        assert_eq!(
            waiting(true, "[sudo] password for me: ", &[]),
            Some(Waiting::Password {
                prompt: "[sudo] password for me:".into()
            })
        );
        // echo off with nothing printed yet (read -s without a prompt)
        assert_eq!(
            waiting(true, "", &[]),
            Some(Waiting::Password {
                prompt: "Password:".into()
            })
        );
        // a session whose termios is unknown (an older service): the wording
        assert_eq!(
            waiting(false, "Enter passphrase for key '/k': ", &[]),
            Some(Waiting::Password {
                prompt: "Enter passphrase for key '/k':".into()
            })
        );
        assert_eq!(waiting(false, "Compiling rift v0.1.0", &[]), None);
        // a password wins over a menu also on screen
        let menu = rows("Q?\n❯ 1. Yes\n  2. No\nEsc to cancel");
        assert!(matches!(
            waiting(true, "Password:", &menu),
            Some(Waiting::Password { .. })
        ));
        assert!(matches!(
            waiting(false, "", &menu),
            Some(Waiting::Choice(_))
        ));
        // a program other than an agent: no rows, no menu
        assert_eq!(waiting(false, "", &[]), None);
    }

    #[test]
    fn everything_is_bounded() {
        let long = "x".repeat(500);
        let screen = rows(&format!("{long}?\n❯ 1. {long}\n  2. b\nEsc to cancel"));
        let c = parse_choices(&screen).unwrap();
        assert_eq!(c.prompt.chars().count(), MAX_PROMPT_CHARS);
        assert_eq!(c.choices[0].chars().count(), MAX_CHOICE_CHARS);
        let many: String = (1..=12).map(|i| format!("  {i}. c{i}\n")).collect();
        assert_eq!(
            parse_choices(&rows(&format!("Q?\n❯{}Esc to cancel", &many[1..]))),
            None
        );
        let ctx: String = (0..30).map(|i| format!("line {i}\n")).collect();
        let c = parse_choices(&rows(&format!("{ctx}Q?\n❯ 1. a\n  2. b\nEsc to cancel"))).unwrap();
        assert_eq!(c.context.len(), MAX_CONTEXT_LINES);
        assert_eq!(c.context.last().unwrap(), "line 29");
    }
}
