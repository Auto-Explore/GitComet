//! Turning a repo-relative worktree path into a `.gitignore` pattern.
//!
//! The escaping here is the whole reason this lives in `gitcomet-core` rather
//! than next to the dialog that uses it: a pattern that looks right and matches
//! nothing is indistinguishable from a working one until the user notices the
//! file is still in the status list. `build/out[1].log` written verbatim is a
//! character class, not a filename.

use rustc_hash::FxHashSet;
use std::path::{Component, Path};

/// The ignore file this module writes to, relative to the repository root.
pub const FILE_NAME: &str = ".gitignore";

/// Marker the append command puts in its command output when every pattern was
/// already present, so nothing was written.
///
/// A shared constant rather than a prose match at each end: the summary shown to
/// the user hinges on it, and "Added X to .gitignore" for a run that touched
/// nothing is a lie the user only catches by opening the file.
pub const NOTHING_TO_ADD: &str = "Already present; nothing to add";

/// Which slice of a path a generated pattern covers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum GitignoreScope {
    /// Just this file, anchored to the repository root.
    File,
    /// The file's immediate parent directory, anchored to the repository root.
    Folder,
    /// Every file sharing this file's extension, at any depth.
    Extension,
}

/// Build the `.gitignore` line for `relative_path` under `scope`.
///
/// `None` when the scope does not apply (a root-level file has no folder to
/// ignore, an extensionless file has no extension to ignore) or the path is not
/// something we can safely write a pattern for.
pub fn pattern_for(relative_path: &Path, scope: GitignoreScope) -> Option<String> {
    let segments = relative_segments(relative_path)?;

    match scope {
        GitignoreScope::File => Some(escape_trailing_spaces(&anchored(&segments))),
        GitignoreScope::Folder => {
            // `segments.len() == 1` is a file sitting directly in the root:
            // its "folder" is the whole repository, which is never what the
            // user means.
            let parent = segments.get(..segments.len().checked_sub(1)?)?;
            if parent.is_empty() {
                return None;
            }
            Some(format!("{}/", anchored(parent)))
        }
        GitignoreScope::Extension => {
            let extension = extension_of(segments.last()?)?;
            Some(escape_trailing_spaces(&format!(
                "*.{}",
                escape_literal(extension)
            )))
        }
    }
}

/// The patterns offerable for a whole status selection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitignoreSuggestions {
    /// One anchored pattern per selected path, in selection order.
    files: Vec<String>,
    /// `/dir/`, only when every path shares one immediate parent.
    folder: Option<String>,
    /// `*.ext`, only when every path shares one extension.
    extension: Option<String>,
}

/// Build the suggestions for a status selection.
///
/// `None` when the selection is empty or any member cannot be expressed as a
/// pattern — one unusable path disqualifies the whole action rather than
/// quietly ignoring a subset of what the user selected.
pub fn suggestions_for_paths(paths: &[std::path::PathBuf]) -> Option<GitignoreSuggestions> {
    if paths.is_empty() {
        return None;
    }

    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        files.push(pattern_for(path, GitignoreScope::File)?);
    }

    // Folder and extension are offered only when every path agrees. Deriving
    // either from a mixed selection would ignore files the user never pointed
    // at — `/build/` picked from one of three scattered files would sweep up
    // that whole directory.
    let folder = all_equal(paths.iter().map(|p| pattern_for(p, GitignoreScope::Folder)));
    let extension = all_equal(
        paths
            .iter()
            .map(|p| pattern_for(p, GitignoreScope::Extension)),
    );

    Some(GitignoreSuggestions {
        files,
        folder,
        extension,
    })
}

/// Explorer directories generate directory rules; indexed contents stay tracked.
pub fn suggestions_for_entries(
    entries: &[(std::path::PathBuf, bool)],
) -> Option<GitignoreSuggestions> {
    let mut suggestions =
        suggestions_for_paths(&entries.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>())?;
    for (pattern, (_, directory)) in suggestions.files.iter_mut().zip(entries) {
        if *directory {
            pattern.push('/');
        }
    }
    if entries.iter().any(|(_, directory)| *directory) {
        suggestions.extension = None;
    }
    Some(suggestions)
}

