//! Delimited text (CSV, TSV) as a table for the file viewer's *Table* view:
//! RFC 4180 fields (quotes, `""` escapes, line breaks inside quotes), rows
//! of uneven length padded, each column as wide as its widest cell up to
//! [`MAX_CELL`], numbers aligned right.

use std::ops::Range;
use std::path::Path;

/// Widest a column is shown (chars); a longer cell ends in `…`.
pub const MAX_CELL: usize = 48;
/// Between two columns.
pub const SEPARATOR: &str = " │ ";

/// The field delimiter of a file the viewer shows as a table, by its
/// extension: `.csv` → `,` (or `;` when the first line has semicolons and
/// no commas, as spreadsheets export in many locales), `.tsv` / `.tab` →
/// tab. `None` for other files.
pub fn delimiter(path: &Path, text: &str) -> Option<char> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "csv" => {
            let first = text.lines().next().unwrap_or("");
            Some(if !first.contains(',') && first.contains(';') {
                ';'
            } else {
                ','
            })
        }
        "tsv" | "tab" => Some('\t'),
        _ => None,
    }
}

/// A parsed table: the first record is the header.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
    /// Shown width of each column (chars).
    pub widths: Vec<usize>,
    /// A column whose every non-empty cell below the header is a number.
    pub numeric: Vec<bool>,
}

impl Table {
    /// Parse `text`; `None` when it holds no record.
    pub fn parse(text: &str, delim: char) -> Option<Table> {
        let mut rows = records(text, delim);
        if rows.is_empty() {
            return None;
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        for r in &mut rows {
            r.resize(cols, String::new());
        }
        let widths = (0..cols)
            .map(|c| {
                rows.iter()
                    .map(|r| r[c].chars().count())
                    .max()
                    .unwrap_or(0)
                    .clamp(1, MAX_CELL)
            })
            .collect();
        let numeric = (0..cols)
            .map(|c| {
                let mut cells = rows[1..]
                    .iter()
                    .map(|r| r[c].trim())
                    .filter(|s| !s.is_empty());
                let mut any = false;
                let all = cells.all(|s| {
                    any = true;
                    is_number(s)
                });
                any && all
            })
            .collect();
        Some(Table {
            rows,
            widths,
            numeric,
        })
    }

    /// Row `i` as one line, cells padded to their column's width (numbers
    /// to the right) and joined by [`SEPARATOR`], with the byte ranges of
    /// the separators in it (drawn dim).
    pub fn line(&self, i: usize) -> (String, Vec<Range<usize>>) {
        let mut out = String::new();
        let mut seps = Vec::new();
        for (c, cell) in self.rows[i].iter().enumerate() {
            if c > 0 {
                seps.push(out.len()..out.len() + SEPARATOR.len());
                out.push_str(SEPARATOR);
            }
            let w = self.widths[c];
            let shown = fit(cell, w);
            let pad = w.saturating_sub(shown.chars().count());
            // the header and text columns to the left
            if self.numeric[c] && i > 0 {
                out.extend(std::iter::repeat_n(' ', pad));
                out.push_str(&shown);
            } else {
                out.push_str(&shown);
                out.extend(std::iter::repeat_n(' ', pad));
            }
        }
        (out.trim_end().to_string(), seps)
    }
}

/// `cell` on one line in at most `w` chars: line breaks as `⏎`, tabs as a
/// blank, a longer one cut with `…`.
fn fit(cell: &str, w: usize) -> String {
    let flat: String = cell
        .chars()
        .map(|c| match c {
            '\n' => '⏎',
            '\r' => ' ',
            '\t' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    if flat.chars().count() <= w {
        return flat;
    }
    let mut cut: String = flat.chars().take(w.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn is_number(s: &str) -> bool {
    let s = s.strip_suffix('%').unwrap_or(s);
    let s = s.trim_start_matches(['+', '-']);
    let s = s
        .strip_prefix(['$', '€', '£', '¥'])
        .unwrap_or(s)
        .replace(['_', '\''], "");
    !s.is_empty() && s.parse::<f64>().is_ok() && s.chars().any(|c| c.is_ascii_digit())
}

/// RFC 4180 records: a field in double quotes may hold the delimiter, line
/// breaks and `""` (one quote); CRLF and LF both end a record; a blank last
/// line is no record.
fn records(text: &str, delim: char) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delim => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_escapes_and_line_breaks() {
        let t = Table::parse(
            "name,note,n\r\n\"Doe, J\",\"said \"\"hi\"\"\",3\nx,\"two\nlines\",-4.5\n",
            ',',
        )
        .unwrap();
        assert_eq!(
            t.rows,
            vec![
                vec!["name", "note", "n"],
                vec!["Doe, J", "said \"hi\"", "3"],
                vec!["x", "two\nlines", "-4.5"],
            ]
        );
        assert_eq!(t.numeric, vec![false, false, true]);
        assert_eq!(t.widths, vec![6, 9, 4]);
    }

    #[test]
    fn lines_pad_align_and_mark_separators() {
        let t = Table::parse("a,qty\nlong name,7\nb,12\n", ',').unwrap();
        let (header, seps) = t.line(0);
        assert_eq!(header, "a         │ qty");
        assert_eq!(&header[seps[0].clone()], SEPARATOR);
        assert_eq!(t.line(1).0, "long name │   7");
        assert_eq!(t.line(2).0, "b         │  12");
    }

    #[test]
    fn uneven_rows_long_cells_and_breaks_in_cells() {
        let long = "x".repeat(MAX_CELL + 10);
        let t = Table::parse(&format!("a,b,c\n1\n\"{long}\",\"p\nq\"\n"), ',').unwrap();
        assert!(t.rows.iter().all(|r| r.len() == 3));
        assert_eq!(t.widths[0], MAX_CELL);
        let line = t.line(2).0;
        assert!(line.starts_with(&format!("{}…", "x".repeat(MAX_CELL - 1))));
        assert!(line.contains("p⏎q"));
    }

    #[test]
    fn delimiter_by_extension_and_first_line() {
        let p = Path::new("x.CSV");
        assert_eq!(delimiter(p, "a,b\n"), Some(','));
        assert_eq!(delimiter(p, "a;b\n1,5;2\n"), Some(';'));
        assert_eq!(delimiter(Path::new("x.tsv"), "a\tb"), Some('\t'));
        assert_eq!(delimiter(Path::new("x.txt"), "a,b"), None);
        assert_eq!(delimiter(Path::new("csv"), "a,b"), None);
    }

    #[test]
    fn numbers() {
        for n in ["3", "-4.5", "1e3", "12%", "$1_000", "+7"] {
            assert!(is_number(n), "{n}");
        }
        for s in ["", "-", "abc", "1.2.3", "NaN", "inf"] {
            assert!(!is_number(s), "{s}");
        }
        assert!(Table::parse("", ',').is_none());
        // a BOM is no part of the first header
        assert_eq!(Table::parse("\u{feff}a,b\n", ',').unwrap().rows[0][0], "a");
    }
}
