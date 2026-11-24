use std::num::NonZeroU32;

use super::{FileDiffPlan, for_each_plan_row_meta};

/// Zero-based diff rows indexed by source line. Most maps take four bytes
/// per line. A row beyond the compact range promotes that map to native indices.
#[derive(Clone, Debug)]
pub struct LineToRowMap(Storage);

#[derive(Clone, Debug)]
enum Storage {
    Compact(Vec<Option<NonZeroU32>>),
    Wide(Vec<Option<usize>>),
}

impl Default for LineToRowMap {
    fn default() -> Self {
        Self::new(0)
    }
}

impl LineToRowMap {
    pub fn new(line_count: usize) -> Self {
        Self(Storage::Compact(vec![None; line_count]))
    }

    /// Build the two maps for a side-by-side diff.
    pub fn for_plan(plan: &FileDiffPlan, old_lines: usize, new_lines: usize) -> (Self, Self) {
        let mut old = Self::new(old_lines);
        let mut new = Self::new(new_lines);
        for_each_plan_row_meta(plan, |index, row| {
            if let Some(line) = row.old_line {
                old.set(line.saturating_sub(1) as usize, index);
            }
            if let Some(line) = row.new_line {
                new.set(line.saturating_sub(1) as usize, index);
            }
        });
        (old, new)
    }

    pub fn get(&self, line: usize) -> Option<usize> {
        match &self.0 {
            Storage::Compact(rows) => rows.get(line)?.map(|row| (row.get() - 1) as usize),
            Storage::Wide(rows) => rows.get(line).copied().flatten(),
        }
    }

    pub fn set(&mut self, line: usize, row: usize) {
        match &mut self.0 {
            Storage::Compact(rows) => {
                let Some(slot) = rows.get_mut(line) else {
                    return;
                };
                if let Some(value) = u32::try_from(row).ok().and_then(|r| r.checked_add(1)) {
                    *slot = NonZeroU32::new(value);
                } else {
                    let mut wide: Vec<_> = rows
                        .iter()
                        .map(|r| r.map(|n| (n.get() - 1) as usize))
                        .collect();
                    wide[line] = Some(row);
                    self.0 = Storage::Wide(wide);
                }
            }
            Storage::Wide(rows) => {
                if let Some(slot) = rows.get_mut(line) {
                    *slot = Some(row);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_preserves_zero_missing_and_large_rows() {
        let mut map = LineToRowMap::new(4);
        map.set(0, 0);
        map.set(1, (u32::MAX - 1) as usize);
        assert!(matches!(map.0, Storage::Compact(_)));
        map.set(2, u32::MAX as usize);
        assert_eq!(map.get(0), Some(0));
        assert_eq!(map.get(1), Some((u32::MAX - 1) as usize));
        assert_eq!(map.get(2), Some(u32::MAX as usize));
        assert_eq!(map.get(3), None);
        assert_eq!(map.get(4), None);
    }
}
