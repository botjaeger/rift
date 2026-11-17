//! Small Markdown reader for the file viewer's *Rendered* mode: block
//! structure (headings, paragraphs, lists, quotes, fenced code, rules,
//! tables) and inline styles (bold, italic, code, strikethrough, links).
//! HTML a README uses for layout (`<p align="center">`, `<h1>`, `<img>`,
//! `<b>`, `<br>`, comments) is read the way GitHub shows it: its tags are
//! not shown, headings and paragraphs keep their centring, and an image
//! shows its alt text as a Markdown image does.
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
    /// `center`: inside an HTML element centred by `align="center"` (or
    /// `<center>`).
    Heading {
        level: u8,
        spans: Vec<Span>,
        center: bool,
    },
    Paragraph {
        spans: Vec<Span>,
        center: bool,
    },
    /// A list item; `marker` is `•` or the item's number (`3.`).
    Item {
        depth: usize,
        marker: String,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    /// A fenced or indented code block; `lang` is the fence's info word
    /// (`sh`, `mermaid`), lowercased, empty when it has none.
    Code {
        lang: String,
        lines: Vec<String>,
    },
    Rule,
    Table {
        align: Vec<Align>,
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
    },
}

/// Info words of fenced blocks a shell runs.
const SHELL_LANGS: [&str; 7] = [
    "sh",
    "bash",
    "zsh",
    "shell",
    "fish",
    "console",
    "shell-session",
];

/// A code block's command for the viewer's Run button: the block of a
/// shell language as it stands, or for a session transcript (`console`,
/// `shell-session`) its `$ ` lines without the prompt (the other lines are
/// output). `None` for any other language, or nothing to run.
pub fn shell_command(lang: &str, lines: &[String]) -> Option<String> {
    if !SHELL_LANGS.contains(&lang) {
        return None;
    }
    let lines: Vec<&str> = if matches!(lang, "console" | "shell-session") {
        lines
            .iter()
            .filter_map(|l| {
                let t = l.trim_start();
                t.strip_prefix("$ ").or_else(|| (t == "$").then_some(""))
            })
            .collect()
    } else {
        lines.iter().map(String::as_str).collect()
    };
    let text = lines.join("\n");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A Mermaid diagram block (shown as its source: there is no renderer).
pub fn is_mermaid(lang: &str) -> bool {
    lang == "mermaid"
}

/// The Markdown file a command printed (`cat README.md`, `bat docs/x.md`,
/// `less`, `more`, `glow`, `head`, `tail`), for the block's "Open in viewer":
/// one file argument with a Markdown extension, no pipes, redirections or
/// command lists. The path is as typed (relative to the block's directory).
pub fn printed_file(command: &str) -> Option<String> {
    const PRINTERS: [&str; 8] = [
        "cat", "bat", "batcat", "less", "more", "glow", "head", "tail",
    ];
    if command.contains(['|', '>', '<', ';', '&', '`', '$', '\n']) {
        return None;
    }
    // a quoted or escaped `~` is a literal one the shell did not expand,
    // but the tokenizer drops the quoting: leave such a command alone
    if ["'~", "\"~", "\\~"].iter().any(|q| command.contains(q)) {
        return None;
    }
    let words = crate::completion_specs::tokenize(command.trim());
    let (program, args) = words.split_first()?;
    if !PRINTERS.contains(&program.as_str()) {
        return None;
    }
    // `head -n 20 x.md`: the count is not a file
    let mut files: Vec<&String> = Vec::new();
    let mut skip = false;
    for a in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if matches!(program.as_str(), "head" | "tail") && matches!(a.as_str(), "-n" | "-c") {
            skip = true;
        } else if !a.starts_with('-') {
            files.push(a);
        }
    }
    match files.as_slice() {
        [f] if is_markdown(std::path::Path::new(f.as_str())) => Some((*f).clone()),
        _ => None,
    }
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
            || html_start(l) == Some(true)
    };
    // HTML elements still open (`<div align="center">` alone, then
    // Markdown): what they centre
    let mut html = HtmlState::default();
    while i < lines.len() {
        let line = lines[i].as_str();
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some(f) = fence(line) {
            let lang = line
                .trim_start()
                .trim_start_matches(f.chars().next().unwrap_or('`'))
                .split(|c: char| c.is_whitespace() || c == '{' || c == ',')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with(f) {
                code.push(lines[i].clone());
                i += 1;
            }
            i += 1; // closing fence (or end of file)
            blocks.push(Block::Code { lang, lines: code });
            continue;
        }
        if html_start(line).is_some() {
            // an HTML block runs to a blank line; a comment to its `-->`
            let comment = line.trim_start().starts_with("<!--");
            let mut src = Vec::new();
            while i < lines.len() {
                let l = lines[i].as_str();
                if !comment && l.trim().is_empty() {
                    break;
                }
                src.push(l);
                i += 1;
                if comment && l.contains("-->") {
                    break;
                }
            }
            html.read(&src.join("\n"), &mut blocks);
            continue;
        }
        if let Some((level, text)) = heading(line) {
            blocks.push(Block::Heading {
                level,
                spans: inline(text),
                center: html.center(),
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
            blocks.push(Block::Code {
                lang: String::new(),
                lines: code,
            });
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
        blocks.push(Block::Paragraph {
            spans: inline(&join(&parts)),
            center: html.center(),
        });
    }
    blocks
}

/// HTML elements that end the text before them and start a block.
const HTML_BLOCK_TAGS: &[&str] = &[
    "blockquote",
    "center",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "th",
    "thead",
    "tr",
    "ul",
];

/// Inline HTML elements a README line may start with.
const HTML_INLINE_TAGS: &[&str] = &[
    "a", "b", "br", "code", "del", "em", "hr", "i", "img", "kbd", "picture", "s", "source", "span",
    "strike", "strong", "sub", "sup", "tt",
];

/// A line that starts an HTML block (`Some(true)` when its element is a
/// block one, which also ends a paragraph above it): up to 3 spaces, then
/// a comment or a known element's tag.
fn html_start(line: &str) -> Option<bool> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        return None;
    }
    if t.starts_with("<!--") {
        return Some(true);
    }
    let rest = t.strip_prefix('<')?;
    let rest = rest.strip_prefix('/').unwrap_or(rest);
    let n = rest.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let after = &rest[n..];
    if !(after.is_empty() || after.starts_with([' ', '\t', '>', '/'])) {
        return None;
    }
    let name = rest[..n].to_ascii_lowercase();
    if HTML_BLOCK_TAGS.contains(&name.as_str()) {
        Some(true)
    } else {
        HTML_INLINE_TAGS.contains(&name.as_str()).then_some(false)
    }
}

