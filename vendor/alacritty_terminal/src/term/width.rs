//! Character and grapheme-cluster widths, as jquast's wcwidth `wcswidth`
//! measures them (the reference ucs-detect checks terminals against).
//!
//! A character's own width comes from [`char_width`]. On top of that, text
//! written into the grid forms clusters:
//!
//! - VS16 (U+FE0F) after a narrow emoji-capable character widens it to two
//!   cells; VS15 (U+FE0E) after a wide emoji narrows it to one.
//! - ZWJ (U+200D) joins the next character into the current cluster.
//! - A second regional indicator joins the first one (a flag), and a skin
//!   tone modifier joins the emoji before it.
//! - A virama joins the following consonant into a conjunct, and a spacing
//!   mark after a base widens its cluster; either caps the cluster at two
//!   cells.
//!
//! [`Cluster`] carries that state between characters `Term::input` receives.

use super::width_tables::{
    AMBIGUOUS, EXT_PICT, MC, REGIONAL_INDICATOR, VIRAMA, VS15_NARROWS, VS16_WIDENS, WIDE,
    WIDTH_BLOCKS, WIDTH_INDEX, WIDTH_LIMIT, ZERO,
};

const ZWJ: char = '\u{200d}';
const VS15: char = '\u{fe0e}';
const VS16: char = '\u{fe0f}';

fn contains(table: &[(u32, u32)], c: char) -> bool {
    let c = c as u32;
    table
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// An emoji whose default presentation is text (Emoji_Presentation=No):
/// drawn as a text glyph unless VS16 (U+FE0F) follows it.
pub fn text_default_emoji(c: char) -> bool {
    contains(VS16_WIDENS, c)
}

/// An emoji whose default presentation is emoji (Emoji_Presentation=Yes,
/// two cells): drawn in color unless VS15 (U+FE0E) follows it.
pub fn emoji_default(c: char) -> bool {
    contains(EXT_PICT, c) && char_width(c) == Some(2)
}

/// Cells one character takes on its own: `None` for C0/C1 controls.
pub fn char_width(c: char) -> Option<usize> {
    match c as u32 {
        0x20..=0x7e => Some(1),
        0x01..=0x1f | 0x7f..=0x9f => None,
        cp if cp < WIDTH_LIMIT => {
            // two-level table: per character, no search
            let block = WIDTH_INDEX[(cp >> 7) as usize] as usize;
            let byte = WIDTH_BLOCKS[block][((cp & 127) >> 2) as usize];
            Some(((byte >> ((cp & 3) * 2)) & 3) as usize)
        },
        _ => Some(range_width(c)),
    }
}

/// [`char_width`] from the range tables (above the lookup table's limit).
fn range_width(c: char) -> usize {
    if contains(ZERO, c) {
        0
    } else if contains(WIDE, c) {
        2
    } else {
        1
    }
}

fn is_regional_indicator(c: char) -> bool {
    c >= '\u{1f1e6}' && contains(REGIONAL_INDICATOR, c)
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)
}

/// What `Term::input` does with a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Starts a new cluster this many cells wide.
    New(usize),
    /// Joins the cluster in the previous cell.
    Join,
    /// Joins the cluster and makes it two cells wide.
    JoinWiden,
    /// Joins the cluster and makes it one cell wide.
    JoinNarrow,
}

/// Cluster state between characters.
#[derive(Debug, Default, Clone)]
pub struct Cluster {
    /// The cluster's base character, while VS15/VS16 may still apply to it.
    base: Option<char>,
    /// The cluster's current width in cells.
    width: usize,
    /// The last character measured (skin tones check it).
    last: Option<char>,
    /// The character before this one was a virama.
    virama: bool,
    /// A ZWJ joins the next character.
    join_next: bool,
    /// Regional indicators written in a row.
    regional: usize,
    /// Where the cursor was after the cluster's last character: a motion
    /// since then ends the cluster.
    pub(crate) cursor_after: Option<(i32, usize, bool)>,
    /// East Asian Ambiguous characters take two cells (Rift's
    /// `ambiguous_width = "wide"`); kept across [`Self::reset`].
    ambiguous_wide: bool,
}

impl Cluster {
    /// Ends the cluster: the next character starts fresh.
    pub fn reset(&mut self) {
        *self = Self { ambiguous_wide: self.ambiguous_wide, ..Self::default() };
    }

    /// Measure East Asian Ambiguous characters (outside the Private Use
    /// Areas) two cells wide instead of one.
    pub fn set_ambiguous_wide(&mut self, wide: bool) {
        self.ambiguous_wide = wide;
    }

    /// [`char_width`], with the ambiguous-width setting applied.
    #[inline]
    fn width_of(&self, c: char) -> Option<usize> {
        let width = char_width(c)?;
        Some(if width == 1 && self.ambiguous_wide && contains(AMBIGUOUS, c) { 2 } else { width })
    }

