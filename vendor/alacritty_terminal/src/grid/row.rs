//! Defines the Row type which makes up lines in the grid.

use std::cmp::{max, min};
use std::ops::{Index, IndexMut};
use std::{mem, ptr, slice};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::grid::GridCell;
use crate::index::Column;
use crate::term::cell::ResetDiscriminant;

/// A row in the grid.
#[derive(Default, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Row<T> {
    inner: Vec<T>,

    /// Maximum number of occupied entries.
    ///
    /// This is the upper bound on the number of elements in the row, which have been modified
    /// since the last reset. All cells after this point are guaranteed to be equal.
    ///
    /// Rift: [`Row::compact`] scans only below it, so every method that
    /// changes cells must keep that guarantee (`grow` and `append` widen it).
    pub(crate) occ: usize,

    /// DEC line size attribute (Rift): 0 single, 3 double-height top half,
    /// 4 bottom half, 6 double-width (the `ESC #` final byte).
    #[cfg_attr(feature = "serde", serde(default))]
    pub attr: u8,

    /// Logical width of a compacted row (Rift), 0 when `inner` holds every
    /// cell. [`Row::compact`] drops the run of equal cells at the end of a
    /// scrollback row and keeps one of them: every cell from
    /// `inner.len() - 1` up to `width` reads as that last stored cell. Any
    /// mutable access restores the full row first.
    #[cfg_attr(feature = "serde", serde(default))]
    width: u32,
}

impl<T: PartialEq> PartialEq for Row<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.into_iter().eq(other)
    }
}

impl<T: Default> Row<T> {
    /// Create a new terminal row.
    ///
    /// Ideally the `template` should be `Copy` in all performance sensitive scenarios.
    pub fn new(columns: usize) -> Row<T> {
        debug_assert!(columns >= 1);

        let mut inner: Vec<T> = Vec::with_capacity(columns);

        // This is a slightly optimized version of `std::vec::Vec::resize`.
        unsafe {
            let mut ptr = inner.as_mut_ptr();

            for _ in 1..columns {
                ptr::write(ptr, T::default());
                ptr = ptr.offset(1);
            }
            ptr::write(ptr, T::default());

            inner.set_len(columns);
        }

        Row { inner, occ: 0, attr: 0, width: 0 }
    }
}

impl<T: Clone> Row<T> {
    /// Restore the cells a [`Row::compact`] dropped.
    #[inline]
    fn expand(&mut self) {
        if self.width != 0 {
            let last = self.inner[self.inner.len() - 1].clone();
            self.inner.resize(self.width as usize, last);
            self.width = 0;
        }
    }

    /// Drop the run of equal cells at the end of the row, keeping one.
    ///
    /// The row reads exactly as before; it only takes less memory. Rows
    /// that would save fewer than 16 cells stay as they are.
    pub fn compact(&mut self)
    where
        T: PartialEq + GridCell,
    {
        self.compact_with(None);
    }

    /// [`Row::compact`], reusing the buffer of `spare` when it is a compacted
    /// row about to be reset: `spare` takes this row's full buffer instead,
    /// so scrolling through a full history allocates nothing.
    pub fn compact_with(&mut self, spare: Option<&mut Row<T>>)
    where
        T: PartialEq + GridCell,
    {
        let len = self.inner.len();
        if self.width != 0 || len < 2 {
            return;
        }

        // Cells from `occ` on equal the last one: only the occupied part is
        // scanned (a full-width scan per scrolled line costs throughput).
        let last = &self.inner[len - 1];
        let scan = min(self.occ, len - 1);
        let keep = self.inner[..scan].iter().rposition(|cell| cell != last).map_or(1, |i| i + 2);
        if len - keep < 16 {
            return;
        }

        match spare {
            // A buffer far larger than needed would keep its size here.
            Some(spare) if spare.width != 0 && spare.inner.capacity() <= 2 * keep + 16 => {
                // `spare` takes this row's old cells, equal from `keep - 1`
                // on. Its reset leaves those alone only when they already
                // are blanks; anything else (a rule of dashes) is reset too.
                let mut blank = last.clone();
                blank.reset(last);
                let dirty = if blank == *last { keep } else { len };

                spare.inner.clear();
                spare.inner.extend_from_slice(&self.inner[..keep]);
                mem::swap(&mut self.inner, &mut spare.inner);
                spare.width = 0;
                spare.occ = dirty;
            },
            _ => {
                // A new allocation, not `shrink_to_fit`: an allocator may
                // shrink in place, and the small remnant then keeps the
                // whole old block's pages resident.
                let mut cells = Vec::with_capacity(keep);
                cells.extend(self.inner.drain(..keep));
                self.inner = cells;
            },
        }
        self.width = len as u32;
    }

