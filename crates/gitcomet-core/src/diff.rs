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

/// Post-image path of the file section whose `diff --git` line is the first of
/// `section_lines`. Scanning stops at the section's first hunk or at the next
/// file's header, so callers can simply hand over the rest of the diff.
///
/// The `diff --git` line alone is ambiguous: git separates the two names with a
/// space and does not escape spaces inside them, so `a/one two b/one two` can be
/// split in three ways. Git resolves that by writing the names again on the
/// `---` / `+++` lines, one per line and terminated by a TAB when they contain a
/// space, and this follows the same order of preference.
pub fn unified_diff_file_path<'a>(
    section_lines: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut lines = section_lines.into_iter();
    let header = lines.next()?;

    let mut old_path = None;
    let mut rename_to = None;
    for line in lines {
        // The `---`/`+++` pair always precedes the first hunk, and a second
        // `diff --git` line starts a section this one's names cannot come from.
        if line.starts_with("@@") || line.starts_with("diff --git ") {
            break;
        }
        if line.starts_with("+++ ") {
            // The post-image name is the one every caller wants, so it wins
            // outright — except for a deletion, where it is `/dev/null`.
            if let Some(path) = parse_unified_path_header(line) {
                return Some(path);
            }
        } else if line.starts_with("--- ") {
            old_path = old_path.or_else(|| parse_unified_path_header(line));
        } else if let Some(rest) = line.strip_prefix("rename to ") {
            // A rename with no content change has no `---`/`+++` pair at all.
            // Unlike those, this name is written bare, with no `a/` or `b/`.
            rename_to = rename_to.or_else(|| Some(unquote_git_path(rest)));
        }
    }

    old_path
        .or(rename_to)
        // Mode-only and binary changes carry neither pair nor rename lines, but
        // both their names are identical, which makes the header unambiguous.
        .or_else(|| parse_diff_git_header_path(header))
}

/// Path on a `--- <name>` or `+++ <name>` line, or `None` for `/dev/null`.
fn parse_unified_path_header(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("--- ")
        .or_else(|| line.strip_prefix("+++ "))?;
    // git appends a TAB after names that need disambiguating; other producers
    // append `\t<timestamp>`. A TAB inside a name is always escaped as `\t`
    // within double quotes, so no real name can contain one here.
    let rest = rest.split('\t').next().unwrap_or(rest);
    let name = unquote_git_path(rest);
    if name == "/dev/null" {
        return None;
    }
    Some(strip_diff_path_prefix(&name))
}

/// Path shared by both halves of a `diff --git` line, using git's own rule from
/// `git_header_name`: a split is only believed when the two names it produces
/// are identical. Renames therefore yield `None` by design.
fn parse_diff_git_header_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    if rest.is_empty() {
        return None;
    }

    if rest.starts_with('"') {
        let (first, after) = split_quoted_git_path(rest)?;
        let second = after.trim_start();
        if second.is_empty() {
            return None;
        }
        let second = if second.starts_with('"') {
            split_quoted_git_path(second)?.0
        } else {
            second.to_string()
        };
        let first = strip_diff_path_prefix(&first);
        let second = strip_diff_path_prefix(&second);
        return (first == second).then_some(second);
    }

    // A double quote can only start the second name, since the first is not
    // quoted and git escapes quotes inside quoted names.
    if let Some(quote_ix) = rest.find('"')
        && rest[..quote_ix].ends_with([' ', '\t'])
    {
        let first = strip_diff_path_prefix(rest[..quote_ix].trim_end_matches([' ', '\t']));
        let second = strip_diff_path_prefix(&split_quoted_git_path(&rest[quote_ix..])?.0);
        return (first == second).then_some(second);
    }

    // Unquoted on both sides: every space and TAB is a candidate separator, and
    // the right one is where the two names come out equal.
    rest.char_indices()
        .filter(|(_, ch)| *ch == ' ' || *ch == '\t')
        .find_map(|(ix, _)| {
            let first = strip_diff_path_prefix(&rest[..ix]);
            let second = strip_diff_path_prefix(&rest[ix + 1..]);
            (!second.is_empty() && first == second).then_some(second)
        })
}