    /// Classifies the next character of an unbroken run of text (no cursor
    /// motion in between). `None` for controls.
    pub fn next(&mut self, c: char) -> Option<Placement> {
        self.cursor_after = Some((0, 0, false));
        self.place(c)
    }

    /// The width of `c` when it starts a new cluster whatever came before:
    /// a visible character below the regional indicators (no flag, skin tone
    /// or emoji) with no ZWJ join or conjunct pending. [`Self::started`]
    /// records it.
    #[inline]
    pub(crate) fn starts_cluster(&self, c: char) -> Option<usize> {
        if self.join_next || self.virama {
            return None;
        }
        if (' '..='~').contains(&c) {
            return Some(1);
        }
        if !('\u{a0}'..'\u{1f1e6}').contains(&c) {
            return None;
        }
        self.width_of(c).filter(|&w| w > 0)
    }

    /// Records the character [`Self::starts_cluster`] accepted, and the
    /// cursor after it.
    #[inline]
    pub(crate) fn started(&mut self, c: char, width: usize, cursor_after: (i32, usize, bool)) {
        self.regional = 0;
        self.base = Some(c);
        self.last = Some(c);
        self.width = width;
        self.cursor_after = Some(cursor_after);
    }

    /// Classifies `c`, updating the state. `None` for controls.
    #[inline]
    pub fn place(&mut self, c: char) -> Option<Placement> {
        // printable ASCII outside a ZWJ join or conjunct: a new one-cell
        // cluster (the hot path of every output flood)
        if (' '..='~').contains(&c) && !self.join_next && !self.virama {
            self.regional = 0;
            self.base = Some(c);
            self.last = Some(c);
            self.width = 1;
            return Some(Placement::New(1));
        }
        self.place_other(c)
    }

