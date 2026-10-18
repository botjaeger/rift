//! "Send to agent…": code from the review panel (a hunk, a file's hunks)
//! or the file viewer (the selected lines) pasted into another pane — a
//! coding agent's, usually — as `path:start-end` and a fenced block. The
//! target is picked in the palette (`PaletteMode::SendTarget`); the text is
//! pasted, never followed by Enter.

use std::path::{Path, PathBuf};

/// Code waiting for its target pane.
#[derive(Debug, Clone, PartialEq)]
pub struct SendSource {
    /// The file, absolute.
    pub path: PathBuf,
    /// One or more ranges (1-based, inclusive) with their text.
    pub parts: Vec<SendPart>,
    /// The pane that was active when it was picked (labelled in the list).
    pub from_pane: Option<rift_pty::PaneId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SendPart {
    pub start: u32,
    pub end: u32,
    pub body: String,
    /// The diff line cap cut the code: `end` is the last line kept, and the
    /// header says `(truncated)`.
    pub truncated: bool,
}

/// The fence's language for `path` (Markdown info strings); empty when the
/// extension is not a known one.
pub fn fence_lang(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "rb" => "ruby",
        "sh" | "bash" | "zsh" => "bash",
        "fish" => "fish",
        "md" | "markdown" => "markdown",
        "json" => "json",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "m" => "objective-c",
        "cs" => "csharp",
        "php" => "php",
        "css" => "css",
        "scss" => "scss",
        "html" | "htm" => "html",
        "xml" => "xml",
        "sql" => "sql",
        "lua" => "lua",
        "wgsl" => "wgsl",
        "diff" | "patch" => "diff",
        _ => "",
    }
}

/// `path` as the target pane should read it: relative to its directory
/// when the file is under it, else absolute.
pub fn shown_path(path: &Path, target_cwd: Option<&Path>) -> String {
    target_cwd
        .and_then(|cwd| path.strip_prefix(cwd).ok())
        .filter(|rel| !rel.as_os_str().is_empty())
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// The pasted text: per part `path:start-end`, then the code in a fence
/// longer than any backtick run inside it; parts separated by a blank
/// line. No trailing newline (the paste never presses Enter).
pub fn format(source: &SendSource, target_cwd: Option<&Path>) -> String {
    let path = shown_path(&source.path, target_cwd);
    let lang = fence_lang(&source.path);
    let parts: Vec<String> = source
        .parts
        .iter()
        .map(|p| {
            let longest = p.body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            let body = p.body.trim_end_matches('\n');
            let cut = if p.truncated { " (truncated)" } else { "" };
            format!(
                "{path}:{}-{}{cut}\n{fence}{lang}\n{body}\n{fence}",
                p.start, p.end
            )
        })
        .collect();
    parts.join("\n\n")
}

/// A review hunk as a part: its new-side line range, and its lines with
/// their ` ` / `+` / `-` marks.
pub fn hunk_part(h: &rift_core::review::Hunk) -> SendPart {
    use rift_core::review::LineKind;
    let start = h.id.new_start.max(1);
    // a hunk the line cap cut ends at its last kept new-side line, not at
    // the end its header names
    let end = if h.partial {
        h.lines
            .iter()
            .filter_map(|l| l.new_no)
            .max()
            .unwrap_or(start)
    } else {
        h.id.new_start + h.id.new_lines.saturating_sub(1)
    }
    .max(start);
    let mut body = String::new();
    for l in &h.lines {
        body.push(match l.kind {
            LineKind::Added => '+',
            LineKind::Removed => '-',
            LineKind::Context => ' ',
        });
        body.push_str(&l.text);
        body.push('\n');
    }
    SendPart {
        start,
        end,
        body,
        truncated: h.partial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rift_core::review::{Hunk, HunkId, Line, LineKind};

    fn line(kind: LineKind, text: &str) -> Line {
        Line {
            kind,
            old_no: None,
            new_no: None,
            text: text.into(),
            emph: None,
        }
    }

    #[test]
    fn formats_a_hunk_as_a_path_range_and_a_fenced_block_without_a_trailing_newline() {
        let h = Hunk {
            id: HunkId {
                old_start: 4,
                old_lines: 3,
                new_start: 4,
                new_lines: 3,
                content_hash: 0,
            },
            header: "@@ -4,3 +4,3 @@".into(),
            lines: vec![
                line(LineKind::Context, "fn main() {"),
                line(LineKind::Removed, "    old();"),
                line(LineKind::Added, "    new();"),
                line(LineKind::Context, "}"),
            ],
            partial: false,
        };
        let src = SendSource {
            path: "/repo/src/main.rs".into(),
            parts: vec![hunk_part(&h)],
            from_pane: None,
        };
        let text = format(&src, Some(Path::new("/repo")));
        assert_eq!(
            text,
            "src/main.rs:4-6\n```rust\n fn main() {\n-    old();\n+    new();\n }\n```"
        );
        assert!(!text.ends_with('\n') && !text.ends_with('\r'));
        // a target elsewhere gets the absolute path
        assert!(format(&src, Some(Path::new("/other"))).starts_with("/repo/src/main.rs:4-6\n"));
    }

    #[test]
    fn the_fence_outgrows_backticks_in_the_code_and_parts_are_separated() {
        let src = SendSource {
            path: "/r/README".into(),
            parts: vec![
                SendPart {
                    start: 1,
                    end: 1,
                    body: "````text````".into(),
                    truncated: false,
                },
                SendPart {
                    start: 9,
                    end: 10,
                    body: "a\nb\n".into(),
                    truncated: false,
                },
            ],
            from_pane: None,
        };
        assert_eq!(
            format(&src, None),
            "/r/README:1-1\n`````\n````text````\n`````\n\n/r/README:9-10\n```\na\nb\n```"
        );
        assert_eq!(fence_lang(Path::new("x.TSX")), "tsx");
        // a pure deletion at the top of a file still names line 1
        let deletion = Hunk {
            id: HunkId {
                old_start: 1,
                old_lines: 1,
                new_start: 0,
                new_lines: 0,
                content_hash: 0,
            },
            header: String::new(),
            lines: vec![line(LineKind::Removed, "gone")],
            partial: false,
        };
        let p = hunk_part(&deletion);
        assert_eq!((p.start, p.end, p.body.as_str()), (1, 1, "-gone\n"));
    }

    #[test]
    fn a_hunk_the_line_cap_cut_names_its_last_kept_line_and_says_truncated() {
        let mut kept: Vec<Line> = (1..=3)
            .map(|n| line(LineKind::Added, &format!("big line {n}")))
            .collect();
        for (i, l) in kept.iter_mut().enumerate() {
            l.new_no = Some(i as u32 + 1);
        }
        let h = Hunk {
            id: HunkId {
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 5000,
                content_hash: 0,
            },
            header: "@@ -0,0 +1,5000 @@".into(),
            lines: kept,
            partial: true,
        };
        let src = SendSource {
            path: "/r/big.txt".into(),
            parts: vec![hunk_part(&h)],
            from_pane: None,
        };
        assert!(
            format(&src, Some(Path::new("/r"))).starts_with("big.txt:1-3 (truncated)\n```\n"),
            "{}",
            format(&src, Some(Path::new("/r")))
        );
    }
}
