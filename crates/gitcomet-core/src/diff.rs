use crate::domain::{Diff, DiffLineKind, SharedLineText};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotatedDiffLine {
    pub kind: DiffLineKind,
    pub text: SharedLineText,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

pub fn annotate_unified(diff: &Diff) -> Vec<AnnotatedDiffLine> {
    let mut old_line: Option<u32> = None;
    let mut new_line: Option<u32> = None;

    let mut out = Vec::with_capacity(diff.lines.len());
    for line in &diff.lines {
        match line.kind {
            DiffLineKind::Hunk => {
                if let Some((old_start, new_start)) = parse_unified_hunk_header(&line.text) {
                    old_line = Some(old_start);
                    new_line = Some(new_start);
                } else {
                    old_line = None;
                    new_line = None;
                }

                out.push(AnnotatedDiffLine {
                    kind: line.kind,
                    text: line.text.clone(),
                    old_line: None,
                    new_line: None,
                });
            }
            DiffLineKind::Context => {
                let current_old = old_line;
                let current_new = new_line;
                if let Some(v) = old_line.as_mut() {
                    *v += 1;
                }
                if let Some(v) = new_line.as_mut() {
                    *v += 1;
                }
                out.push(AnnotatedDiffLine {
                    kind: line.kind,
                    text: line.text.clone(),
                    old_line: current_old,
                    new_line: current_new,
                });
            }
            DiffLineKind::Remove => {
                let current_old = old_line;
                if let Some(v) = old_line.as_mut() {
                    *v += 1;
                }
                out.push(AnnotatedDiffLine {
                    kind: line.kind,
                    text: line.text.clone(),
                    old_line: current_old,
                    new_line: None,
                });
            }
            DiffLineKind::Add => {
                let current_new = new_line;
                if let Some(v) = new_line.as_mut() {
                    *v += 1;
                }
                out.push(AnnotatedDiffLine {
                    kind: line.kind,
                    text: line.text.clone(),
                    old_line: None,
                    new_line: current_new,
                });
            }
            DiffLineKind::Header => out.push(AnnotatedDiffLine {
                kind: line.kind,
                text: line.text.clone(),
                old_line: None,
                new_line: None,
            }),
        }
    }

    out
}

fn parse_unified_hunk_header(text: &str) -> Option<(u32, u32)> {
    // Formats:
    // @@ -l,s +l,s @@
    // @@ -l +l @@
    // @@ -l,0 +l,0 @@
    let text = text.strip_prefix("@@")?.trim_start();
    let text = text.split("@@").next()?.trim();

    let mut it = text.split_whitespace();
    let old = it.next()?;
    let new = it.next()?;

    let old_start = parse_range_start(old.strip_prefix('-')?)?;
    let new_start = parse_range_start(new.strip_prefix('+')?)?;
    Some((old_start, new_start))
}

fn parse_range_start(s: &str) -> Option<u32> {
    let start = s.split(',').next()?;
    start.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DiffArea, DiffTarget};
    use std::path::PathBuf;

    #[test]
    fn annotate_tracks_line_numbers_through_hunks() {
        let diff = Diff::from_unified(
            DiffTarget::WorkingTree {
                path: PathBuf::from("src/lib.rs"),
                area: DiffArea::Unstaged,
            },
            "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,4 @@ fn main() {
 line1
-line2
+line2 changed
 line3
+line4
",
        );

        let annotated = annotate_unified(&diff);
        let mut rows = annotated
            .iter()
            .filter(|l| {
                matches!(
                    l.kind,
                    DiffLineKind::Context | DiffLineKind::Add | DiffLineKind::Remove
                )
            })
            .map(|l| (l.kind, l.old_line, l.new_line, l.text.as_ref()))
            .collect::<Vec<_>>();

        // Context lines include a leading space in unified diff.
        // `Diff::from_unified` keeps the raw line text.
        assert_eq!(
            rows.remove(0),
            (DiffLineKind::Context, Some(10), Some(10), " line1")
        );
        assert_eq!(
            rows.remove(0),
            (DiffLineKind::Remove, Some(11), None, "-line2")
        );
        assert_eq!(
            rows.remove(0),
            (DiffLineKind::Add, None, Some(11), "+line2 changed")
        );
        assert_eq!(
            rows.remove(0),
            (DiffLineKind::Context, Some(12), Some(12), " line3")
        );
        assert_eq!(
            rows.remove(0),
            (DiffLineKind::Add, None, Some(13), "+line4")
        );
    }

    #[test]
    fn parse_hunk_header_variants() {
        assert_eq!(parse_unified_hunk_header("@@ -1 +2 @@"), Some((1, 2)));
        assert_eq!(parse_unified_hunk_header("@@ -1,0 +2,10 @@"), Some((1, 2)));
        assert_eq!(
            parse_unified_hunk_header("@@ -42,7 +100,8 @@ fn x"),
            Some((42, 100))
        );
    }
}
