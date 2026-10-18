//! Block filter (Warp): narrow one block's output to the lines that match
//! a query. Toggles: match case (⌥C), regular expression (⌥R), invert
//! (⌥I, keep the lines that do NOT match) and context lines (⌥↑ / ⌥↓: N
//! lines before and after each match, a separator row between runs).
//!
//! Every block keeps its own filter until it is cleared (Esc in the filter
//! bar, or "Clear output filter"): the filter shortcut turns a kept filter
//! off and on again, and it re-applies with the same query and toggles.
//! An invalid regular expression shows an inline error and the last valid
//! query stays applied. Regexes are the `regex` crate's (linear time, no
//! backtracking) with compile-size limits ([`Matcher::new`]).

use rift_core::BlockId;

use crate::views::terminal::Matcher;

/// Most context lines one side of a match.
pub const MAX_CONTEXT: u8 = 20;

#[derive(Clone)]
pub struct BlockFilter {
    pub block: BlockId,
    pub query: String,
    /// "Aa": match case exactly.
    pub case: bool,
    /// ".*": the query is a regular expression.
    pub regex: bool,
    /// "!": keep the lines that do not match.
    pub invert: bool,
    /// Lines kept before and after each match.
    pub context: u8,
    /// Applied; a filter switched off keeps its query and toggles.
    pub enabled: bool,
    /// Why the query does not compile; the last valid one stays applied.
    pub error: Option<String>,
    /// The last valid query's matcher (`None`: empty, every line shows).
    matcher: Option<Matcher>,
    /// The last valid query and its regex toggle (what the header names).
    applied: (String, bool),
}

impl std::fmt::Debug for BlockFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(block={} query={:?} case={} regex={} invert={} context={} enabled={} error={:?} applied={})",
            self.block,
            self.query,
            self.case,
            self.regex,
            self.invert,
            self.context,
            self.enabled,
            self.error,
            self.matcher.is_some()
        )
    }
}

impl BlockFilter {
    pub fn new(block: BlockId) -> Self {
        BlockFilter {
            block,
            query: String::new(),
            case: false,
            regex: false,
            invert: false,
            context: 0,
            enabled: true,
            error: None,
            matcher: None,
            applied: (String::new(), false),
        }
    }