/// `Some(value)` when every item is the same `Some(value)`, else `None`.
fn all_equal(mut items: impl Iterator<Item = Option<String>>) -> Option<String> {
    let first = items.next()??;
    items
        .all(|item| item.as_deref() == Some(first.as_str()))
        .then_some(first)
}

impl GitignoreSuggestions {
    /// The `.gitignore` lines this scope would write, in order.
    pub fn lines_for(&self, scope: GitignoreScope) -> Vec<String> {
        match scope {
            GitignoreScope::File => self.files.clone(),
            GitignoreScope::Folder => self.folder.clone().into_iter().collect(),
            GitignoreScope::Extension => self.extension.clone().into_iter().collect(),
        }
    }

    /// The scopes worth offering, in menu order. Always contains `File`.
    pub fn applicable_scopes(&self) -> Vec<GitignoreScope> {
        let mut scopes = vec![GitignoreScope::File];
        if self.folder.is_some() {
            scopes.push(GitignoreScope::Folder);
        }
        if self.extension.is_some() {
            scopes.push(GitignoreScope::Extension);
        }
        scopes
    }
}

/// Append `patterns` to the contents of a `.gitignore`.
///
/// Returns `None` when there is nothing to do — every pattern is already in the
/// file — so the caller can skip the write and tell the user rather than
/// touching the file's mtime and tripping the filesystem watcher for nothing.
///
/// Existing lines are never rewritten. Comparison uses [`trim_trailing_spaces`],
/// not a plain `trim`: leading whitespace *is* significant in a pattern, so
/// `  /build/` is genuinely a different line from `/build/` and must not
/// suppress it — and a plain `trim_end` would eat the escaped trailing space
/// that [`pattern_for`] just went to the trouble of adding, leaving a dangling
/// backslash that swallows the newline.
pub fn append_patterns(existing: &str, patterns: &[String]) -> Option<String> {
    let terminator = crate::text_utils::detect_line_ending_from_texts(
        [existing],
        crate::text_utils::LineEndingDetectionMode::DominantCrlfVsLf,
    );

    // Indexed rather than rescanned per pattern: a 500-file selection against a
    // 2000-line `.gitignore` is a plausible bulk case, and the naive form walks
    // both strings a million times on the store's worker thread.
    let mut present = FxHashSet::default();
    present.extend(existing.lines().map(trim_trailing_spaces));

    // `seen` dedupes within `patterns` itself: two selected files in the
    // same folder both produce `/build/`.
    let mut seen = FxHashSet::with_capacity_and_hasher(patterns.len(), Default::default());
    let mut to_append = Vec::new();
    for pattern in patterns {
        let pattern = trim_trailing_spaces(pattern);
        if pattern.is_empty() {
            continue;
        }
        if !present.contains(pattern) && seen.insert(pattern) {
            to_append.push(pattern);
        }
    }

    // Decided before allocating: re-ignoring already-ignored files is the common
    // bulk case, and it used to copy the whole file into a fresh String only to
    // throw it away here.
    if to_append.is_empty() {
        return None;
    }

    // Git treats a file whose last line has no terminator as needing one before
    // anything is appended after it.
    let leading_terminator = !existing.is_empty() && !existing.ends_with('\n');
    let capacity = to_append.iter().fold(
        existing.len().saturating_add(if leading_terminator {
            terminator.len()
        } else {
            0
        }),
        |capacity, pattern| capacity.saturating_add(pattern.len().saturating_add(terminator.len())),
    );
    let mut out = String::with_capacity(capacity);
    out.push_str(existing);
    if leading_terminator {
        out.push_str(terminator);
    }
    for pattern in to_append {
        out.push_str(pattern);
        out.push_str(terminator);
    }
    Some(out)
}