/// One HTML tag at the start of a text.
struct Tag<'a> {
    /// Lowercased.
    name: String,
    closing: bool,
    /// What stands between the name and `>`.
    attrs: &'a str,
    /// Bytes the tag takes.
    len: usize,
}

/// The tag `s` starts with: `<name …>`, `</name>`, `<name/>`.
fn tag_at(s: &str) -> Option<Tag<'_>> {
    let body = s.strip_prefix('<')?;
    let (closing, body) = match body.strip_prefix('/') {
        Some(b) => (true, b),
        None => (false, body),
    };
    if !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let n = body.bytes().take_while(u8::is_ascii_alphanumeric).count();
    let after = &body[n..];
    if !after.starts_with([' ', '\t', '\n', '>', '/']) {
        return None;
    }
    // the closing `>`, outside quoted attribute values
    let mut quote = None;
    for (k, c) in after.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                return Some(Tag {
                    name: body[..n].to_ascii_lowercase(),
                    closing,
                    attrs: after[..k].trim_end_matches('/'),
                    len: s.len() - after.len() + k + 1,
                });
            }
            (None, '<') => return None,
            _ => {}
        }
    }
    None
}

/// The value of attribute `key` (lowercase) in a tag's `attrs`.
fn attr(attrs: &str, key: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find(key) {
        let at = from + p;
        from = at + key.len();
        if at == 0 || !lower.as_bytes()[at - 1].is_ascii_whitespace() {
            continue;
        }
        let Some(rest) = attrs[from..].trim_start().strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = match rest.chars().next() {
            Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
            _ => rest.split(char::is_whitespace).next().unwrap_or(""),
        };
        return Some(entities(value));
    }
    None
}