    /// Compile the query with the current toggles. An invalid one records
    /// the error and keeps the previous matcher applied.
    pub fn recompile(&mut self) {
        match Matcher::new(&self.query, self.case, self.regex) {
            Ok(m) => {
                self.matcher = m;
                self.error = None;
                self.applied = (self.query.clone(), self.regex);
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The applied matcher (`None`: nothing to narrow by).
    pub fn matcher(&self) -> Option<&Matcher> {
        self.matcher.as_ref()
    }

    /// Header note: `filter "q" · shown/total lines` with the toggles; it
    /// names the applied query (the last valid one).
    pub fn note(&self, shown: usize, total: usize) -> String {
        let (query, regex) = &self.applied;
        let mut flags = String::new();
        if self.case {
            flags.push_str(" Aa");
        }
        if self.invert {
            flags.push_str(" not");
        }
        if self.context > 0 {
            flags.push_str(&format!(" ±{}", self.context));
        }
        let q = if *regex {
            format!("/{query}/")
        } else {
            format!("\"{query}\"")
        };
        format!("filter {q}{flags} · {shown}/{total} lines")
    }

    /// Context lines one step up (`up`) or down, within 0..=MAX_CONTEXT.
    pub fn step_context(&mut self, up: bool) {
        self.context = if up {
            (self.context + 1).min(MAX_CONTEXT)
        } else {
            self.context.saturating_sub(1)
        };
    }
}

/// The block filters of one pane, and the one the filter bar edits.
#[derive(Default, Debug)]
pub struct BlockFilters {
    pub list: Vec<BlockFilter>,
    /// The block whose filter the bar edits (while it is open).
    pub editing: Option<BlockId>,
}

impl BlockFilters {
    pub fn get(&self, block: BlockId) -> Option<&BlockFilter> {
        self.list.iter().find(|f| f.block == block)
    }

    pub fn get_mut(&mut self, block: BlockId) -> Option<&mut BlockFilter> {
        self.list.iter_mut().find(|f| f.block == block)
    }

    /// The filters that apply.
    pub fn enabled(&self) -> impl Iterator<Item = &BlockFilter> {
        self.list.iter().filter(|f| f.enabled)
    }

    pub fn editing_mut(&mut self) -> Option<&mut BlockFilter> {
        let block = self.editing?;
        self.get_mut(block)
    }

    pub fn editing(&self) -> Option<&BlockFilter> {
        self.get(self.editing?)
    }

    /// Clear `block`'s filter (its query and toggles are forgotten).
    pub fn remove(&mut self, block: BlockId) {
        self.list.retain(|f| f.block != block);
        if self.editing == Some(block) {
            self.editing = None;
        }
    }

    /// Forget the filters of blocks that are gone.
    pub fn retain_blocks(&mut self, alive: impl Fn(BlockId) -> bool) {
        self.list.retain(|f| alive(f.block));
        if self.editing.is_some_and(|b| !alive(b)) {
            self.editing = None;
        }
    }
}

/// Which of a block's output lines stay: `matched[i]` = line `i` matches.
/// Inverted, the lines that do not match are the hits; `context` lines
/// on each side of every hit stay too.
pub fn keep_mask(matched: &[bool], invert: bool, context: usize) -> Vec<bool> {
    let hits: Vec<bool> = matched.iter().map(|m| *m != invert).collect();
    if context == 0 {
        return hits;
    }
    let n = hits.len();
    let mut keep = vec![false; n];
    // sweep: `reach` = the last line the newest hit's after-context covers
    let mut reach: Option<usize> = None;
    for i in 0..n {
        if hits[i] {
            let from = i.saturating_sub(context);
            for k in keep.iter_mut().take(i + 1).skip(from) {
                *k = true;
            }
            reach = Some(i + context);
        } else if reach.is_some_and(|r| i <= r) {
            keep[i] = true;
        }
    }
    keep
}

/// Indices of the kept lines that start a run after an earlier one (a
/// separator row goes above each).
pub fn separators(keep: &[bool]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut seen = false;
    for i in 0..keep.len() {
        if keep[i] {
            if seen && i > 0 && !keep[i - 1] {
                out.push(i);
            }
            seen = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(s: &str) -> Vec<bool> {
        s.chars().map(|c| c == '1').collect()
    }

    fn show(v: &[bool]) -> String {
        v.iter().map(|b| if *b { '1' } else { '0' }).collect()
    }

    #[test]
    fn keep_mask_plain_inverted_and_context() {
        let m = mask("0010000100");
        assert_eq!(show(&keep_mask(&m, false, 0)), "0010000100");
        assert_eq!(show(&keep_mask(&m, true, 0)), "1101111011");
        // one line each side; the runs stay apart
        assert_eq!(show(&keep_mask(&m, false, 1)), "0111001110");
        // two lines: the runs merge
        assert_eq!(show(&keep_mask(&m, false, 2)), "1111111111");
        // clamped at both ends
        assert_eq!(
            show(&keep_mask(&mask("1000000001"), false, 3)),
            "1111001111"
        );
        assert!(keep_mask(&[], false, 5).is_empty());
    }

    #[test]
    fn separators_between_runs_only() {
        assert_eq!(separators(&mask("0111001110")), vec![6]);
        assert_eq!(separators(&mask("1111111111")), Vec::<usize>::new());
        assert_eq!(separators(&mask("1010101")), vec![2, 4, 6]);
        assert_eq!(separators(&mask("0000")), Vec::<usize>::new());
    }

    #[test]
    fn invalid_regex_keeps_the_last_valid_matcher() {
        let mut f = BlockFilter::new(1);
        f.regex = true;
        f.query = "err(or)?".into();
        f.recompile();
        assert!(f.error.is_none());
        let hits = |f: &BlockFilter| f.matcher().unwrap().find("an error here").len();
        assert_eq!(hits(&f), 1);
        f.query = "err(or".into();
        f.recompile();
        assert!(f.error.is_some(), "an unclosed group is an error");
        // the previous query still applies
        assert_eq!(hits(&f), 1);
        f.query = "x".into();
        f.recompile();
        assert!(f.error.is_none());
        assert_eq!(hits(&f), 0);
    }

    #[test]
    fn case_toggle_and_huge_regex_bound() {
        let mut f = BlockFilter::new(1);
        f.query = "ERROR".into();
        f.recompile();
        assert_eq!(f.matcher().unwrap().find("error").len(), 1);
        f.case = true;
        f.recompile();
        assert_eq!(f.matcher().unwrap().find("error").len(), 0);
        // a regex past the 1 MiB compile limit (under the crate default 10 MiB) is refused
        f.regex = true;
        f.query = "(a{200}){200}".into();
        f.recompile();
        assert!(f.error.is_some(), "the compile-size limit applies");
    }

    #[test]
    fn filters_persist_per_block_and_toggle() {
        let mut fs = BlockFilters::default();
        let mut a = BlockFilter::new(3);
        a.query = "x".into();
        fs.list.push(a);
        fs.list.push(BlockFilter::new(5));
        assert_eq!(fs.enabled().count(), 2);
        fs.get_mut(3).unwrap().enabled = false;
        assert_eq!(fs.enabled().map(|f| f.block).collect::<Vec<_>>(), vec![5]);
        assert_eq!(fs.get(3).unwrap().query, "x", "kept while off");
        fs.editing = Some(5);
        fs.remove(5);
        assert!(fs.editing.is_none() && fs.get(5).is_none());
        fs.retain_blocks(|b| b != 3);
        assert!(fs.list.is_empty());
    }

    #[test]
    fn note_names_the_toggles() {
        let mut f = BlockFilter::new(1);
        f.query = "a.c".into();
        f.recompile();
        assert_eq!(f.note(2, 9), "filter \"a.c\" · 2/9 lines");
        f.regex = true;
        f.case = true;
        f.invert = true;
        f.context = 2;
        f.recompile();
        assert_eq!(f.note(5, 9), "filter /a.c/ Aa not ±2 · 5/9 lines");
        // an invalid edit leaves the note on the applied query
        f.query = "a.c(".into();
        f.recompile();
        assert_eq!(f.note(5, 9), "filter /a.c/ Aa not ±2 · 5/9 lines");
        f.context = MAX_CONTEXT;
        f.step_context(true);
        assert_eq!(f.context, MAX_CONTEXT);
        f.context = 0;
        f.step_context(false);
        assert_eq!(f.context, 0);
    }
}