/// Trim trailing spaces from a `.gitignore` line the way git itself does.
///
/// Git's `trim_trailing_spaces` walks the line and treats a backslash as
/// consuming the character after it, so an escaped space is kept *and* ends the
/// run being considered for trimming. A plain `str::trim_end` does not know
/// that, and using one on a pattern would silently unescape it.
pub fn trim_trailing_spaces(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut trailing_run: Option<usize> = None;
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b' ' => {
                trailing_run.get_or_insert(ix);
                ix += 1;
            }
            // Skip the escaped character with the backslash. A lone trailing
            // backslash walks past the end, which is fine: the run is already
            // cleared, so nothing is trimmed — git bails out there too.
            b'\\' => {
                trailing_run = None;
                ix += 2;
            }
            _ => {
                trailing_run = None;
                ix += 1;
            }
        }
    }
    // Safe to slice: the index always points at an ASCII space, which is a char
    // boundary even when the line contains multi-byte characters.
    match trailing_run {
        Some(ix) => &line[..ix],
        None => line,
    }
}

/// Split a repo-relative path into UTF-8 segments.
///
/// Anything that is not a plain relative path — absolute, a `..` escape, a
/// Windows drive prefix — is rejected rather than sanitized: those never come
/// out of a status listing, so seeing one means the caller is confused and a
/// silently-corrected pattern would hide that.
///
/// A segment containing a line break is rejected for a sharper reason:
/// `.gitignore` is line-oriented and git has no escape for a newline inside a
/// pattern, so a file legitimately named `"a\nb.log"` cannot be expressed at
/// all. Emitting it anyway would split into two unrelated patterns — `/a`, which
/// ignores an entire unrelated root-level entry, and `b.log`, which ignores that
/// name at every depth. Refusing to offer the action is the only safe answer.
fn relative_segments(relative_path: &Path) -> Option<Vec<&str>> {
    let mut segments = Vec::new();
    for component in relative_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let part = part.to_str()?;
                if part.contains(['\n', '\r']) {
                    return None;
                }
                segments.push(part);
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!segments.is_empty()).then_some(segments)
}

/// Join segments into a pattern anchored at the `.gitignore`'s own directory.
///
/// The leading `/` does double duty: it pins the pattern to the repository root
/// instead of matching the same name at any depth, and it occupies the first
/// column, so a path starting with `#` or `!` needs no further escaping — those
/// two are only special at the very start of a pattern.
fn anchored(segments: &[&str]) -> String {
    let mut out = String::with_capacity(segments.iter().map(|segment| segment.len() + 1).sum());
    for segment in segments {
        out.push('/');
        out.push_str(&escape_literal(segment));
    }
    out
}

/// Escape the glob metacharacters so git matches the segment literally.
/// Borrowed when there is nothing to escape, which is nearly every path.
fn escape_literal(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(['\\', '*', '?', '[', ']']) {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 4);
    for ch in text.chars() {
        if matches!(ch, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(ch);
    }
    std::borrow::Cow::Owned(out)
}

/// Escape trailing spaces, which git strips from a pattern unless quoted.
///
/// Every space in the run needs its own backslash: git's `trim_trailing_spaces`
/// walks the line and only an escaped space resets the run, so `foo\  ` still
/// loses its last space.
fn escape_trailing_spaces(pattern: &str) -> String {
    let kept = pattern.trim_end_matches(' ');
    if kept.len() == pattern.len() {
        return pattern.to_string();
    }
    let trailing = pattern.len() - kept.len();
    let mut out = String::with_capacity(pattern.len() + trailing);
    out.push_str(kept);
    for _ in 0..trailing {
        out.push_str("\\ ");
    }
    out
}

/// The file name's extension, or `None` when there isn't a usable one.
fn extension_of(file_name: &str) -> Option<&str> {
    let extension = Path::new(file_name).extension()?.to_str()?;
    (!extension.is_empty()).then_some(extension)
}

#[cfg(test)]
mod tests;