/// Strips the `a/` or `b/` diff prefix, leaving names that carry no prefix
/// (`diff.noprefix`) untouched.
fn strip_diff_path_prefix(name: &str) -> String {
    name.strip_prefix("a/")
        .or_else(|| name.strip_prefix("b/"))
        .unwrap_or(name)
        .to_string()
}

/// C-unquotes `name` when git wrote it quoted, and returns it unchanged
/// otherwise.
fn unquote_git_path(name: &str) -> String {
    if name.starts_with('"') {
        split_quoted_git_path(name)
            .map(|(unquoted, _)| unquoted)
            .unwrap_or_else(|| name.to_string())
    } else {
        name.to_string()
    }
}

/// Decodes a leading C-quoted name as written by git's `quote_c_style`, and
/// returns it together with whatever follows the closing quote.
fn split_quoted_git_path(text: &str) -> Option<(String, &str)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }

    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut ix = 1usize;
    while ix < bytes.len() {
        match bytes[ix] {
            b'"' => {
                let rest = text.get(ix + 1..)?;
                return Some((String::from_utf8_lossy(&out).into_owned(), rest));
            }
            b'\\' => {
                ix += 1;
                let escape = *bytes.get(ix)?;
                match escape {
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'v' => out.push(0x0b),
                    b'0'..=b'7' => {
                        // Up to three octal digits, as git emits them.
                        let mut value = u32::from(escape - b'0');
                        let mut digits = 1;
                        while digits < 3
                            && let Some(next) = bytes.get(ix + 1)
                            && (b'0'..=b'7').contains(next)
                        {
                            value = value * 8 + u32::from(next - b'0');
                            ix += 1;
                            digits += 1;
                        }
                        out.push(u8::try_from(value).ok()?);
                    }
                    other => out.push(other),
                }
                ix += 1;
            }
            other => {
                out.push(other);
                ix += 1;
            }
        }
    }

    // Unterminated quote: not something git produces.
    None
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

    fn file_path_of(section: &str) -> Option<String> {
        unified_diff_file_path(section.lines())
    }

    #[test]
    fn file_path_reads_a_plain_modification() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
",
            ),
            Some("src/lib.rs".to_string())
        );
    }

    /// The whole point of the header pair: `diff --git` alone cannot be split
    /// correctly here, but the TAB-terminated `+++` line is unambiguous.
    #[test]
    fn file_path_reads_a_name_containing_spaces() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/src/rules - Copy - Copy.rs b/src/rules - Copy - Copy.rs
index 313043d..74a9311 100644
--- a/src/rules - Copy - Copy.rs\t
+++ b/src/rules - Copy - Copy.rs\t
@@ -1 +1 @@
",
            ),
            Some("src/rules - Copy - Copy.rs".to_string())
        );
    }

    /// Without any `---`/`+++` pair the header line has to carry the name, and
    /// there both halves are identical whatever spaces they contain.
    #[test]
    fn file_path_falls_back_to_the_git_header_line() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/src/lib.rs b/src/lib.rs
old mode 100644
new mode 100755
",
            ),
            Some("src/lib.rs".to_string())
        );
        assert_eq!(
            file_path_of(
                "\
diff --git a/my notes.md b/my notes.md
old mode 100644
new mode 100755
",
            ),
            Some("my notes.md".to_string())
        );
        assert_eq!(
            file_path_of(
                "\
diff --git a/logo.png b/logo.png
index 1111111..2222222 100644
Binary files a/logo.png and b/logo.png differ
",
            ),
            Some("logo.png".to_string())
        );
    }

    #[test]
    fn file_path_prefers_the_post_image_name_of_a_rename() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/src/rules.rs b/src/rules - Copy.rs
similarity index 94%
rename from src/rules.rs
rename to src/rules - Copy.rs
index 1e0101a..313043d 100644
--- a/src/rules.rs
+++ b/src/rules - Copy.rs\t
@@ -1 +1 @@
",
            ),
            Some("src/rules - Copy.rs".to_string())
        );
    }

    /// A rename with no content change carries neither `---` nor `+++`, and its
    /// header halves differ, so only `rename to` can answer.
    #[test]
    fn file_path_reads_a_pure_rename() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
