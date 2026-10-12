//! Small Markdown reader for the file viewer's *Rendered* mode: block
//! structure (headings, paragraphs, lists, quotes, fenced code, rules,
//! tables) and inline styles (bold, italic, code, strikethrough, links).
//! Not a full CommonMark implementation — anything it does not recognise
//! stays as plain paragraph text, so nothing is ever hidden.

/// Inline style of a run of text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// Column alignment of a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    /// A list item; `marker` is `•` or the item's number (`3.`).
    Item {
        depth: usize,
        marker: String,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    Code(Vec<String>),
    Rule,
    Table {
        align: Vec<Align>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
}

/// Markdown file by extension.
pub fn is_markdown(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdown" | "mkd" | "mdx"
        )
    })
}

fn fence(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let t = line.trim_start();
    let level = t.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) || line.len() - t.len() > 3 {
        return None;
    }
    let rest = &t[level..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    // closing #s are decoration
    let text = rest.trim().trim_end_matches('#').trim_end();
    Some((level as u8, text))
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|c| t.chars().all(|x| x.to_string() == *c))
}

/// `(indent, marker, text)` of a list item line.
fn list_item(line: &str) -> Option<(usize, String, &str)> {
    let indent = line.len() - line.trim_start().len();
    let t = line.trim_start();
    if let Some(rest) = t
        .strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .or_else(|| t.strip_prefix("+ "))
    {
        // task list boxes
        let rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix("[ ] ") {
            return Some((indent, "☐".into(), r));
        }
        if let Some(r) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            return Some((indent, "☑".into(), r));
        }
        return Some((indent, "•".into(), rest));
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let after = &t[digits..];
        if let Some(rest) = after
            .strip_prefix(". ")
            .or_else(|| after.strip_prefix(") "))
        {
            return Some((indent, format!("{}.", &t[..digits]), rest.trim_start()));
        }
    }
    None
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cur.push('|');
                chars.next();
            }
            '`' => {
                in_code = !in_code;
                cur.push(c);
            }
            '|' if !in_code => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn table_separator(line: &str) -> Option<Vec<Align>> {
    if !line.contains('-') || !line.trim_start().starts_with(['|', ':', '-']) {
        return None;
    }
    table_cells(line)
        .iter()
        .map(|c| {
            let inner = c.trim_matches(':');
            if inner.is_empty() || !inner.chars().all(|x| x == '-') {
                return None;
            }
            Some(match (c.starts_with(':'), c.ends_with(':')) {
                (true, true) => Align::Center,
                (false, true) => Align::Right,
                _ => Align::Left,
            })
        })
        .collect()
}

/// Paragraph text from source lines: soft breaks become spaces, a line
/// ending in two spaces or `\` breaks the line.
fn join(lines: &[&str]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        let hard = l.ends_with("  ") || l.ends_with('\\');
        let l = l.trim();
        let l = l.strip_suffix('\\').unwrap_or(l);
        out.push_str(l);
        if i + 1 < lines.len() {
            out.push(if hard { '\n' } else { ' ' });
        }
    }
    out
}

/// Parse a document into blocks.
pub fn parse(lines: &[String]) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut i = 0;
    let starts_block = |l: &str| {
        l.trim().is_empty()
            || fence(l).is_some()
            || heading(l).is_some()
            || is_rule(l)
            || list_item(l).is_some()
            || l.trim_start().starts_with('>')
    };
    while i < lines.len() {
        let line = lines[i].as_str();
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some(f) = fence(line) {
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(f) {
                code.push(lines[i].clone());
                i += 1;
            }
            i += 1; // closing fence (or end of file)
            blocks.push(Block::Code(code));
            continue;
        }
        if let Some((level, text)) = heading(line) {
            blocks.push(Block::Heading {
                level,
                spans: inline(text),
            });
            i += 1;
            continue;
        }
        if is_rule(line) {
            blocks.push(Block::Rule);
            i += 1;
            continue;
        }
        // indented code (4 spaces) outside lists
        if line.starts_with("    ") && list_item(line).is_none() {
            let mut code = Vec::new();
            while i < lines.len() && (lines[i].starts_with("    ") || lines[i].trim().is_empty()) {
                code.push(lines[i].get(4..).unwrap_or("").to_string());
                i += 1;
            }
            while code.last().is_some_and(|l| l.trim().is_empty()) {
                code.pop();
            }
            blocks.push(Block::Code(code));
            continue;
        }
        if let Some((indent, marker, text)) = list_item(line) {
            let mut parts = vec![text];
            i += 1;
            // lazy continuation lines belong to the item
            while i < lines.len() && !starts_block(&lines[i]) {
                parts.push(lines[i].trim());
                i += 1;
            }
            blocks.push(Block::Item {
                depth: indent / 2,
                marker,
                spans: inline(&join(&parts)),
            });
            continue;
        }
        if line.trim_start().starts_with('>') {
            let mut parts = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let t = lines[i].trim_start()[1..].trim_start();
                parts.push(t);
                i += 1;
            }
            blocks.push(Block::Quote(inline(&join(&parts))));
            continue;
        }
        if line.contains('|')
            && let Some(align) = lines.get(i + 1).and_then(|l| table_separator(l))
        {
            let header: Vec<Vec<Span>> = table_cells(line).iter().map(|c| inline(c)).collect();
            if header.len() == align.len() {
                i += 2;
                let mut rows = Vec::new();
                while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                    let mut row: Vec<Vec<Span>> =
                        table_cells(&lines[i]).iter().map(|c| inline(c)).collect();
                    row.resize(align.len(), Vec::new());
                    rows.push(row);
                    i += 1;
                }
                blocks.push(Block::Table {
                    align,
                    header,
                    rows,
                });
                continue;
            }
        }
        // paragraph: until a blank line or another block starts
        let mut parts = vec![line];
        i += 1;
        while i < lines.len() && !starts_block(&lines[i]) {
            if lines
                .get(i + 1)
                .is_some_and(|l| table_separator(l).is_some())
                && lines[i].contains('|')
            {
                break;
            }
            parts.push(lines[i].as_str());
            i += 1;
        }
        blocks.push(Block::Paragraph(inline(&join(&parts))));
    }
    blocks
}