/// Text with its HTML character references (`&amp;`, `&#169;`, `&#x2014;`)
/// decoded; an unknown one stays as written.
fn entities(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let decoded = rest.find(';').filter(|&e| e <= 10).and_then(|e| {
            let c = match &rest[1..e] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => '\u{a0}',
                "copy" => '©',
                "mdash" => '—',
                "ndash" => '–',
                "hellip" => '…',
                name => {
                    let n = name.strip_prefix('#')?;
                    let v = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => n.parse().ok()?,
                    };
                    char::from_u32(v)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// What an inline HTML element does to the text.
enum InlineHtml {
    /// Turned a style on or off, or nothing to show (`<span>`).
    Styled,
    Break,
    /// An image's alt text.
    Alt(String),
}

/// An inline element's effect (`None`: not an inline element).
fn inline_html(tag: &Tag, style: &mut Style) -> Option<InlineHtml> {
    let on = !tag.closing;
    match tag.name.as_str() {
        "b" | "strong" => style.bold = on,
        "i" | "em" => style.italic = on,
        "code" | "kbd" | "tt" => style.code = on,
        "s" | "del" | "strike" => style.strike = on,
        "a" => style.link = on,
        "br" => return Some(InlineHtml::Break),
        "img" if on => {
            return Some(InlineHtml::Alt(attr(tag.attrs, "alt").unwrap_or_default()));
        }
        "img" | "span" | "sub" | "sup" | "picture" | "source" => {}
        _ => return None,
    }
    Some(InlineHtml::Styled)
}

/// HTML block elements open across the document.
#[derive(Default)]
struct HtmlState {
    /// `(name, centred)`, outermost first.
    open: Vec<(String, bool)>,
}

/// An HTML block's text as it gathers, its whitespace collapsed as a
/// browser does it.
#[derive(Default)]
struct HtmlText {
    spans: Vec<Span>,
    style: Style,
    /// Whitespace waits to be written until text follows it: the style of
    /// the text it stood in.
    gap: Option<Style>,
    started: bool,
}

impl HtmlText {
    fn text(&mut self, s: &str, style: Style) {
        for c in s.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                // the space keeps the style of the text it stood in
                // (`a <code>b`, `</b> c`)
                self.gap.get_or_insert(style);
                continue;
            }
            if let Some(gap) = self.gap
                && self.started
            {
                push(&mut self.spans, " ", gap);
            }
            self.gap = None;
            self.started = true;
            push(&mut self.spans, c.encode_utf8(&mut [0; 4]), style);
        }
    }

    fn line_break(&mut self) {
        push(&mut self.spans, "\n", self.style);
        self.gap = None;
        self.started = false;
    }

    /// The text so far as a block (nothing when it is blank).
    fn flush(&mut self, level: Option<u8>, center: bool, out: &mut Vec<Block>) {
        let mut spans = std::mem::take(&mut self.spans);
        self.gap = None;
        self.started = false;
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start_matches('\n').to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end_matches('\n').to_string();
        }
        spans.retain(|s| !s.text.is_empty());
        if spans.iter().all(|s| s.text.trim().is_empty()) {
            return;
        }
        out.push(match level {
            Some(level) => Block::Heading {
                level,
                spans,
                center,
            },
            None => Block::Paragraph { spans, center },
        });
    }
}

impl HtmlState {
    fn center(&self) -> bool {
        self.open.last().is_some_and(|o| o.1)
    }