    #[inline(never)]
    fn place_other(&mut self, c: char) -> Option<Placement> {
        let width = self.width_of(c)?;
        // a visible character that joins nothing: a new cluster (most
        // non-ASCII text — accents, CJK, symbols)
        if width > 0
            && !self.join_next
            && !self.virama
            && c < '\u{1f1e6}'
            && !matches!(c, ZWJ | VS15 | VS16)
        {
            self.regional = 0;
            self.base = Some(c);
            self.last = Some(c);
            self.width = width;
            return Some(Placement::New(width));
        }
        let regional = is_regional_indicator(c);
        let regional_before = self.regional;
        self.regional = if regional { regional_before + 1 } else { 0 };
        if self.cursor_after.is_none() {
            // nothing to join: a lone modifier stands for itself
            self.base = None;
            self.width = 0;
            self.virama = false;
            self.join_next = false;
        }

        if self.join_next {
            self.join_next = false;
            self.virama = false;
            return Some(Placement::Join);
        }
        if c == ZWJ {
            if !self.virama && self.width > 0 {
                self.join_next = true;
            }
            return Some(Placement::Join);
        }
        if c == VS16 {
            let base = self.base.take();
            if base.is_some_and(|b| contains(VS16_WIDENS, b)) && self.width == 1 {
                self.width = 2;
                return Some(Placement::JoinWiden);
            }
            return Some(Placement::Join);
        }
        if c == VS15 {
            let base = self.base.take();
            if base.is_some_and(|b| contains(VS15_NARROWS, b)) && self.width == 2 {
                self.width = 1;
                return Some(Placement::JoinNarrow);
            }
            return Some(Placement::Join);
        }
        if regional && regional_before % 2 == 1 && self.width > 0 {
            self.last = Some(c);
            return Some(Placement::Join);
        }
        if is_skin_tone(c) && self.width > 0 && self.last.is_some_and(|l| contains(EXT_PICT, l)) {
            return Some(Placement::Join);
        }

        if width > 0 {
            self.last = Some(c);
            if self.virama {
                // virama + consonant: a conjunct, two cells
                self.virama = false;
                self.base = Some(c);
                let before = std::mem::replace(&mut self.width, 2);
                return Some(match before {
                    // the virama had no base: the conjunct starts here
                    0 => Placement::New(2),
                    1 => Placement::JoinWiden,
                    _ => Placement::Join,
                });
            }
            self.virama = false;
            self.base = Some(c);
            self.width = width;
            return Some(Placement::New(width));
        }
        if contains(VIRAMA, c) {
            self.virama = true;
            return Some(Placement::Join);
        }
        if self.width == 0 {
            // a combining character with nothing before it
            return Some(Placement::Join);
        }
        self.virama = false;
        if self.base.is_some() && contains(MC, c) {
            self.base = None;
            let widen = self.width < 2;
            self.width = 2;
            return Some(if widen { Placement::JoinWiden } else { Placement::Join });
        }
        Some(Placement::Join)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Total width of `s` written in one go.
    fn width(s: &str) -> usize {
        let mut cluster = Cluster::default();
        let mut total = 0;
        for c in s.chars() {
            match cluster.place(c).unwrap() {
                Placement::New(w) => total += w,
                Placement::Join => {},
                Placement::JoinWiden => total += 1,
                Placement::JoinNarrow => total -= 1,
            }
            cluster.cursor_after = Some((0, total, false));
        }
        total
    }

    #[test]
    fn lookup_table_matches_the_ranges() {
        for c in (0xa0..WIDTH_LIMIT).filter_map(char::from_u32) {
            assert_eq!(char_width(c), Some(range_width(c)), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn default_presentations() {
        // text by default: stopwatch, check mark; emoji by default: watch
        assert!(text_default_emoji('\u{23f1}') && !emoji_default('\u{23f1}'));
        assert!(text_default_emoji('\u{2714}') && !emoji_default('\u{2714}'));
        assert!(emoji_default('\u{231a}') && !text_default_emoji('\u{231a}'));
        assert!(emoji_default('\u{1f600}'));
        // neither: CJK, box drawing, arrows without emoji status
        for c in ['中', '─', '→', 'a'] {
            assert!(!emoji_default(c) && !text_default_emoji(c), "{c}");
        }
    }

    #[test]
    fn ambiguous_width() {
        let wide = |s: &str| {
            let mut cluster = Cluster::default();
            cluster.set_ambiguous_wide(true);
            let mut total = 0;
            for c in s.chars() {
                // a cursor motion between characters resets the cluster
                cluster.reset();
                total += match cluster.place(c).unwrap() {
                    Placement::New(w) => w,
                    _ => 0,
                };
            }
            total
        };
        // narrow by default: ± ① α ─ Ж
        assert_eq!(width("±①α─Ж"), 5);
        assert_eq!(wide("±①α─Ж"), 10);
        // the fast path agrees
        let mut cluster = Cluster::default();
        cluster.set_ambiguous_wide(true);
        assert_eq!(cluster.starts_cluster('±'), Some(2));
        assert_eq!(Cluster::default().starts_cluster('±'), Some(1));
        // never: ASCII, wide CJK, combining marks, Powerline and Nerd Font
        // symbols (Private Use Area), plain Latin
        assert_eq!(wide("a漢e"), 4);
        assert_eq!(wide("\u{301}"), 0);
        assert_eq!(wide("\u{e0b0}\u{f015}"), 2);
    }

    #[test]
    fn plain_and_wide() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("漢字"), 4);
        assert_eq!(width("e\u{301}"), 1);
        assert_eq!(char_width('\u{7}'), None);
    }

    #[test]
    fn variation_selectors() {
        // narrow heart + VS16 is an emoji, two cells
        assert_eq!(width("\u{2764}\u{fe0f}"), 2);
        // wide emoji + VS15 is text, one cell
        assert_eq!(width("\u{231a}\u{fe0e}"), 1);
        // VS16 after a plain letter changes nothing
        assert_eq!(width("a\u{fe0f}"), 1);
    }

    #[test]
    fn zwj_sequences_and_modifiers() {
        // family: man ZWJ woman ZWJ girl
        assert_eq!(width("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"), 2);
        // waving hand + skin tone
        assert_eq!(width("\u{1f44b}\u{1f3fd}"), 2);
    }

    #[test]
    fn flags_pair_regional_indicators() {
        assert_eq!(width("\u{1f1fa}\u{1f1f8}"), 2);
        assert_eq!(width("\u{1f1fa}"), 2);
        assert_eq!(width("\u{1f1fa}\u{1f1f8}\u{1f1eb}\u{1f1f7}"), 4);
    }

    #[test]
    fn indic_conjuncts_and_spacing_marks() {
        // Devanagari KA + VIRAMA + SSA: one conjunct, two cells
        assert_eq!(width("\u{915}\u{94d}\u{937}"), 2);
        // KA + spacing vowel sign AA: two cells
        assert_eq!(width("\u{915}\u{93e}"), 2);
        // Tai Tham: spacing mark and sakot with no base before them still
        // make the following consonant a two-cell conjunct
        assert_eq!(width("\u{1a63}\u{1a60}\u{1a20}"), 2);
    }

    #[test]
    fn a_motion_ends_the_cluster() {
        let mut cluster = Cluster::default();
        assert_eq!(cluster.place('\u{2764}'), Some(Placement::New(1)));
        // the cursor moved: VS16 has no base
        assert_eq!(cluster.place('\u{fe0f}'), Some(Placement::Join));
    }
}
