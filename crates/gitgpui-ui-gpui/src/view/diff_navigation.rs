use super::*;

pub(super) fn diff_nav_prev_target(entries: &[usize], current: usize) -> Option<usize> {
    entries.iter().rev().find(|&&ix| ix < current).copied()
}

pub(super) fn diff_nav_next_target(entries: &[usize], current: usize) -> Option<usize> {
    entries.iter().find(|&&ix| ix > current).copied()
}

fn conflict_nav_entries<T>(rows: &[T], mut is_change: impl FnMut(&T) -> bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (ix, row) in rows.iter().enumerate() {
        let is_change = is_change(row);
        if is_change && !in_block {
            out.push(ix);
            in_block = true;
        } else if !is_change {
            in_block = false;
        }
    }
    out
}

pub(super) fn change_block_entries(
    len: usize,
    mut is_change: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    let mut out = Vec::new();
    let mut in_block = false;
    for ix in 0..len {
        let is_change = is_change(ix);
        if is_change && !in_block {
            out.push(ix);
            in_block = true;
        } else if !is_change {
            in_block = false;
        }
    }
    out
}

pub(super) fn conflict_nav_entries_for_split(rows: &[FileDiffRow]) -> Vec<usize> {
    conflict_nav_entries(rows, |row| {
        row.kind != gitgpui_core::file_diff::FileDiffRowKind::Context
    })
}

pub(super) fn conflict_nav_entries_for_inline(rows: &[ConflictInlineRow]) -> Vec<usize> {
    conflict_nav_entries(rows, |row| {
        row.kind != gitgpui_core::domain::DiffLineKind::Context
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitgpui_core::domain::DiffLineKind as DK;
    use gitgpui_core::file_diff::FileDiffRowKind as K;

    #[test]
    fn diff_nav_prev_next_targets_do_not_wrap() {
        let entries = vec![10, 20, 30];

        assert_eq!(diff_nav_prev_target(&entries, 10), None);
        assert_eq!(diff_nav_next_target(&entries, 30), None);

        assert_eq!(diff_nav_prev_target(&entries, 25), Some(20));
        assert_eq!(diff_nav_next_target(&entries, 25), Some(30));

        assert_eq!(diff_nav_next_target(&entries, 0), Some(10));
        assert_eq!(diff_nav_prev_target(&entries, 100), Some(30));
    }

    #[test]
    fn conflict_nav_entries_group_contiguous_changes() {
        let split_rows = vec![
            FileDiffRow {
                kind: K::Context,
                old_line: Some(1),
                new_line: Some(1),
                old: Some("a".into()),
                new: Some("a".into()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: K::Remove,
                old_line: Some(2),
                new_line: None,
                old: Some("b".into()),
                new: None,
                eof_newline: None,
            },
            FileDiffRow {
                kind: K::Add,
                old_line: None,
                new_line: Some(2),
                old: None,
                new: Some("b2".into()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: K::Context,
                old_line: Some(3),
                new_line: Some(3),
                old: Some("c".into()),
                new: Some("c".into()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: K::Modify,
                old_line: Some(4),
                new_line: Some(4),
                old: Some("d".into()),
                new: Some("d2".into()),
                eof_newline: None,
            },
            FileDiffRow {
                kind: K::Context,
                old_line: Some(5),
                new_line: Some(5),
                old: Some("e".into()),
                new: Some("e".into()),
                eof_newline: None,
            },
        ];
        assert_eq!(conflict_nav_entries_for_split(&split_rows), vec![1, 4]);

        let inline_rows = vec![
            ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: DK::Context,
                old_line: Some(1),
                new_line: Some(1),
                content: "a".into(),
            },
            ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: DK::Remove,
                old_line: Some(2),
                new_line: None,
                content: "b".into(),
            },
            ConflictInlineRow {
                side: ConflictPickSide::Theirs,
                kind: DK::Add,
                old_line: None,
                new_line: Some(2),
                content: "b2".into(),
            },
            ConflictInlineRow {
                side: ConflictPickSide::Ours,
                kind: DK::Context,
                old_line: Some(3),
                new_line: Some(3),
                content: "c".into(),
            },
            ConflictInlineRow {
                side: ConflictPickSide::Theirs,
                kind: DK::Add,
                old_line: None,
                new_line: Some(4),
                content: "d2".into(),
            },
        ];
        assert_eq!(conflict_nav_entries_for_inline(&inline_rows), vec![1, 4]);
    }
}
