//! Block output in other formats: HTML with the theme's colors ("Copy as
//! HTML"), Markdown ("Copy as Markdown"), and the texts a multi-block
//! selection copies. Output is untrusted: HTML escapes every `<`, `>`, `&`
//! and quote, Markdown fences it with a fence longer than any backtick run
//! inside, and nothing here interprets it. Every copy and export takes the
//! masker in force (`mask_secrets`): secrets leave masked, as shown.

use rift_core::secrets::Masker;
use rift_remote::protocol::Run;

/// `text` with its secrets masked by `m` (masking off: as it is).
pub fn masked(text: String, m: Option<&Masker>) -> String {
    match m.map(|m| m.mask(&text)) {
        Some(std::borrow::Cow::Owned(s)) => s,
        _ => text,
    }
}

/// Styled lines with their secrets masked: matched on the whole text (a
/// token split over runs of different colours, a key over several lines),
/// each masked char replaced in its own run. Masking keeps the char count.
pub fn mask_runs(lines: &mut [Vec<Run>], m: Option<&Masker>) {
    let Some(m) = m else {
        return;
    };
    let joined: String = lines
        .iter()
        .map(|l| l.iter().map(|r| r.text.as_str()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let std::borrow::Cow::Owned(out) = m.mask(&joined) else {
        return;
    };
    let mut chars = out.chars();
    for (i, line) in lines.iter_mut().enumerate() {
        if i > 0 {
            chars.next(); // the line break
        }
        for run in line.iter_mut() {
            let n = run.text.chars().count();
            run.text = chars.by_ref().take(n).collect();
        }
    }
}

/// The plain text beside Copy as HTML's: the command, then the output.
pub fn plain(command: &str, output: &str, m: Option<&Masker>) -> String {
    let text = if command.is_empty() {
        output.to_string()
    } else {
        format!("{command}\n{output}")
    };
    masked(text, m)
}

/// `s` with HTML's special characters escaped (text and attribute safe).
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A block as HTML: a monospace `<pre>` in the theme's foreground and
/// background, the command first, then each output line's runs as styled
/// spans. `fg` / `bg` are `rrggbb`; `trimmed` = output rows gone from
/// scrollback before the lines (said in a note line).
pub fn html(
    command: &str,
    lines: &[Vec<Run>],
    trimmed: u64,
    (fg, bg): (&str, &str),
    m: Option<&Masker>,
) -> String {
    let command = masked(command.to_string(), m);
    let mut lines = lines.to_vec();
    mask_runs(&mut lines, m);
    let mut out = format!(
        "<pre style=\"font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; \
         color: #{fg}; background-color: #{bg}; padding: 8px; white-space: pre-wrap;\">"
    );
    if !command.is_empty() {
        out.push_str(&format!("<b>$ {}</b>\n", escape_html(&command)));
    }
    if trimmed > 0 {
        out.push_str(&format!("<i>… ({trimmed} lines trimmed)</i>\n"));
    }
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for run in line {
            let mut style = String::new();
            if let Some(c) = &run.fg {
                style.push_str(&format!("color: #{};", hex_only(c)));
            }
            if let Some(c) = &run.bg {
                style.push_str(&format!("background-color: #{};", hex_only(c)));
            }
            if run.flags & Run::BOLD != 0 {
                style.push_str("font-weight: bold;");
            }
            if run.flags & Run::ITALIC != 0 {
                style.push_str("font-style: italic;");
            }
            if run.flags & Run::DIM != 0 {
                style.push_str("opacity: 0.6;");
            }
            match (
                run.flags & Run::UNDERLINE != 0,
                run.flags & Run::STRIKE != 0,
            ) {
                (true, true) => style.push_str("text-decoration: underline line-through;"),
                (true, false) => style.push_str("text-decoration: underline;"),
                (false, true) => style.push_str("text-decoration: line-through;"),
                _ => {}
            }
            let text = escape_html(&run.text);
            if style.is_empty() {
                out.push_str(&text);
            } else {
                out.push_str(&format!("<span style=\"{style}\">{text}</span>"));
            }
        }
    }
    out.push_str("</pre>");
    out
}

/// A colour as hex digits only (whatever a run carries, nothing else can
/// reach the style attribute).
fn hex_only(c: &str) -> String {
    c.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(6)
        .collect()
}

/// A Markdown code fence longer than every backtick run in `text`.
fn fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// A block as Markdown: the command in a `sh` fence, then the output in a
/// plain fence, with an honest note when its start was trimmed from
/// scrollback (`trimmed` lines) or its end cut (`cut`).
pub fn markdown(
    command: &str,
    output: &str,
    trimmed: u64,
    cut: bool,
    m: Option<&Masker>,
) -> String {
    let command = masked(command.to_string(), m);
    let output = masked(output.to_string(), m);
    let (command, output) = (command.as_str(), output.as_str());
    let mut out = String::new();
    if !command.is_empty() {
        let f = fence(command);
        out.push_str(&format!("{f}sh\n{command}\n{f}\n\n"));
    }
    let mut body = String::new();
    if trimmed > 0 {
        body.push_str(&format!("… ({trimmed} lines trimmed)\n"));
    }
    body.push_str(output.strip_suffix('\n').unwrap_or(output));
    if cut {
        body.push_str("\n… (rest of the output not copied)");
    }
    let f = fence(&body);
    out.push_str(&format!("{f}\n{body}\n{f}\n"));
    out
}

/// What a multi-block selection copies, oldest block first: commands one
/// per line; outputs, or each command with its output, separated by a
/// blank line.
pub fn join_commands(cmds: &[String], m: Option<&Masker>) -> String {
    masked(cmds.join("\n"), m)
}

pub fn join_blocks(parts: &[String], m: Option<&Masker>) -> String {
    let text = parts
        .iter()
        .map(|p| p.strip_suffix('\n').unwrap_or(p))
        .collect::<Vec<_>>()
        .join("\n\n");
    masked(text, m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, fg: Option<&str>, flags: u8) -> Run {
        Run {
            text: text.into(),
            fg: fg.map(str::to_string),
            bg: None,
            flags,
        }
    }

    #[test]
    fn html_escapes_and_colours() {
        let lines = vec![
            vec![run("<b>&\"'", None, 0)],
            vec![run("ok", Some("b4fa72"), Run::BOLD), run(" plain", None, 0)],
        ];
        let h = html("echo <x> && y", &lines, 0, ("ffffff", "050505"), None);
        assert!(
            h.starts_with("<pre style=\"font-family: ui-monospace"),
            "{h}"
        );
        assert!(
            h.contains("color: #ffffff; background-color: #050505"),
            "{h}"
        );
        assert!(h.contains("<b>$ echo &lt;x&gt; &amp;&amp; y</b>\n"), "{h}");
        assert!(h.contains("&lt;b&gt;&amp;&quot;&#39;\n"), "{h}");
        assert!(
            h.contains("<span style=\"color: #b4fa72;font-weight: bold;\">ok</span> plain</pre>"),
            "{h}"
        );
        // no raw tag from the output survives
        assert!(!h.contains("<b>&"), "{h}");
    }

    #[test]
    fn html_colour_values_cannot_break_out() {
        let lines = vec![vec![run("x", Some("\"><script>"), 0)]];
        let h = html("", &lines, 3, ("fff", "000"), None);
        // only its hex digits reach the style attribute
        assert!(h.contains("<span style=\"color: #c;\">x</span>"), "{h}");
        assert!(h.contains("<i>… (3 lines trimmed)</i>"), "{h}");
    }

    #[test]
    fn markdown_fences_and_trim_note() {
        let md = markdown("ls", "a\nb\n", 0, false, None);
        assert_eq!(md, "```sh\nls\n```\n\n```\na\nb\n```\n");
        // backticks inside need a longer fence
        let md = markdown("echo '```'", "```\ncode\n````", 12, false, None);
        assert!(md.starts_with("````sh\necho '```'\n````\n\n"), "{md}");
        assert!(
            md.contains("`````\n… (12 lines trimmed)\n```\ncode\n````\n`````\n"),
            "{md}"
        );
        assert!(markdown("x", "y", 0, true, None).contains("… (rest of the output not copied)"));
    }

    /// `mask_secrets`: no copy or export path lets a secret out (multi-block
    /// copies, Copy as HTML and its plain text, Copy as Markdown, and the
    /// texts Save output / Export buffer / Compare take through `masked`).
    #[test]
    fn no_copy_or_export_leaks_a_secret() {
        // a fake token, built from parts: never issued
        let token = format!("ghp_{}", "FAKE0123".repeat(5));
        let m = Masker::new(&[]).unwrap();
        let m = Some(&m);
        let cmd = format!("echo {token}");
        let out = format!("key {token} end\n");
        // the token split over two coloured runs is still one secret
        let (a, b) = token.split_at(10);
        let lines = vec![vec![
            run("key ", None, 0),
            run(a, Some("ff0000"), 0),
            run(b, None, Run::BOLD),
        ]];
        let texts = [
            (
                "copy commands",
                join_commands(&[cmd.clone(), "ls".into()], m),
            ),
            ("copy outputs", join_blocks(&[out.clone(), "x".into()], m)),
            (
                "copy both",
                join_blocks(&[format!("{cmd}\n{out}"), "x".into()], m),
            ),
            ("html", html(&cmd, &lines, 0, ("fff", "000"), m)),
            ("html plain", plain(&cmd, &out, m)),
            ("markdown", markdown(&cmd, &out, 0, false, m)),
            ("save / export / compare", masked(out.clone(), m)),
        ];
        let tail = &token[token.len() - 8..];
        for (path, text) in &texts {
            assert!(!text.contains(tail), "{path} leaks: {text}");
            assert!(text.contains('•'), "{path} masks nothing: {text}");
        }
        // masking off: as printed
        assert!(markdown(&cmd, &out, 0, false, None).contains(&token));
        assert!(html(&cmd, &lines, 0, ("fff", "000"), None).contains(b));
    }

    #[test]
    fn multi_selection_texts() {
        assert_eq!(join_commands(&["a".into(), "b".into()], None), "a\nb");
        assert_eq!(
            join_blocks(&["a\n1\n".into(), "b\n2".into()], None),
            "a\n1\n\nb\n2"
        );
    }
}
