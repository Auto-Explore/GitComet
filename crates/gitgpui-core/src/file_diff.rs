#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDiffRowKind {
    Context,
    Add,
    Remove,
    Modify,
}

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
enum EditKind {
    Equal,
    Insert,
    Delete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Edit<'a> {
    kind: EditKind,
    old: Option<&'a str>,
    new: Option<&'a str>,
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

        let del_len = del_end - del_start;
        let ins_len = ins_end - ins_start;
        let paired = del_len.min(ins_len);

        for i in 0..paired {
            let d = &rows[del_start + i];
            let a = &rows[ins_start + i];
            out.push(FileDiffRow {
                kind: FileDiffRowKind::Modify,
                old_line: d.old_line,
                new_line: a.new_line,
                old: d.old.clone(),
                new: a.new.clone(),
                eof_newline: None,
            });
        }

        if del_len > paired {
            out.extend(rows[(del_start + paired)..del_end].iter().cloned());
        }
        if ins_len > paired {
            out.extend(rows[(ins_start + paired)..ins_end].iter().cloned());
        }
    }

    out
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }

    // Keep this simple: we render by rows and don't currently model "missing trailing newline".
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

fn myers_edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Edit<'a>> {
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
}