    /// Increase the number of columns in the row.
    #[inline]
    pub fn grow(&mut self, columns: usize)
    where
        T: Default,
    {
        if self.len() >= columns {
            return;
        }

        self.expand();
        // the new cells may differ from the old trailing ones (Rift: `occ`)
        self.occ = self.inner.len();
        self.inner.resize_with(columns, T::default);
    }

    /// Reduce the number of columns in the row.
    ///
    /// This will return all non-empty cells that were removed.
    pub fn shrink(&mut self, columns: usize) -> Option<Vec<T>>
    where
        T: GridCell,
    {
        if self.len() <= columns {
            return None;
        }

        self.expand();

        // Split off cells for a new row.
        let mut new_row = self.inner.split_off(columns);
        let index = new_row.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1);
        new_row.truncate(index);

        self.occ = min(self.occ, columns);

        if new_row.is_empty() { None } else { Some(new_row) }
    }

    /// Reset all cells in the row to the `template` cell.
    #[inline]
    pub fn reset<D>(&mut self, template: &T)
    where
        T: ResetDiscriminant<D> + GridCell,
        D: PartialEq,
    {
        debug_assert!(!self.inner.is_empty());

        self.expand();

        // Mark all cells as dirty if template cell changed.
        let len = self.inner.len();
        if self.inner[len - 1].discriminant() != template.discriminant() {
            self.occ = len;
        }

        // Reset every dirty cell in the row.
        for item in &mut self.inner[0..self.occ] {
            item.reset(template);
        }

        self.occ = 0;
        self.attr = 0;
    }

    #[inline]
    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.expand();
        self.occ = self.inner.len();
        self.inner.last_mut()
    }

    #[inline]
    pub fn append(&mut self, vec: &mut Vec<T>)
    where
        T: GridCell,
    {
        self.expand();
        self.inner.append(vec);
        // the appended cells follow the old trailing ones (Rift: `occ`)
        self.occ = self.inner.len();
    }

    #[inline]
    pub fn append_front(&mut self, mut vec: Vec<T>) {
        self.expand();
        self.occ += vec.len();

        vec.append(&mut self.inner);
        self.inner = vec;
    }

    #[inline]
    pub fn front_split_off(&mut self, at: usize) -> Vec<T> {
        self.expand();
        self.occ = self.occ.saturating_sub(at);

        let mut split = self.inner.split_off(at);
        std::mem::swap(&mut split, &mut self.inner);
        split
    }

    /// All cells, for writing.
    #[inline]
    pub fn cells_mut(&mut self) -> &mut [T] {
        self.expand();
        self.occ = self.inner.len();
        &mut self.inner[..]
    }

    /// The cells in `start..end`, for writing.
    #[inline]
    pub fn cells_mut_range(&mut self, start: Column, end: Column) -> &mut [T] {
        self.expand();
        self.occ = max(self.occ, end.0);
        &mut self.inner[start.0..end.0]
    }
}

#[allow(clippy::len_without_is_empty)]
impl<T> Row<T> {
    #[inline]
    pub fn from_vec(vec: Vec<T>, occ: usize) -> Row<T> {
        Row { inner: vec, occ, attr: 0, width: 0 }
    }

    /// Whether [`Row::compact`] dropped cells this row reads back.
    #[inline]
    pub fn is_compact(&self) -> bool {
        self.width != 0
    }

    #[inline]
    pub fn len(&self) -> usize {
        if self.width == 0 { self.inner.len() } else { self.width as usize }
    }

    #[inline]
    pub fn last(&self) -> Option<&T> {
        self.inner.last()
    }

    /// Check if all cells in the row are empty.
    #[inline]
    pub fn is_clear(&self) -> bool
    where
        T: GridCell,
    {
        self.inner.iter().all(GridCell::is_empty)
    }

    /// Number of cells the row actually stores.
    #[cfg(test)]
    pub(crate) fn stored_cells(&self) -> usize {
        self.inner.len()
    }

    /// Number of cells the row's buffer has room for.
    #[cfg(test)]
    pub(crate) fn stored_capacity(&self) -> usize {
        self.inner.capacity()
    }
}