    /// Read one HTML block into `out`. Its text shows as written (Markdown
    /// inside an HTML block is not read, as on GitHub); the tags of other
    /// elements are hidden.
    fn read(&mut self, src: &str, out: &mut Vec<Block>) {
        let mut text = HtmlText::default();
        let mut level = None;
        let mut rest = src;
        while !rest.is_empty() {
            if let Some(r) = rest.strip_prefix("<!--") {
                rest = r.find("-->").map_or("", |e| &r[e + 3..]);
                continue;
            }
            if let Some(tag) = tag_at(rest) {
                rest = &rest[tag.len..];
                let name = tag.name.as_str();
                if HTML_BLOCK_TAGS.contains(&name) {
                    text.flush(level, self.center(), out);
                    let heading = name.strip_prefix('h').and_then(|n| n.parse::<u8>().ok());
                    if tag.closing {
                        if let Some(p) = self.open.iter().rposition(|o| o.0 == name) {
                            self.open.truncate(p);
                        }
                        if heading.is_some() {
                            level = None;
                        }
                    } else {
                        let centred = self.center()
                            || name == "center"
                            || attr(tag.attrs, "align")
                                .is_some_and(|a| a.eq_ignore_ascii_case("center"));
                        self.open.push((tag.name.clone(), centred));
                        if heading.is_some() {
                            level = heading;
                        }
                    }
                } else if name == "hr" {
                    text.flush(level, self.center(), out);
                    out.push(Block::Rule);
                } else {
                    let mut style = text.style;
                    match inline_html(&tag, &mut style) {
                        Some(InlineHtml::Break) => text.line_break(),
                        Some(InlineHtml::Alt(alt)) => text.text(
                            &alt,
                            Style {
                                link: true,
                                ..style
                            },
                        ),
                        _ => {}
                    }
                    text.style = style;
                }
                continue;
            }
            let n = rest
                .char_indices()
                .skip(1)
                .find(|c| c.1 == '<')
                .map_or(rest.len(), |c| c.0);
            let style = text.style;
            text.text(&entities(&rest[..n]), style);
            rest = &rest[n..];
        }
        text.flush(level, self.center(), out);
    }
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
        // inline HTML: `<b>`, `<br>`, `<img alt>`, `<kbd>`, comments
        if c == '<' {
            let here = &s[byte(i)..];
            let mut next = style;
            let skip = if let Some(r) = here.strip_prefix("<!--") {
                r.find("-->").map(|e| 4 + e + 3)
            } else {
                tag_at(here).and_then(|tag| {
                    let fx = inline_html(&tag, &mut next)?;
                    push(&mut spans, &std::mem::take(&mut text), style);
                    match fx {
                        InlineHtml::Break => text.push('\n'),
                        InlineHtml::Alt(alt) => {
                            push(&mut spans, &alt, Style { link: true, ..next })
                        }
                        InlineHtml::Styled => {}
                    }
                    Some(tag.len)
                })
            };
            if let Some(len) = skip {
                style = next;
                let end_byte = byte(i) + len;
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
            matches!(&b[0], Block::Heading { level: 1, spans, center: false } if plain(spans) == "Agent guide — Rift")
        );
        assert!(
            matches!(&b[1], Block::Paragraph { spans: s, .. } if plain(s) == "Some text continues here.")
        );
        assert!(matches!(&b[2], Block::Item { depth: 0, marker, .. } if marker == "•"));
        assert!(matches!(&b[3], Block::Item { depth: 1, spans, .. } if spans[1].style.code));
        assert!(matches!(&b[4], Block::Item { marker, .. } if marker == "1."));
        assert!(matches!(&b[5], Block::Quote(s) if plain(s) == "quoted more"));
        assert_eq!(b[6], Block::Rule);
        assert_eq!(
            b[7],
            Block::Code {
                lang: "sh".into(),
                lines: vec!["cargo run".into()]
            }
        );
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
        assert!(matches!(&doc("a | b\nc")[0], Block::Paragraph { .. }));
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
    fn html_reads_as_github_shows_it() {
        let b = doc(concat!(
            "<p align=\"center\">\n",
            "  <img src=\"icons/icon.svg\" alt=\"Logo: two eyes\" width=\"128\">\n",
            "</p>\n\n",
            "<h1 align=\"center\">Lurk</h1>\n\n",
            "<p align=\"center\"><strong>Read it.</strong> Nobody&nbsp;knows &amp; <code>x</code></p>\n\n",
            "<!-- a note\n\nover lines -->\n",
            "Plain text, not centred.\n",
            "<p>A block tag ends it.</p>\n\n",
            "<div align=\"center\">\n\n# Centred title\n\n</div>\n\n",
            "# Left\n",
        ));
        let rows: Vec<(String, bool, Option<u8>)> = b
            .iter()
            .map(|b| match b {
                Block::Heading {
                    level,
                    spans,
                    center,
                } => (plain(spans), *center, Some(*level)),
                Block::Paragraph { spans, center } => (plain(spans), *center, None),
                other => panic!("{other:?}"),
            })
            .collect();
        let row = |t: &str, c: bool, l: Option<u8>| (t.to_string(), c, l);
        assert_eq!(
            rows,
            [
                row("Logo: two eyes", true, None),
                row("Lurk", true, Some(1)),
                row("Read it. Nobody\u{a0}knows & x", true, None),
                row("Plain text, not centred.", false, None),
                row("A block tag ends it.", false, None),
                row("Centred title", true, Some(1)),
                row("Left", false, Some(1)),
            ]
        );
        let Block::Paragraph { spans: s, .. } = &b[2] else {
            panic!("{b:?}")
        };
        assert!(s[0].style.bold && s[0].text == "Read it.");
        assert!(!s[1].style.bold);
        assert!(s.iter().any(|x| x.text == "x" && x.style.code), "{s:?}");
        // an unknown tag or a comparison stays text
        for text in ["<T> is generic", "a <b and c"] {
            assert!(
                matches!(&doc(text)[0], Block::Paragraph { spans, .. } if plain(spans) == text),
                "{text}"
            );
        }
    }

    #[test]
    fn inline_html_in_markdown_text() {
        let s = inline(
            "Press <kbd>⌘K</kbd> <!-- hidden -->to<br>clear <b>now</b> <img src=x alt=\"pic\">",
        );
        assert_eq!(plain(&s), "Press ⌘K to\nclear now pic");
        assert!(s.iter().any(|x| x.text == "⌘K" && x.style.code));
        assert!(s.iter().any(|x| x.text == "now" && x.style.bold));
        assert!(s.iter().any(|x| x.text == "pic" && x.style.link));
        assert_eq!(plain(&inline("Vec<T> and a < b")), "Vec<T> and a < b");
    }

    #[test]
    fn markdown_extensions() {
        assert!(is_markdown(std::path::Path::new("AGENTS.md")));
        assert!(is_markdown(std::path::Path::new("x.MARKDOWN")));
        assert!(!is_markdown(std::path::Path::new("main.rs")));
    }

    #[test]
    fn code_blocks_keep_their_language() {
        let b = doc(
            "```Bash title=x\nls\n```\n~~~mermaid\ngraph TD\n~~~\n```\nplain\n```\n\n    indented\n",
        );
        let langs: Vec<&str> = b
            .iter()
            .map(|b| match b {
                Block::Code { lang, .. } => lang.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(langs, ["bash", "mermaid", "", ""]);
        assert!(is_mermaid("mermaid") && !is_mermaid("bash"));
    }

    #[test]
    fn shell_command_runs_shell_blocks_only() {
        let l = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            shell_command("sh", &l(&["cd /tmp", "ls -la", ""])).as_deref(),
            Some("cd /tmp\nls -la")
        );
        // a session transcript: the prompted lines, not the output
        assert_eq!(
            shell_command("console", &l(&["$ echo hi", "hi", "  $ pwd", "/tmp"])).as_deref(),
            Some("echo hi\npwd")
        );
        assert_eq!(shell_command("console", &l(&["output only"])), None);
        assert_eq!(shell_command("python", &l(&["print(1)"])), None);
        assert_eq!(
            shell_command("", &l(&["ls"])),
            None,
            "no info word: not runnable"
        );
        assert_eq!(shell_command("mermaid", &l(&["graph TD"])), None);
        assert_eq!(shell_command("bash", &l(&["", "  "])), None);
    }

    #[test]
    fn printed_file_finds_one_markdown_argument() {
        assert_eq!(printed_file("cat README.md").as_deref(), Some("README.md"));
        assert_eq!(
            printed_file("bat --paging=never 'docs/my notes.md'").as_deref(),
            Some("docs/my notes.md")
        );
        assert_eq!(
            printed_file("glow ~/x.markdown").as_deref(),
            Some("~/x.markdown")
        );
        assert_eq!(
            printed_file("head -n 20 README.md").as_deref(),
            Some("README.md")
        );
        assert_eq!(printed_file("tail -c 9 a.md").as_deref(), Some("a.md"));
        for no in [
            "cat a.md b.md",
            "cat main.rs",
            "rm README.md",
            "cat README.md | head",
            "cat README.md > out.md",
            "cat $(ls *.md)",
            "cat README.md; rm -rf x",
            "cat",
            // expansions name some other file
            "cat `pick`.md",
            "cat $DIR/x.md",
            // the shell did not expand a quoted `~`
            "cat '~/x.md'",
            "cat \"~/x.md\"",
            "cat \\~/x.md",
        ] {
            assert_eq!(printed_file(no), None, "{no:?}");
        }
    }
}
