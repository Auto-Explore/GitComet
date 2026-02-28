#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffRowKind {
    Context,
    Add,
    Remove,
    Modify,
}

const REPLACEMENT_ALIGN_CELL_BUDGET: usize = 50_000;
const REPLACEMENT_GAP_COST: u32 = 100;
const REPLACEMENT_PAIR_BASE_COST: u32 = 80;
const REPLACEMENT_PAIR_SCALE_COST: u32 = 120;
const REPLACEMENT_DISSIMILAR_PENALTY_COST: u32 = 40;
const REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffEofNewline {
    MissingInOld,
    MissingInNew,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDiffRow {
    pub kind: FileDiffRowKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub old: Option<String>,
    pub new: Option<String>,
    pub eof_newline: Option<FileDiffEofNewline>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditKind {
    Equal,
    Insert,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Edit<'a> {
    pub(crate) kind: EditKind,
    pub(crate) old: Option<&'a str>,
    pub(crate) new: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplacementAlignStep {
    None,
    Pair,
    Delete,
    Insert,
}

pub fn side_by_side_rows(old: &str, new: &str) -> Vec<FileDiffRow> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);

    let edits = myers_edits(&old_lines, &new_lines);

    let mut raw = Vec::with_capacity(edits.len());
    let mut old_ln: u32 = 1;
    let mut new_ln: u32 = 1;

    for e in edits {
        match e.kind {
            EditKind::Equal => {
                let text = e.old.unwrap_or_default();
                raw.push(FileDiffRow {
                    kind: FileDiffRowKind::Context,
                    old_line: Some(old_ln),
                    new_line: Some(new_ln),
                    old: Some(text.to_string()),
                    new: Some(text.to_string()),
                    eof_newline: None,
                });
                old_ln = old_ln.saturating_add(1);
                new_ln = new_ln.saturating_add(1);
            }
            EditKind::Delete => {
                raw.push(FileDiffRow {
                    kind: FileDiffRowKind::Remove,
                    old_line: Some(old_ln),
                    new_line: None,
                    old: Some(e.old.unwrap_or_default().to_string()),
                    new: None,
                    eof_newline: None,
                });
                old_ln = old_ln.saturating_add(1);
            }
            EditKind::Insert => {
                raw.push(FileDiffRow {
                    kind: FileDiffRowKind::Add,
                    old_line: None,
                    new_line: Some(new_ln),
                    old: None,
                    new: Some(e.new.unwrap_or_default().to_string()),
                    eof_newline: None,
                });
                new_ln = new_ln.saturating_add(1);
            }
        }
    }

    annotate_eof_newline(pair_replacements(raw), old, new)
}

fn pair_replacements(rows: Vec<FileDiffRow>) -> Vec<FileDiffRow> {
    let mut out = Vec::with_capacity(rows.len());
    let mut ix = 0usize;

    while ix < rows.len() {
        if rows[ix].kind != FileDiffRowKind::Remove {
            out.push(rows[ix].clone());
            ix += 1;
            continue;
        }

        let del_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Remove {
            ix += 1;
        }
        let del_end = ix;

        let ins_start = ix;
        while ix < rows.len() && rows[ix].kind == FileDiffRowKind::Add {
            ix += 1;
        }
        let ins_end = ix;

        if ins_start == ins_end {
            out.extend(rows[del_start..del_end].iter().cloned());
            continue;
        }

        out.extend(align_replacement_runs(
            &rows[del_start..del_end],
            &rows[ins_start..ins_end],
        ));
    }

    out
}

fn align_replacement_runs(deletes: &[FileDiffRow], inserts: &[FileDiffRow]) -> Vec<FileDiffRow> {
    if deletes.is_empty() {
        return inserts.to_vec();
    }
    if inserts.is_empty() {
        return deletes.to_vec();
    }

    if deletes.len().saturating_mul(inserts.len()) > REPLACEMENT_ALIGN_CELL_BUDGET {
        return pair_replacement_runs_by_position(deletes, inserts);
    }

    let n = deletes.len();
    let m = inserts.len();
    let width = m + 1;
    let mut cost = vec![u32::MAX / 4; (n + 1) * width];
    let mut step = vec![ReplacementAlignStep::None; (n + 1) * width];
    cost[0] = 0;

    for i in 1..=n {
        let idx = i * width;
        cost[idx] = (i as u32) * REPLACEMENT_GAP_COST;
        step[idx] = ReplacementAlignStep::Delete;
    }
    for j in 1..=m {
        cost[j] = (j as u32) * REPLACEMENT_GAP_COST;
        step[j] = ReplacementAlignStep::Insert;
    }

    for i in 1..=n {
        for j in 1..=m {
            let idx = i * width + j;
            let del_idx = (i - 1) * width + j;
            let ins_idx = i * width + (j - 1);
            let pair_idx = (i - 1) * width + (j - 1);

            let pair_cost = cost[pair_idx].saturating_add(replacement_pair_cost(
                deletes[i - 1].old.as_deref().unwrap_or_default(),
                inserts[j - 1].new.as_deref().unwrap_or_default(),
            ));
            let insert_cost = cost[ins_idx].saturating_add(REPLACEMENT_GAP_COST);
            let delete_cost = cost[del_idx].saturating_add(REPLACEMENT_GAP_COST);

            let mut best_cost = pair_cost;
            let mut best_step = ReplacementAlignStep::Pair;

            if insert_cost < best_cost {
                best_cost = insert_cost;
                best_step = ReplacementAlignStep::Insert;
            }
            if delete_cost < best_cost {
                best_cost = delete_cost;
                best_step = ReplacementAlignStep::Delete;
            }

            cost[idx] = best_cost;
            step[idx] = best_step;
        }
    }

    let mut i = n;
    let mut j = m;
    let mut aligned_rev = Vec::with_capacity(n + m);
    while i > 0 || j > 0 {
        let idx = i * width + j;
        match step[idx] {
            ReplacementAlignStep::Pair if i > 0 && j > 0 => {
                aligned_rev.push(make_modify_row(&deletes[i - 1], &inserts[j - 1]));
                i -= 1;
                j -= 1;
            }
            ReplacementAlignStep::Insert if j > 0 => {
                aligned_rev.push(inserts[j - 1].clone());
                j -= 1;
            }
            ReplacementAlignStep::Delete if i > 0 => {
                aligned_rev.push(deletes[i - 1].clone());
                i -= 1;
            }
            _ if j > 0 => {
                aligned_rev.push(inserts[j - 1].clone());
                j -= 1;
            }
            _ if i > 0 => {
                aligned_rev.push(deletes[i - 1].clone());
                i -= 1;
            }
            _ => break,
        }
    }

    aligned_rev.reverse();
    aligned_rev
}

fn pair_replacement_runs_by_position(
    deletes: &[FileDiffRow],
    inserts: &[FileDiffRow],
) -> Vec<FileDiffRow> {
    let paired = deletes.len().min(inserts.len());
    let mut out = Vec::with_capacity(deletes.len() + inserts.len());

    for i in 0..paired {
        out.push(make_modify_row(&deletes[i], &inserts[i]));
    }
    if deletes.len() > paired {
        out.extend(deletes[paired..].iter().cloned());
    }
    if inserts.len() > paired {
        out.extend(inserts[paired..].iter().cloned());
    }
    out
}

fn make_modify_row(delete: &FileDiffRow, insert: &FileDiffRow) -> FileDiffRow {
    FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: delete.old_line,
        new_line: insert.new_line,
        old: delete.old.clone(),
        new: insert.new.clone(),
        eof_newline: None,
    }
}