/// Iterator over a row's cells, including the ones a compacted row does not
/// store.
#[derive(Clone, Debug)]
pub struct Cells<'a, T> {
    row: &'a Row<T>,
    front: usize,
    back: usize,
}

impl<'a, T> Iterator for Cells<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<&'a T> {
        if self.front == self.back {
            return None;
        }
        let row: &'a Row<T> = self.row;
        let cell = &row[Column(self.front)];
        self.front += 1;
        Some(cell)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.back - self.front;
        (len, Some(len))
    }
}

impl<T> DoubleEndedIterator for Cells<'_, T> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        Some(&self.row[Column(self.back)])
    }
}

impl<T> ExactSizeIterator for Cells<'_, T> {}

impl<'a, T> IntoIterator for &'a Row<T> {
    type IntoIter = Cells<'a, T>;
    type Item = &'a T;

    #[inline]
    fn into_iter(self) -> Cells<'a, T> {
        Cells { row: self, front: 0, back: self.len() }
    }
}

impl<'a, T: Clone> IntoIterator for &'a mut Row<T> {
    type IntoIter = slice::IterMut<'a, T>;
    type Item = &'a mut T;

    #[inline]
    fn into_iter(self) -> slice::IterMut<'a, T> {
        self.cells_mut().iter_mut()
    }
}

impl<T> Index<Column> for Row<T> {
    type Output = T;

    #[inline]
    fn index(&self, index: Column) -> &T {
        match self.inner.get(index.0) {
            Some(cell) => cell,
            None => {
                assert!(index.0 < self.len(), "column {} out of {}", index.0, self.len());
                &self.inner[self.inner.len() - 1]
            },
        }
    }
}

impl<T: Clone> IndexMut<Column> for Row<T> {
    #[inline]
    fn index_mut(&mut self, index: Column) -> &mut T {
        self.expand();
        self.occ = max(self.occ, *index + 1);
        &mut self.inner[index.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &str, columns: usize) -> Row<char> {
        let mut row = Row::from_vec(cells.chars().collect(), cells.len());
        row.inner.resize(columns, ' ');
        row
    }

    #[test]
    fn compact_keeps_every_cell_readable() {
        let mut r = row("hello", 80);
        let before = r.clone();
        r.compact();
        assert_eq!(r.inner.len(), 6);
        assert_eq!(r.len(), 80);
        assert_eq!(r, before);
        assert_eq!(r[Column(4)], 'o');
        assert_eq!(r[Column(79)], ' ');
        assert_eq!(r.into_iter().rev().position(|c| *c != ' '), Some(75));
        assert_eq!(r.into_iter().count(), 80);
    }

    #[test]
    fn writing_restores_the_full_row() {
        let mut r = row("ab", 40);
        r.compact();
        r[Column(30)] = 'x';
        assert_eq!(r.inner.len(), 40);
        assert_eq!(r.len(), 40);
        assert_eq!(r[Column(30)], 'x');
        assert_eq!(r[Column(39)], ' ');
        assert_eq!(r.occ, 31);
    }

    #[test]
    fn short_runs_and_full_rows_stay_as_they_are() {
        let mut r = row("abcdefghij", 20);
        r.compact();
        assert_eq!(r.inner.len(), 20);

        let mut full = row(&"xy".repeat(20), 40);
        full.compact();
        assert_eq!(full.inner.len(), 40);
    }

    #[test]
    fn a_blank_row_keeps_one_cell() {
        let mut r = row("", 100);
        r.compact();
        assert_eq!(r.inner.len(), 1);
        assert!(r.into_iter().all(|c| *c == ' '));
        assert_eq!(r.len(), 100);
    }

    // Compaction scans below `occ` only: growing and appending must keep
    // every cell from `occ` on equal to the last one.
    #[test]
    fn grow_and_append_keep_the_cells_compaction_skips() {
        let mut r = row("ab", 20);
        r.inner[2..].fill('x');
        r.grow(60);
        let before = r.clone();
        r.compact();
        assert_eq!(r, before);
        assert_eq!(r[Column(19)], 'x');

        let mut r = row("ab", 20);
        r.append(&mut vec!['y'; 20]);
        r.inner.resize(60, ' ');
        let before = r.clone();
        r.compact();
        assert_eq!(r, before);
        assert_eq!(r[Column(39)], 'y');
    }

    #[test]
    #[should_panic(expected = "column 50 out of 50")]
    fn reading_past_the_width_still_panics() {
        let mut r = row("a", 50);
        r.compact();
        let _ = r[Column(50)];
    }
}