",
            ),
            Some("new name.txt".to_string())
        );
    }

    #[test]
    fn file_path_handles_added_and_deleted_files() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/new file.txt b/new file.txt
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/new file.txt\t
@@ -0,0 +1 @@
",
            ),
            Some("new file.txt".to_string())
        );
        assert_eq!(
            file_path_of(
                "\
diff --git a/gone file.txt b/gone file.txt
deleted file mode 100644
index 1111111..0000000
--- a/gone file.txt\t
+++ /dev/null
@@ -1 +0,0 @@
",
            ),
            Some("gone file.txt".to_string())
        );
    }

    /// With `core.quotePath` left at its default, git octal-escapes non-ASCII
    /// bytes and wraps the name in double quotes.
    #[test]
    fn file_path_unquotes_c_style_names() {
        assert_eq!(
            file_path_of(
                "\
diff --git \"a/h\\303\\251llo.txt\" \"b/h\\303\\251llo.txt\"
index 1111111..2222222 100644
--- \"a/h\\303\\251llo.txt\"
+++ \"b/h\\303\\251llo.txt\"
@@ -1 +1 @@
",
            ),
            Some("héllo.txt".to_string())
        );
        // Quotes and backslashes stay quoted whatever `core.quotePath` says.
        assert_eq!(
            file_path_of(
                "\
diff --git \"a/we\\\"ird\\tname.txt\" \"b/we\\\"ird\\tname.txt\"
index 1111111..2222222 100644
--- \"a/we\\\"ird\\tname.txt\"
+++ \"b/we\\\"ird\\tname.txt\"
@@ -1 +1 @@
",
            ),
            Some("we\"ird\tname.txt".to_string())
        );
    }

    /// `a/` and `b/` are ordinary directory names, and only the header lines
    /// carry them as prefixes — `rename to` writes the path bare.
    #[test]
    fn file_path_keeps_a_leading_directory_called_a_or_b() {
        assert_eq!(
            file_path_of(
                "\
diff --git a/a/notes.md b/a/notes.md
index 1111111..2222222 100644
--- a/a/notes.md
+++ b/a/notes.md
@@ -1 +1 @@
",
            ),
            Some("a/notes.md".to_string())
        );
        assert_eq!(
            file_path_of(
                "\
diff --git a/b/old.md b/b/new name.md
similarity index 100%
rename from b/old.md
rename to b/new name.md
",
            ),
            Some("b/new name.md".to_string())
        );
        assert_eq!(
            file_path_of(
                "\
diff --git a/a/notes.md b/a/notes.md
old mode 100644
new mode 100755
",
            ),
            Some("a/notes.md".to_string())
        );
    }

    #[test]
    fn file_path_keeps_names_written_without_a_prefix() {
        assert_eq!(
            file_path_of(
                "\
diff --git src/lib.rs src/lib.rs
index 1111111..2222222 100644
--- src/lib.rs
+++ src/lib.rs
@@ -1 +1 @@
",
            ),
            Some("src/lib.rs".to_string())
        );
    }

    /// Scanning must not run past the section it was asked about.
    #[test]
    fn file_path_stops_at_the_next_section_and_at_the_first_hunk() {
        let two_files = "\
diff --git a/first.txt b/first.txt
old mode 100644
new mode 100755
diff --git a/second.txt b/second.txt
--- a/second.txt
+++ b/second.txt
@@ -1 +1 @@
";
        assert_eq!(
            unified_diff_file_path(two_files.lines()),
            Some("first.txt".to_string())
        );

        // A `+++ ` inside hunk content must not be mistaken for a header.
        assert_eq!(
            file_path_of(
                "\
diff --git a/notes.txt b/notes.txt
--- a/notes.txt
+++ b/notes.txt
@@ -1,2 +1,2 @@
-+++ b/decoy.txt
++++ b/other decoy.txt
",
            ),
            Some("notes.txt".to_string())
        );
    }

    #[test]
    fn file_path_rejects_malformed_headers() {
        for text in [
            "",
            "diff --git",
            "diff --git ",
            "index 1111111..2222222 100644",
            "diff --git a/one.txt b/two.txt",
        ] {
            assert_eq!(unified_diff_file_path([text]), None, "{text:?}");
        }
    }
}