fn push(spans: &mut Vec<Span>, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match spans.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => spans.push(Span {
            text: text.to_string(),
            style,
        }),
    }
}

/// Inline spans of one block's text.
pub fn inline(s: &str) -> Vec<Span> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|c| c.1);
    let byte = |i: usize| chars.get(i).map_or(s.len(), |c| c.0);
    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();
    let mut i = 0;
    // a closing `delim` (not preceded by whitespace) somewhere after `from`
    let closes = |delim: &str, from: usize| {
        let rest = &s[byte(from)..];
        let mut search = 0;
        while let Some(p) = rest[search..].find(delim) {
            let p = search + p;
            let before = rest[..p].chars().last();
            if p > 0 && before.is_some_and(|c| !c.is_whitespace()) {
                return true;
            }
            search = p + delim.len();
        }
        false
    };
    while i < chars.len() {
        let c = chars[i].1;
        // escapes
        if c == '\\' && at(i + 1).is_some_and(|n| n.is_ascii_punctuation()) {
            text.push(at(i + 1).unwrap_or('\\'));
            i += 2;
            continue;
        }
        // code spans: a run of backticks up to the same run
        if c == '`' {
            let run = chars[i..].iter().take_while(|c| c.1 == '`').count();
            let delim = "`".repeat(run);
            let start = byte(i + run);
            if let Some(end) = s[start..].find(&delim) {
                push(&mut spans, &std::mem::take(&mut text), style);
                let code = &s[start..start + end];
                let code = if code.starts_with(' ') && code.ends_with(' ') && code.len() > 2 {
                    &code[1..code.len() - 1]
                } else {
                    code
                };
                push(
                    &mut spans,
                    code,
                    Style {
                        code: true,
                        ..style
                    },
                );
                let end_byte = start + end + delim.len();
                i = chars.partition_point(|c| c.0 < end_byte);
                continue;
            }
        }
        // links and images: [text](url), ![alt](url)
        let img = c == '!' && at(i + 1) == Some('[');
        if c == '[' || img {
            let open = if img { i + 1 } else { i };
            let from = byte(open + 1);
            if let Some(close) = s[from..].find("](") {
                let label_end = from + close;
                if let Some(paren) = s[label_end + 2..].find(')') {
                    push(&mut spans, &std::mem::take(&mut text), style);
                    for sp in inline(&s[from..label_end]) {
                        push(
                            &mut spans,
                            &sp.text,
                            Style {
                                link: true,
                                ..sp.style
                            },
                        );
                    }
                    let end_byte = label_end + 2 + paren + 1;
                    i = chars.partition_point(|c| c.0 < end_byte);
                    continue;
                }
            }
        }
        // autolinks: <https://…>
        if c == '<'
            && let Some(close) = s[byte(i + 1)..].find('>')
        {
            let inner = &s[byte(i + 1)..byte(i + 1) + close];
            if inner.starts_with("http://") || inner.starts_with("https://") {
                push(&mut spans, &std::mem::take(&mut text), style);
                push(
                    &mut spans,
                    inner,
                    Style {
                        link: true,
                        ..style
                    },
                );
                let end_byte = byte(i + 1) + close + 1;
                i = chars.partition_point(|c| c.0 < end_byte);
                continue;
            }
        }
        // emphasis delimiters
        let double = at(i + 1) == Some(c);
        if (c == '*' || c == '_' || c == '~') && (double || c != '~') {
            let len = if double { 2 } else { 1 };
            let delim: String = std::iter::repeat_n(c, len).collect();
            let prev = i.checked_sub(1).and_then(at);
            let next = at(i + len);
            // `_` inside a word (snake_case) is text
            let intraword = c == '_'
                && prev.is_some_and(char::is_alphanumeric)
                && next.is_some_and(char::is_alphanumeric);
            let on = match (c, len) {
                ('~', _) => style.strike,
                (_, 2) => style.bold,
                _ => style.italic,
            };
            let opens = !on
                && !intraword
                && next.is_some_and(|n| !n.is_whitespace())
                && closes(&delim, i + len);
            let shuts = on && !intraword && prev.is_some_and(|p| !p.is_whitespace());
            if opens || shuts {
                push(&mut spans, &std::mem::take(&mut text), style);
                match (c, len) {
                    ('~', _) => style.strike = !on,
                    (_, 2) => style.bold = !on,
                    _ => style.italic = !on,
                }
                i += len;
                continue;
            }
        }
        text.push(c);
        i += 1;
    }
    push(&mut spans, &text, style);
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(s: &str) -> Vec<Block> {
        parse(&s.lines().map(String::from).collect::<Vec<_>>())
    }

    fn plain(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn blocks() {
        let b = doc("# Agent guide — Rift\n\nSome text\ncontinues here.\n\n\
             - one\n  - nested `code`\n1. first\n> quoted\n> more\n\n---\n\
             ```sh\ncargo run\n```\n");
        assert!(
            matches!(&b[0], Block::Heading { level: 1, spans } if plain(spans) == "Agent guide — Rift")
        );
        assert!(matches!(&b[1], Block::Paragraph(s) if plain(s) == "Some text continues here."));
        assert!(matches!(&b[2], Block::Item { depth: 0, marker, .. } if marker == "•"));
        assert!(matches!(&b[3], Block::Item { depth: 1, spans, .. } if spans[1].style.code));
        assert!(matches!(&b[4], Block::Item { marker, .. } if marker == "1."));
        assert!(matches!(&b[5], Block::Quote(s) if plain(s) == "quoted more"));
        assert_eq!(b[6], Block::Rule);
        assert_eq!(b[7], Block::Code(vec!["cargo run".into()]));
        assert_eq!(b.len(), 8);
    }

    #[test]
    fn tables() {
        let b = doc("| File | Holds |\n|---|:--:|\n| `README.md` | What \\| why |\n| x |\n");
        let Block::Table {
            align,
            header,
            rows,
        } = &b[0]
        else {
            panic!("{b:?}")
        };
        assert_eq!(align, &[Align::Left, Align::Center]);
        assert_eq!(plain(&header[1]), "Holds");
        assert!(rows[0][0][0].style.code);
        assert_eq!(plain(&rows[0][1]), "What | why");
        assert_eq!(rows[1].len(), 2, "short rows pad to the column count");
        // a lone pipe is paragraph text
        assert!(matches!(&doc("a | b\nc")[0], Block::Paragraph(_)));
    }

    #[test]
    fn inline_styles() {
        let s =
            inline("**bold** and *it* and ~~gone~~ [link **x**](http://a) `a*b*` snake_case_name");
        assert!(s.iter().any(|x| x.text == "bold" && x.style.bold));
        assert!(s.iter().any(|x| x.text == "it" && x.style.italic));
        assert!(s.iter().any(|x| x.text == "gone" && x.style.strike));
        assert!(
            s.iter()
                .any(|x| x.text == "x" && x.style.link && x.style.bold)
        );
        assert!(s.iter().any(|x| x.text == "a*b*" && x.style.code));
        assert!(plain(&s).ends_with("snake_case_name"));
        assert!(!s.iter().any(|x| x.text.contains("case") && x.style.italic));
        // unmatched delimiters stay literal
        assert_eq!(
            plain(&inline("2 * 3 = 6 and a ** b")),
            "2 * 3 = 6 and a ** b"
        );
        assert_eq!(plain(&inline(r"\*not\*")), "*not*");
        assert_eq!(plain(&inline("<https://x.y>")), "https://x.y");
    }

    #[test]
    fn markdown_extensions() {
        assert!(is_markdown(std::path::Path::new("AGENTS.md")));
        assert!(is_markdown(std::path::Path::new("x.MARKDOWN")));
        assert!(!is_markdown(std::path::Path::new("main.rs")));
    }
}
