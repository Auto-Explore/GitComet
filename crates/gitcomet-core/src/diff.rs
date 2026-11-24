use crate::domain::{Diff, DiffLineKind, SharedLineText, parse_unified_hunk_header};

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
        if line.text.raw_bytes().starts_with(b"\\") {
            out.push(AnnotatedDiffLine {
                kind: line.kind,
                text: line.text.clone(),
                old_line: None,
                new_line: None,
            });
            continue;
        }
        match line.kind {
            DiffLineKind::Hunk => {
                if let Some(header) = parse_unified_hunk_header(line.text.as_bytes())
                    && let [old] = header.parents.as_slice()
                {
                    old_line = Some(old.start);
                    new_line = Some(header.result.start);
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
        } else if let Some(rest) = line
            .strip_prefix("rename to ")
            .or_else(|| line.strip_prefix("copy to "))
        {
            // A rename or copy with no content change has no `---`/`+++` pair
            // at all. Unlike those, this name is written bare, with no `a/` or
            // `b/`.
            rename_to = rename_to.or_else(|| Some(unquote_git_path(rest)));
        }
    }

    old_path
        .or(rename_to)
        // Mode-only and binary changes carry neither pair nor rename lines, but
        // both their names are identical, which makes the header unambiguous.
        .or_else(|| parse_diff_git_header_path(header))
}

/// Whether `line` is the header git writes for a rename's source. A copy
/// writes `copy from` instead.
pub fn is_rename_source_header(line: &str) -> bool {
    line.starts_with("rename from ")
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
    if !text.starts_with('"') {
        return None;
    }
    let (decoded, consumed) = gix_quote::ansi_c::undo(text.as_bytes().into()).ok()?;
    Some((
        String::from_utf8_lossy(&decoded).into_owned(),
        text.get(consumed..)?,
    ))
}

#[cfg(test)]
mod tests;