fn replacement_pair_cost(old: &str, new: &str) -> u32 {
    if old == new {
        return 0;
    }

    let max_len_usize = old.chars().count().max(new.chars().count()).max(1);
    let max_len = max_len_usize as u32;
    let distance = levenshtein_distance(old, new) as u32;
    let (shared_prefix, shared_suffix) = shared_boundary_bytes(old, new);

    let mut cost = REPLACEMENT_PAIR_BASE_COST
        + distance
            .min(max_len)
            .saturating_mul(REPLACEMENT_PAIR_SCALE_COST)
            / max_len;
    if shared_prefix == 0
        && shared_suffix == 0
        && max_len_usize >= REPLACEMENT_DISSIMILAR_PENALTY_MIN_LEN
    {
        cost = cost.saturating_add(REPLACEMENT_DISSIMILAR_PENALTY_COST);
    }

    cost
}

fn shared_boundary_bytes(a: &str, b: &str) -> (usize, usize) {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut prefix = 0usize;
    while prefix < a_bytes.len() && prefix < b_bytes.len() && a_bytes[prefix] == b_bytes[prefix] {
        prefix += 1;
    }

    let max_suffix = a_bytes.len().min(b_bytes.len()).saturating_sub(prefix);
    let mut suffix = 0usize;
    while suffix < max_suffix
        && a_bytes[a_bytes.len() - 1 - suffix] == b_bytes[b_bytes.len() - 1 - suffix]
    {
        suffix += 1;
    }

    (prefix, suffix)
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }

    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr: Vec<usize> = vec![0; b_chars.len() + 1];

    for (i, a_ch) in a_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, b_ch) in b_chars.iter().enumerate() {
            let subst = prev[j] + usize::from(a_ch != b_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = subst.min(insert).min(delete);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[b_chars.len()]
}

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    // Keep row tokenization line-oriented; EOF newline deltas are annotated separately.
    text.lines().collect()
}

