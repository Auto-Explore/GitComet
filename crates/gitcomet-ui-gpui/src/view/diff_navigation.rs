//! Change navigation shared by every diff surface: one stop per change block,
//! landing on the block's first row.

/// `current` is the focused row; with nothing focused there is no previous stop.
pub(crate) fn diff_nav_prev_target(entries: &[usize], current: Option<usize>) -> Option<usize> {
    let current = current?;
    entries.iter().rev().find(|&&ix| ix < current).copied()
}

/// With nothing focused the first stop is reachable, even one at row 0.
pub(crate) fn diff_nav_next_target(entries: &[usize], current: Option<usize>) -> Option<usize> {
    match current {
        Some(current) => entries.iter().find(|&&ix| ix > current).copied(),
        None => entries.first().copied(),
    }
}

/// First row of each contiguous run of changed rows.
pub(crate) fn change_block_entries(len: usize, is_change: impl FnMut(usize) -> bool) -> Vec<usize> {
    change_block_entries_with_transparent_rows(len, is_change, |_| false)
}

/// Like [`change_block_entries`], but a transparent row keeps an open block
/// open, e.g. a `\ No newline` marker between the `-` and `+` sides of one
/// edit. `is_transparent` is only asked inside a block, for unchanged rows.
pub(crate) fn change_block_entries_with_transparent_rows(
    len: usize,
    mut is_change: impl FnMut(usize) -> bool,
    mut is_transparent: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_block = false;
    for ix in 0..len {
        if is_change(ix) {
            if !in_block {
                out.push(ix);
                in_block = true;
            }
        } else if in_block && !is_transparent(ix) {
            in_block = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_nav_prev_next_targets_do_not_wrap() {
        let entries = vec![10, 20, 30];

        assert_eq!(diff_nav_prev_target(&entries, Some(10)), None);
        assert_eq!(diff_nav_next_target(&entries, Some(30)), None);

        assert_eq!(diff_nav_prev_target(&entries, Some(25)), Some(20));
        assert_eq!(diff_nav_next_target(&entries, Some(25)), Some(30));

        assert_eq!(diff_nav_next_target(&entries, Some(0)), Some(10));
        assert_eq!(diff_nav_prev_target(&entries, Some(100)), Some(30));
    }

    #[test]
    fn diff_nav_targets_without_focus_reach_the_first_stop_only_forwards() {
        let entries = vec![0, 20];

        assert_eq!(diff_nav_next_target(&entries, None), Some(0));
        assert_eq!(diff_nav_prev_target(&entries, None), None);
        assert_eq!(diff_nav_next_target(&[], None), None);
    }

    #[test]
    fn change_block_entries_mark_the_first_row_of_each_run() {
        let changed = [true, true, false, false, true, false, true, true];
        assert_eq!(
            change_block_entries(changed.len(), |ix| changed[ix]),
            vec![0, 4, 6]
        );
        assert!(change_block_entries(3, |_| false).is_empty());
    }

    #[test]
    fn transparent_rows_neither_start_nor_split_a_block() {
        // `-`, marker, `+`, marker, context, marker, `+`
        let changed = [true, false, true, false, false, false, true];
        let transparent = [false, true, false, true, false, true, false];
        let mut asked = Vec::new();
        let entries = change_block_entries_with_transparent_rows(
            changed.len(),
            |ix| changed[ix],
            |ix| {
                asked.push(ix);
                transparent[ix]
            },
        );

        assert_eq!(entries, vec![0, 6]);
        // Row 5 is a marker outside any block, so it is never looked up.
        assert_eq!(asked, vec![1, 3, 4]);
    }
}