fn annotate_eof_newline(
    mut rows: Vec<FileDiffRow>,
    old_text: &str,
    new_text: &str,
) -> Vec<FileDiffRow> {
    let Some(marker) = eof_newline_delta(old_text, new_text) else {
        return rows;
    };

    if let Some(last) = rows.last_mut() {
        // EOF newline changes are semantic file changes, even when the text on the
        // final line is otherwise equal.
        if last.kind == FileDiffRowKind::Context {
            last.kind = FileDiffRowKind::Modify;
        }
        last.eof_newline = Some(marker);
        return rows;
    }

    rows.push(FileDiffRow {
        kind: FileDiffRowKind::Modify,
        old_line: None,
        new_line: None,
        old: None,
        new: None,
        eof_newline: Some(marker),
    });
    rows
}

fn eof_newline_delta(old_text: &str, new_text: &str) -> Option<FileDiffEofNewline> {
    let old_has_newline = old_text.ends_with('\n');
    let new_has_newline = new_text.ends_with('\n');
    match (old_has_newline, new_has_newline) {
        (false, true) => Some(FileDiffEofNewline::MissingInOld),
        (true, false) => Some(FileDiffEofNewline::MissingInNew),
        _ => None,
    }
}

pub(crate) fn myers_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
    let n = old.len() as isize;
    let m = new.len() as isize;
    let max = (n + m) as usize;
    let offset = max as isize;

    let mut v = vec![0isize; 2 * max + 1];
    let mut trace: Vec<Vec<isize>> = Vec::with_capacity(max + 1);
    {
        let mut x = 0isize;
        let mut y = 0isize;
        while x < n && y < m && old[x as usize] == new[y as usize] {
            x += 1;
            y += 1;
        }
        v[offset as usize] = x;
    }
    trace.push(v.clone());

    let mut last_d = 0usize;
    if v[offset as usize] >= n && v[offset as usize] >= m {
        last_d = 0;
    } else {
        'outer: for d in 1..=max {
            let d_isize = d as isize;
            let mut next = v.clone();

            for k in (-d_isize..=d_isize).step_by(2) {
                let k_idx = (offset + k) as usize;

                let x = if k == -d_isize
                    || (k != d_isize && v[(offset + k - 1) as usize] < v[(offset + k + 1) as usize])
                {
                    v[(offset + k + 1) as usize]
                } else {
                    v[(offset + k - 1) as usize] + 1
                };

                let mut x = x;
                let mut y = x - k;
                while x < n && y < m && old[x as usize] == new[y as usize] {
                    x += 1;
                    y += 1;
                }
                next[k_idx] = x;

                if x >= n && y >= m {
                    v = next;
                    trace.push(v.clone());
                    last_d = d;
                    break 'outer;
                }
            }

            v = next;
            trace.push(v.clone());
        }
    }

    if n == 0 && m == 0 {
        return Vec::new();
    }

    if last_d == 0 && n == m && v[offset as usize] == n {
        return old
            .iter()
            .map(|&s| Edit {
                kind: EditKind::Equal,
                old: Some(s),
                new: Some(s),
            })
            .collect();
    }

    let mut x = n;
    let mut y = m;
    let mut rev: Vec<Edit<'a>> = Vec::with_capacity(last_d + (n + m) as usize);

    for d in (1..=last_d).rev() {
        let v = &trace[d - 1];
        let d_isize = d as isize;
        let k = x - y;

        let prev_k = if k == -d_isize
            || (k != d_isize && v[(offset + k - 1) as usize] < v[(offset + k + 1) as usize])
        {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v[(offset + prev_k) as usize];
        let prev_y = prev_x - prev_k;

        while x > prev_x && y > prev_y {
            rev.push(Edit {
                kind: EditKind::Equal,
                old: Some(old[(x - 1) as usize]),
                new: Some(new[(y - 1) as usize]),
            });
            x -= 1;
            y -= 1;
        }

        if x == prev_x {
            rev.push(Edit {
                kind: EditKind::Insert,
                old: None,
                new: Some(new[(y - 1) as usize]),
            });
            y -= 1;
        } else {
            rev.push(Edit {
                kind: EditKind::Delete,
                old: Some(old[(x - 1) as usize]),
                new: None,
            });
            x -= 1;
        }
    }

    while x > 0 && y > 0 {
        rev.push(Edit {
            kind: EditKind::Equal,
            old: Some(old[(x - 1) as usize]),
            new: Some(new[(y - 1) as usize]),
        });
        x -= 1;
        y -= 1;
    }
    while x > 0 {
        rev.push(Edit {
            kind: EditKind::Delete,
            old: Some(old[(x - 1) as usize]),
            new: None,
        });
        x -= 1;
    }
    while y > 0 {
        rev.push(Edit {
            kind: EditKind::Insert,
            old: None,
            new: Some(new[(y - 1) as usize]),
        });
        y -= 1;
    }

    rev.reverse();
    rev
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remove_row(old_line: u32, old: &str) -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffRowKind::Remove,
            old_line: Some(old_line),
            new_line: None,
            old: Some(old.to_string()),
            new: None,
            eof_newline: None,
        }
    }

    fn add_row(new_line: u32, new: &str) -> FileDiffRow {
        FileDiffRow {
            kind: FileDiffRowKind::Add,
            old_line: None,
            new_line: Some(new_line),
            old: None,
            new: Some(new.to_string()),
            eof_newline: None,
        }
    }

    #[test]
    fn pairs_delete_insert_into_modify_rows() {
        let old = "a\nb\nc\n";
        let new = "a\nb2\nc\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Context,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Context
            ]
        );

        let mid = &rows[1];
        assert_eq!(mid.old.as_deref(), Some("b"));
        assert_eq!(mid.new.as_deref(), Some("b2"));
        assert_eq!(mid.old_line, Some(2));
        assert_eq!(mid.new_line, Some(2));
        assert_eq!(mid.eof_newline, None);
    }

    #[test]
    fn handles_additions_and_deletions() {
        let old = "a\nb\n";
        let new = "a\nb\nc\n";
        let rows = side_by_side_rows(old, new);
        assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Add));

        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        let rows = side_by_side_rows(old, new);
        assert!(rows.iter().any(|r| r.kind == FileDiffRowKind::Remove));
    }

    #[test]
    fn marks_missing_newline_in_new_file() {
        let old = "a\n";
        let new = "a";

        let rows = side_by_side_rows(old, new);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
        assert_eq!(rows[0].old.as_deref(), Some("a"));
        assert_eq!(rows[0].new.as_deref(), Some("a"));
        assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInNew));
    }

    #[test]
    fn marks_missing_newline_in_old_file() {
        let old = "a";
        let new = "a\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, FileDiffRowKind::Modify);
        assert_eq!(rows[0].old.as_deref(), Some("a"));
        assert_eq!(rows[0].new.as_deref(), Some("a"));
        assert_eq!(rows[0].eof_newline, Some(FileDiffEofNewline::MissingInOld));
    }

    #[test]
    fn preserves_existing_modify_rows_when_eof_newline_differs() {
        let old = "a\nb\n";
        let new = "a\nc";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            vec![FileDiffRowKind::Context, FileDiffRowKind::Modify]
        );
        assert_eq!(rows[1].old.as_deref(), Some("b"));
        assert_eq!(rows[1].new.as_deref(), Some("c"));
        assert_eq!(rows[1].eof_newline, Some(FileDiffEofNewline::MissingInNew));
    }

    #[test]
    fn asymmetric_replacement_pairs_best_matching_lines() {
        let rows = vec![
            remove_row(10, "alpha"),
            remove_row(11, "beta"),
            add_row(20, "intro"),
            add_row(21, "alpha changed"),
            add_row(22, "beta changed"),
        ];

        let paired = pair_replacements(rows);
        assert_eq!(
            paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Add,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Modify
            ]
        );
        assert_eq!(paired[0].new.as_deref(), Some("intro"));
        assert_eq!(paired[1].old.as_deref(), Some("alpha"));
        assert_eq!(paired[1].new.as_deref(), Some("alpha changed"));
        assert_eq!(paired[2].old.as_deref(), Some("beta"));
        assert_eq!(paired[2].new.as_deref(), Some("beta changed"));
    }

    #[test]
    fn dissimilar_single_line_replacement_stays_add_remove() {
        let rows = vec![remove_row(1, "aaaaaaaa"), add_row(1, "zzzzzzzz")];
        let paired = pair_replacements(rows);

        assert_eq!(
            paired.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![FileDiffRowKind::Remove, FileDiffRowKind::Add]
        );
    }

    #[test]
    fn side_by_side_aligns_asymmetric_replacement_in_context() {
        let old = "start\nalpha\nbeta\nend\n";
        let new = "start\nintro\nalpha changed\nbeta changed\nend\n";

        let rows = side_by_side_rows(old, new);
        assert_eq!(
            rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
            vec![
                FileDiffRowKind::Context,
                FileDiffRowKind::Add,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Modify,
                FileDiffRowKind::Context
            ]
        );
    }
}
